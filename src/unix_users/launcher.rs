//! Launcher - runs a program as a person's Unix user: `moon as-unix-user <name> <program> …`.
//!
//! The server is root and nothing it starts for a person is. Every such start is this
//! executable run again with this command, which becomes the user and then the program. The
//! switch is moon's own rather than `su`'s or `sudo`'s, and it is the same one for a shell on
//! a pty, a language server, a git command and a display application.
//!
//! The two halves are here: what the server runs ([`UnixUser::command`],
//! [`UnixUser::command_line`]), and what the command does once it is running
//! ([`become_and_run`]).

use std::{
    collections::HashMap,
    convert::Infallible,
    ffi::CString,
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};

use super::UnixUser;

/// The command of this executable that becomes a Unix user. Not in `moon --help`: it is run by
/// a server, never typed.
pub(crate) const AS_UNIX_USER_COMMAND: &str = "as-unix-user";

/// The variables of the server's own environment a person's process keeps: moon's, which say
/// which server a shell belongs to and where it listens. Everything else of root's
/// environment - its `HOME`, its `SSH_AUTH_SOCK` - stays with root.
const KEPT_VARIABLE_PREFIX: &str = "MOON";

/// What the command exits with when it could not become the user or run the program - what a
/// shell exits with for a command it could not run. Not 1, which a program like a search
/// means "found nothing" by.
pub(crate) const COULD_NOT_RUN_STATUS: i32 = 126;

/// The `PATH` a login starts from, before the person's profile adds to it.
const LOGIN_BASE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// How long the server goes on using the login `PATH` it last asked a user's shell for. Short
/// enough that a tool somebody just installed is found without the server being restarted,
/// which would end everyone's agents.
const LOGIN_PATH_KEPT_FOR: Duration = Duration::from_secs(30);

/// How long a user's login shell is given to say its `PATH`.
const LOGIN_PATH_ASKED_FOR: Duration = Duration::from_secs(15);

/// Where each user's runtime directory is, by uid - what `XDG_RUNTIME_DIR` names.
const RUNTIME_DIRS: &str = "/run/user";

/// What a person's processes create is the group's to change: 664 files, 775 folders.
pub(super) const GROUP_WRITABLE_UMASK: libc::mode_t = 0o002;

/// The server's own variables a person's process is started with - see
/// [`KEPT_VARIABLE_PREFIX`].
pub(crate) fn kept_server_environment() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(name, _)| name.starts_with(KEPT_VARIABLE_PREFIX))
        .collect()
}

impl UnixUser {
    /// The command line that runs `program` with `arguments` as this user: this executable,
    /// asked to become the user first.
    ///
    /// Whoever runs it clears the environment and gives it [`kept_server_environment`] and
    /// whatever the program itself is told through one. The user's own `HOME`, `PATH` and the
    /// rest are set by the command once it has become the user - see [`become_and_run`].
    pub(crate) fn command_line(&self, program: &str, arguments: &[String]) -> Result<Vec<String>> {
        let moon = super::moon_executable()?;
        let moon = moon
            .to_str()
            .with_context(|| format!("moon's own path {moon:?} is not UTF-8"))?;
        let mut line = vec![
            moon.to_string(),
            AS_UNIX_USER_COMMAND.to_string(),
            self.name.clone(),
            program.to_string(),
        ];
        line.extend(arguments.iter().cloned());
        Ok(line)
    }

    /// `program` as this user, for a start that is not on a pty: the environment cleared down
    /// to what a person's process keeps, and arguments still to be added.
    ///
    /// It is handed the user's login `PATH` as the server last heard it - see
    /// [`LOGIN_PATH_KEPT_FOR`] - rather than left to ask for it: git is run this way several
    /// times a second, and a login shell each time is a login shell too many.
    pub(crate) fn command(&self, program: &str) -> Result<Command> {
        let mut command = self.command_asking_for_its_path(program)?;
        command.env("PATH", self.login_path()?);
        Ok(command)
    }

    fn command_asking_for_its_path(&self, program: &str) -> Result<Command> {
        let line = self.command_line(program, &[])?;
        let mut command = Command::new(&line[0]);
        command
            .args(&line[1..])
            .env_clear()
            .envs(kept_server_environment());
        // Which characters the program reads and writes - see `crate::shell_locale`. A shell
        // on a pty is told the same by whoever starts it.
        if let Some(lang) = crate::shell_locale::shell_lang() {
            command.env("LANG", lang);
        }
        Ok(command)
    }

    /// The user's login `PATH`, asked of their shell when the server has not heard it lately:
    /// where what they installed is looked for.
    pub(crate) fn login_path(&self) -> Result<String> {
        static HEARD: Mutex<Option<HashMap<String, (Instant, String)>>> = Mutex::new(None);
        let heard = || HEARD.lock().expect("nothing panics holding the login paths");
        if let Some((at, path)) = heard().get_or_insert_with(HashMap::new).get(&self.name)
            && at.elapsed() < LOGIN_PATH_KEPT_FOR
        {
            return Ok(path.clone());
        }
        // Asked with the lock let go: it is a person's own shell profile that is run, and
        // everyone else's git waits on that lock. Two askers at once both ask, and agree.
        let path = self.ask_for_login_path()?;
        heard()
            .get_or_insert_with(HashMap::new)
            .insert(self.name.clone(), (Instant::now(), path.clone()));
        Ok(path)
    }

    /// Run the user's login shell for its `PATH`, through the command that becomes them: it
    /// works the `PATH` out as it does, so any program run through it can say what it was
    /// given. Given [`LOGIN_PATH_ASKED_FOR`] at most: a profile that never ends is that
    /// person's to mend, and the thread asking may be holding what every request needs.
    fn ask_for_login_path(&self) -> Result<String> {
        let mut asked = self
            .command_asking_for_its_path("/bin/sh")?
            .args(["-c", "printf %s \"$PATH\""])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to ask for the login PATH of {}", self.name))?;
        let deadline = Instant::now() + LOGIN_PATH_ASKED_FOR;
        let status = loop {
            if let Some(status) = asked.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = asked.kill();
                let _ = asked.wait();
                bail!(
                    "the login shell of {} did not say its PATH within {} seconds: something \
                     in their shell profile does not end",
                    self.name,
                    LOGIN_PATH_ASKED_FOR.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        ensure!(
            status.success(),
            "asking for the login PATH of {} failed with {status}",
            self.name
        );
        let mut path = String::new();
        asked
            .stdout
            .take()
            .expect("the command was started with its output piped")
            .read_to_string(&mut path)
            .context("a PATH is UTF-8")?;
        Ok(path)
    }
}

/// `moon as-unix-user <name> <program> [arguments…]`: become that user, and then the program.
///
/// The process gets the user's groups, `HOME`, login `PATH` and `XDG_RUNTIME_DIR`. A `PATH` it
/// was handed is kept: the server hands one over when it has asked already. Any step that
/// fails ends it there: the program is never run as root.
pub(crate) fn become_and_run(words: &[String]) -> Result<Infallible> {
    let [name, program, arguments @ ..] = words else {
        bail!("`{AS_UNIX_USER_COMMAND}` takes a Unix user and a program to run as it");
    };
    // SAFETY: reads this process's own credentials.
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "only root becomes another Unix user"
    );
    let user = UnixUser::named(name)?.with_context(|| format!("there is no Unix user {name}"))?;
    ensure!(
        user.uid != 0,
        "{name} is root, which nobody's program runs as"
    );

    let runtime_dir = runtime_dir_of(&user)?;
    hand_over_the_terminal(&user)?;
    switch_to(&user)?;
    // SAFETY: sets this process's own mask, and cannot fail.
    unsafe { libc::umask(GROUP_WRITABLE_UMASK) };

    // SAFETY: this process has one thread, and is about to be replaced by the program.
    unsafe {
        std::env::set_var("HOME", &user.home);
        std::env::set_var("USER", &user.name);
        std::env::set_var("LOGNAME", &user.name);
        std::env::set_var("SHELL", &user.shell);
        std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
        if std::env::var_os("PATH").is_none() {
            std::env::set_var("PATH", LOGIN_BASE_PATH);
            std::env::set_var("PATH", login_path(&user));
        }
    }

    let failed = Command::new(program).args(arguments).exec();
    Err(failed).with_context(|| format!("failed to run {program} as {name}"))
}

/// The user's runtime directory, made when the machine has not made one: a server starts a
/// person's processes without a login, which is what makes it on a machine with logind.
fn runtime_dir_of(user: &UnixUser) -> Result<PathBuf> {
    let dir = PathBuf::from(RUNTIME_DIRS).join(user.uid.to_string());
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(RUNTIME_DIRS)
        .with_context(|| format!("failed to make {RUNTIME_DIRS}"))?;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("failed to make {}", dir.display()))?;
    std::os::unix::fs::chown(&dir, Some(user.uid), Some(user.gid))
        .with_context(|| format!("failed to give {} to {}", dir.display(), user.name))?;
    Ok(dir)
}

/// Give the terminal this process is on to the user, as a login does. The pty was opened by
/// root, and a program that opens its terminal by name - `gpg` asking for a passphrase on
/// `GPG_TTY` - is refused a terminal that is still root's.
fn hand_over_the_terminal(user: &UnixUser) -> Result<()> {
    // SAFETY: asks about, and changes the owner of, this process's own standard input.
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) != 1 {
            return Ok(());
        }
        // The group is left as it is: `tty`, which is what lets `write` and `wall` reach it.
        if libc::fchown(libc::STDIN_FILENO, user.uid, libc::gid_t::MAX) != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to give the terminal to {}", user.name));
        }
    }
    Ok(())
}

/// Become the user for good: its groups, its group and its uid, in the order that leaves
/// each still allowed. Checked afterwards, because a process that kept a way back to root
/// would run the person's program with it.
fn switch_to(user: &UnixUser) -> Result<()> {
    let name = CString::new(user.name.as_str()).context("a user name holds no NUL")?;
    let failed = |step: &str| {
        Err::<(), _>(std::io::Error::last_os_error())
            .with_context(|| format!("failed to {step} of {}", user.name))
    };
    // SAFETY: each call changes this process's own credentials, with a name that outlives it.
    unsafe {
        if libc::initgroups(name.as_ptr(), user.gid as _) != 0 {
            return failed("take the groups");
        }
        if libc::setgid(user.gid) != 0 {
            return failed("take the group");
        }
        if libc::setuid(user.uid) != 0 {
            return failed("take the uid");
        }
        ensure!(
            libc::getuid() == user.uid
                && libc::geteuid() == user.uid
                && libc::getgid() == user.gid
                && libc::getegid() == user.gid,
            "this process did not become {}",
            user.name
        );
        ensure!(
            libc::setuid(0) != 0,
            "this process can still become root after becoming {}",
            user.name
        );
    }
    Ok(())
}

/// The `PATH` a login shell of the user has, which is where what they installed is found -
/// see `crate::shell_path` for why a login shell is asked. Run with the process already the
/// user and its `HOME` set. A profile that fails or says nothing leaves the base `PATH`: the
/// person needs a shell to mend it in.
fn login_path(user: &UnixUser) -> String {
    let said = Command::new(&user.shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    said.unwrap_or_else(|| LOGIN_BASE_PATH.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn someone() -> UnixUser {
        UnixUser {
            name: "moon-someone".to_string(),
            uid: 2001,
            gid: 2000,
            home: PathBuf::from("/home/moon-someone"),
            shell: "/bin/bash".to_string(),
        }
    }

    #[test]
    fn a_program_is_run_through_this_executable_asked_to_become_the_user() {
        let line = someone()
            .command_line("claude", &["--resume".to_string()])
            .expect("expected a command line");

        assert_eq!(
            line[1..],
            ["as-unix-user", "moon-someone", "claude", "--resume"]
        );
        assert_eq!(
            PathBuf::from(&line[0]),
            std::env::current_exe().expect("a test has an executable")
        );
    }

    #[test]
    fn a_command_keeps_moons_variables_and_the_locale_and_none_of_the_servers_others() {
        let command = someone()
            .command_asking_for_its_path("git")
            .expect("expected a command");

        let kept: Vec<String> = command
            .get_envs()
            .filter_map(|(name, value)| value.map(|_| name.to_string_lossy().into_owned()))
            .collect();
        assert!(
            kept.iter()
                .all(|name| name.starts_with("MOON") || name == "LANG"),
            "got {kept:?}"
        );
    }

    #[test]
    fn a_process_that_is_not_root_becomes_nobody() {
        // SAFETY: reads this process's own credentials.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let refused = become_and_run(&["nobody".to_string(), "true".to_string()])
            .expect_err("only root becomes another user");

        assert!(refused.to_string().contains("only root"), "got {refused}");
    }
}
