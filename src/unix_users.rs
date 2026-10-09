//! Unix users - one per person on a shared `moon serve`, so each person's shells and agents
//! have a home of their own and the agent logins in it.
//!
//! Applies only when GitHub sign-in is set up and `moon serve` runs as root - see
//! [`decide_for_this_server`]. Everywhere else nobody has one: every process moon starts runs
//! as the user moon itself runs as, and what is asked here answers so - [`command`] is
//! `Command::new`, and no thread has a current user.
//!
//! Not a security boundary. The people of one server trust each other; what is kept apart is
//! whose login an agent uses, not what a person can reach.

mod current_user;
mod launcher;

pub(crate) use current_user::{
    as_user, current_user, as_the_server, be_the_server_on_this_thread, carried, command,
    person_this_is_done_for, user_to_work_as,
};
pub(crate) use launcher::{
    AS_UNIX_USER_COMMAND, COULD_NOT_RUN_STATUS, become_and_run, kept_server_environment,
};

use std::{
    ffi::CString, os::unix::fs::PermissionsExt, path::PathBuf, process::Command, sync::OnceLock,
};

use anyhow::{Context, Result, bail, ensure};
use sha1::{Digest, Sha1};

/// What a server that gives each person a Unix user knows from its start.
#[derive(Clone, Debug)]
struct SharedServer {
    /// The group that owns the project folder, which every person's Unix user is made in: what
    /// one of them writes in the project, the others can change.
    project_group: u32,
    /// Where moon's own executable is, which every person's program is started through - see
    /// [`launcher`]. Read once: a newer moon installed over a running server leaves the
    /// server unable to say where its own executable is, and the path is still good - what
    /// is there now is moon.
    moon: PathBuf,
}

/// `None` for a server that runs everything as its own user, and for a process that is no
/// server at all.
static SHARED_SERVER: OnceLock<Option<SharedServer>> = OnceLock::new();

/// Decide, as `moon serve` starts, whether each person gets a Unix user: when people sign in
/// with GitHub and the server is root, which is what lets it make users and become them.
///
/// `project_folder` is the folder the server was started on. Its group is the one every
/// person's user is made in, so a folder owned by root's group is refused rather than putting
/// everyone who signs in into that group.
pub(crate) fn decide_for_this_server(
    people_sign_in_with_github: bool,
    project_folder: &std::path::Path,
) -> Result<()> {
    // SAFETY: reads this process's own credentials.
    let serves_as_root = unsafe { libc::geteuid() } == 0;
    let shared = match people_sign_in_with_github && serves_as_root {
        false => None,
        true => {
            ensure!(
                cfg!(target_os = "linux"),
                "a server that is root and signs people in with GitHub gives each of them a \
                 Unix user, which moon does on Linux only: serve as another user here"
            );
            use std::os::unix::fs::MetadataExt;
            let project_group = std::fs::metadata(project_folder)
                .with_context(|| format!("failed to read {}", project_folder.display()))?
                .gid();
            ensure!(
                project_group != 0,
                "{} belongs to root's group, and each person who signs in is given a Unix user \
                 in the group of this folder: give it a group of its own, with `chgrp -R` and \
                 `chmod -R g+w`",
                project_folder.display()
            );
            let moon = everyone_can_run_moon()?;
            // What the server itself writes for a person - a saved file, a card - is the
            // group's to change, as what their own processes write is.
            write_for_the_group();
            Some(SharedServer {
                project_group,
                moon,
            })
        }
    };
    SHARED_SERVER
        .set(shared)
        .ok()
        .context("a process decides once whether its people have Unix users")
}

/// Every person's program is started by this executable run again as them - see [`launcher`]
/// - so it has to be somewhere all of them can run it from, which a build left in root's home
/// is not. Found out as the server starts, rather than by everyone's first shell.
fn everyone_can_run_moon() -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    const OTHERS_READ_AND_RUN: u32 = 0o005;
    const OTHERS_ENTER: u32 = 0o001;
    let moon = std::env::current_exe().context("failed to find moon's own executable")?;
    let allows = |path: &std::path::Path, needed: u32| -> Result<bool> {
        let mode = std::fs::metadata(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .mode();
        Ok(mode & needed == needed)
    };
    let closed = match allows(&moon, OTHERS_READ_AND_RUN)? {
        false => Some(moon.as_path()),
        true => {
            let mut closed = None;
            for folder in moon.ancestors().skip(1) {
                if !allows(folder, OTHERS_ENTER)? {
                    closed = Some(folder);
                    break;
                }
            }
            closed
        }
    };
    if let Some(closed) = closed {
        bail!(
            "moon is at {}, and {} is closed to other users: each person's shells are \
             started by moon run as them, so install it where everyone can run it, such as \
             /usr/local/bin",
            moon.display(),
            closed.display()
        );
    }
    Ok(moon)
}

/// Make what this process writes the group's to change. A `moon` typed in a person's shell
/// on a shared server does this first: the card or the line of the wire it writes is for
/// everyone on the board to change, whatever mask that person's shell profile set.
pub(crate) fn write_for_the_group() {
    // SAFETY: sets this process's own mask, and cannot fail.
    unsafe { libc::umask(launcher::GROUP_WRITABLE_UMASK) };
}

/// Whether each person of this server has a Unix user of their own. When they do, nothing a
/// person starts may run as the server's user, which is root.
pub(crate) fn each_person_has_one() -> bool {
    shared_server().is_some()
}

fn shared_server() -> Option<SharedServer> {
    SHARED_SERVER.get().cloned().flatten()
}

/// Moon's own executable, to start a person's program through: where the server found it as
/// it started, and this process's own for anything that is no such server.
fn moon_executable() -> Result<PathBuf> {
    match shared_server() {
        Some(shared) => Ok(shared.moon),
        None => std::env::current_exe().context("failed to find moon's own executable"),
    }
}

/// One account of this machine's user database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnixUser {
    pub(crate) name: String,
    pub(crate) uid: u32,
    /// The user's own group, which is the project's - see [`SharedServer::project_group`].
    pub(crate) gid: u32,
    pub(crate) home: PathBuf,
    /// The login shell the account is set up with.
    pub(crate) shell: String,
}

/// One person of a shared server: who they are on GitHub, and the Unix user everything
/// started for them runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Person {
    pub(crate) github_login: String,
    pub(crate) unix_user: UnixUser,
}

/// An account as the user database has it, with the comment moon marks its own by.
struct Account {
    user: UnixUser,
    comment: String,
}

impl UnixUser {
    /// The account of this name, and `None` when the machine has none.
    pub(crate) fn named(name: &str) -> Result<Option<Self>> {
        Ok(account_named(name)?.map(|account| account.user))
    }
}

/// How much room `getpwnam_r` is given for an account's strings. Far more than a line of
/// `/etc/passwd` holds; an account that needs more is refused rather than read short.
const ACCOUNT_BUFFER_BYTES: usize = 16 * 1024;

fn account_named(name: &str) -> Result<Option<Account>> {
    let c_name = CString::new(name).with_context(|| format!("{name:?} is no user name"))?;
    let mut buffer = vec![0u8; ACCOUNT_BUFFER_BYTES];
    // SAFETY: `passwd` is plain data that `getpwnam_r` fills in.
    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to memory that outlives the call, and the buffer's length is
    // the one passed.
    let error = unsafe {
        libc::getpwnam_r(
            c_name.as_ptr(),
            &mut passwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    // No such account is said either way, by which user database is asked: nothing found and
    // no error, or one of these two errors.
    if found.is_null() && [0, libc::ENOENT, libc::ESRCH].contains(&error) {
        return Ok(None);
    }
    if error != 0 {
        bail!(
            "failed to look up the Unix user {name}: {}",
            std::io::Error::from_raw_os_error(error)
        );
    }
    // SAFETY: on success the strings of `passwd` point into `buffer`, which is still alive.
    let text = |field: *const libc::c_char| unsafe {
        std::ffi::CStr::from_ptr(field)
            .to_string_lossy()
            .into_owned()
    };
    Ok(Some(Account {
        user: UnixUser {
            name: text(passwd.pw_name),
            uid: passwd.pw_uid,
            gid: passwd.pw_gid,
            home: PathBuf::from(text(passwd.pw_dir)),
            shell: text(passwd.pw_shell),
        },
        comment: text(passwd.pw_gecos),
    }))
}

/// What every name starts with. No account a system or its packages make starts this way, so
/// a name from here is never `root`, `git` or a daemon's, whatever the login is.
const NAME_PREFIX: &str = "moon-";

/// The longest name `useradd` takes.
const LONGEST_NAME: usize = 32;

/// How many hex digits of the login's digest end the name of a login too long to fit whole.
const DIGEST_DIGITS: usize = 8;

/// The Unix user name of a GitHub login: the prefix and the login in lower case.
///
/// GitHub tells logins apart without regard to case, so lower case loses nothing. A login too
/// long to fit keeps its start and ends in a digest of the whole of it, so two long logins that
/// start alike are still two names.
pub(crate) fn name_for_github_login(login: &str) -> Result<String> {
    ensure!(
        !login.is_empty()
            && login
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "{login:?} is no GitHub login: one is letters, digits, hyphens and underscores"
    );
    let login = login.to_ascii_lowercase();
    let whole = format!("{NAME_PREFIX}{login}");
    if whole.len() <= LONGEST_NAME {
        return Ok(whole);
    }
    let digest: String = Sha1::digest(login.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let kept = LONGEST_NAME - NAME_PREFIX.len() - 1 - DIGEST_DIGITS;
    Ok(format!(
        "{NAME_PREFIX}{}-{}",
        &login[..kept],
        &digest[..DIGEST_DIGITS]
    ))
}

/// What makes the account. Named in full: root's `PATH` under `su` leaves `/usr/sbin` out.
const USERADD: &str = "/usr/sbin/useradd";

/// The shell a new user is given, when the machine has it.
const PREFERRED_SHELL: &str = "/bin/bash";
const FALLBACK_SHELL: &str = "/bin/sh";

/// Only the person, and root, read a home: the agent logins are in it.
const HOME_MODE: u32 = 0o700;

/// What moon writes in the comment of an account it makes, which is how an account it made is
/// told from one that only has the name. The account id is `github:<number>`, and a comment
/// cannot hold a colon.
fn comment_for(account_id: &str) -> String {
    format!("moon {}", account_id.replace(':', " "))
}

/// The Unix user of a GitHub account: the one `recorded` for it at an earlier sign-in, and
/// otherwise the one named after its login - see [`name_for_github_login`]. Made when the
/// machine has no account of that name.
///
/// An account of that name that moon did not make for this GitHub account is refused, and so
/// is whatever goes wrong making one: the sign-in fails rather than going on as another user.
pub(crate) fn of_account(
    recorded: Option<&str>,
    github_login: &str,
    account_id: &str,
) -> Result<UnixUser> {
    let shared = shared_server().context("this server gives nobody a Unix user")?;
    let name = match recorded {
        Some(name) => name.to_string(),
        None => name_for_github_login(github_login)?,
    };
    found_or_made(&name, &comment_for(account_id), shared.project_group)
}

/// The account of this name, when it is one moon made with this comment; a new one in this
/// group, when the machine has none of the name.
fn found_or_made(name: &str, comment: &str, project_group: u32) -> Result<UnixUser> {
    if let Some(account) = account_named(name)? {
        ensure!(
            account.comment == comment,
            "the Unix user {name} already exists and was not made by moon for this GitHub \
             account"
        );
        return Ok(account.user);
    }

    let shell = match std::path::Path::new(PREFERRED_SHELL).exists() {
        true => PREFERRED_SHELL,
        false => FALLBACK_SHELL,
    };
    let made = Command::new(USERADD)
        .args(["--create-home", "--shell", shell, "--comment", comment])
        .args(["--gid", &project_group.to_string()])
        .arg(name)
        .output()
        .with_context(|| format!("failed to run {USERADD} to make the Unix user {name}"))?;
    if !made.status.success() {
        bail!(
            "{USERADD} did not make the Unix user {name}: {}",
            String::from_utf8_lossy(&made.stderr).trim()
        );
    }
    let user =
        UnixUser::named(name)?.with_context(|| format!("{USERADD} made no Unix user {name}"))?;
    std::fs::set_permissions(&user.home, std::fs::Permissions::from_mode(HOME_MODE))
        .with_context(|| format!("failed to make {} private", user.home.display()))?;
    Ok(user)
}

/// What a person's own git is told, in their `~/.gitconfig`, each time they sign in.
///
/// Who they are, so a commit their agent makes is theirs rather than the Unix user's. And that
/// every folder is safe: the project is one checkout worked in by all of them, which git
/// otherwise refuses in everyone's shell but its owner's.
pub(crate) fn tell_git_who_they_are(user: &UnixUser, name: &str, email: &str) -> Result<()> {
    let settings = [
        ("user.name", name),
        ("user.email", email),
        ("safe.directory", "*"),
    ];
    for (key, value) in settings {
        let told = user
            .command("git")?
            .args(["config", "--global", "--replace-all", key, value])
            .current_dir(&user.home)
            .output()
            .with_context(|| format!("failed to run git as {}", user.name))?;
        ensure!(
            told.status.success(),
            "git did not take {key} for {}: {}",
            user.name,
            String::from_utf8_lossy(&told.stderr).trim()
        );
    }
    Ok(())
}

/// How long a person's processes get to leave after being hung up on, before what is left is
/// killed: an agent writes its session down on the way out.
const HANGUP_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// How many times what is left of a person's processes is listed and killed before giving up
/// on something that starts them faster than that.
const KILL_ROUNDS: usize = 20;

/// End every process of this user, which is what kicking somebody out comes to: hung up on,
/// given a moment, and killed if still there. Only ever a person's user - one moon made.
pub(crate) fn end_processes_of(user: &UnixUser) -> Result<()> {
    ensure!(user.uid != 0, "root's processes are not a person's to end");
    let signal_all = |signal: libc::c_int| -> Result<usize> {
        let processes = processes_of(user.uid)?;
        for pid in &processes {
            // SAFETY: sends a signal; a process gone since it was listed is an error ignored.
            unsafe { libc::kill(*pid, signal) };
        }
        Ok(processes.len())
    };
    signal_all(libc::SIGHUP)?;
    let deadline = std::time::Instant::now() + HANGUP_GRACE;
    while std::time::Instant::now() < deadline {
        if processes_of(user.uid)?.is_empty() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Killed until a listing finds none: a process started between a listing and its kill
    // is in the next one.
    for _ in 0..KILL_ROUNDS {
        if signal_all(libc::SIGKILL)? == 0 {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!(
        "processes of {} are still being started after {KILL_ROUNDS} rounds of killing them",
        user.name
    )
}

/// The processes running as this uid, read from `/proc`: each one's `status` says whose it is.
fn processes_of(uid: u32) -> Result<Vec<libc::pid_t>> {
    ensure!(
        cfg!(target_os = "linux"),
        "the processes of a Unix user are listed on Linux only"
    );
    let mut processes = Vec::new();
    for entry in std::fs::read_dir("/proc").context("failed to list /proc")? {
        let Ok(pid) = entry?.file_name().to_string_lossy().parse::<libc::pid_t>() else {
            continue;
        };
        // A process that ended between the listing and the read has nothing to say.
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            continue;
        };
        let real_uid = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|uids| uids.split_whitespace().next())
            .and_then(|real| real.parse::<u32>().ok());
        if real_uid == Some(uid) {
            processes.push(pid);
        }
    }
    Ok(processes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(login: &str) -> String {
        name_for_github_login(login).expect("expected a name")
    }

    /// What `useradd` takes as a name: a lower case letter or underscore, then lower case
    /// letters, digits, underscores and hyphens, 32 characters at most.
    fn is_a_unix_user_name(name: &str) -> bool {
        let mut chars = name.chars();
        let first = chars.next().expect("a name is not empty");
        (first.is_ascii_lowercase() || first == '_')
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            && name.len() <= LONGEST_NAME
    }

    #[test]
    fn a_login_is_named_in_lower_case_behind_the_prefix() {
        assert_eq!(named("Antoine-MP"), "moon-antoine-mp");
        assert_eq!(named("antoine-mp"), named("Antoine-MP"));
    }

    #[test]
    fn a_login_with_a_leading_digit_is_still_a_unix_user_name() {
        assert_eq!(named("0xdeadbeef"), "moon-0xdeadbeef");
        assert!(is_a_unix_user_name(&named("0xdeadbeef")));
    }

    #[test]
    fn an_over_long_login_fits_and_stays_apart_from_one_that_starts_alike() {
        // GitHub's longest login is 39 characters.
        let one = "a".repeat(38) + "b";
        let other = "a".repeat(38) + "c";

        assert!(is_a_unix_user_name(&named(&one)), "got {}", named(&one));
        assert_eq!(named(&one).len(), LONGEST_NAME);
        assert_ne!(named(&one), named(&other));
    }

    #[test]
    fn a_login_named_like_a_system_account_is_not_that_account() {
        for system_account in ["root", "git", "daemon", "nobody", "www-data"] {
            let name = named(system_account);
            assert_ne!(name, system_account);
            assert!(name.starts_with(NAME_PREFIX), "got {name}");
        }
    }

    #[test]
    fn what_is_no_github_login_is_refused() {
        for login in ["", "a b", "a:b", "../etc", "é"] {
            assert!(name_for_github_login(login).is_err(), "took {login:?}");
        }
    }

    /// Makes a real account, so it only runs where that was asked for: as root on Linux, with
    /// `MOON_TEST_MAKES_UNIX_USERS` naming the account to make. Nothing deletes it afterwards,
    /// as nothing in moon does.
    #[test]
    fn a_unix_user_is_made_once_in_the_project_group_and_is_not_anothers() {
        let Ok(name) = std::env::var("MOON_TEST_MAKES_UNIX_USERS") else {
            return;
        };
        // SAFETY: reads this process's own credentials.
        let group = unsafe { libc::getegid() } + 100;

        let made = found_or_made(&name, "moon github 1", group).expect("expected a user");
        let found = found_or_made(&name, "moon github 1", group).expect("expected it again");
        let anothers = found_or_made(&name, "moon github 2", group);

        assert_eq!(made, found);
        assert_eq!((made.name.as_str(), made.gid), (name.as_str(), group));
        let home = std::fs::metadata(&made.home).expect("expected a home");
        assert_eq!(home.permissions().mode() & 0o777, HOME_MODE);
        assert!(anothers.is_err(), "another account's user was handed over");
    }

    #[test]
    fn the_comment_of_an_account_names_the_github_account_without_a_colon() {
        assert_eq!(comment_for("github:583231"), "moon github 583231");
    }
}
