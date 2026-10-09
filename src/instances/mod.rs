//! The windows running on this machine, and how a shell reaches one.
//!
//! `moon open <file>` is meant to land in a window that is already open rather than start
//! another one, so every window writes down where it is and listens on a socket of its own:
//! `~/.moonreview/instances/<pid>.json` says which project the window with that pid is on
//! and when it was last in front, and `<pid>.sock` beside it is where it is asked to open a
//! file. Both are written when the window opens a project and taken away when it closes; a
//! window that was killed leaves them behind, and the next read clears those out.
//!
//! `moon shell <folder>` reaches a window the same way, and asks it for a shell in that
//! folder rather than a tab on a file. `moon open <folder>` does too, and asks it for its
//! file picker on that folder.
//!
//! `moon wire post @handle …` reaches one moon and no other: the one holding the shell of
//! the agent the line is for, which it asks to type the line in - see
//! [`crate::moontasks::wire`].
//!
//! `moon agent start`, `tell` and `view` each reach one moon too - the one an agent of the
//! board is started in, and the one holding the shell of the agent told or looked at - and
//! are answered with what came of it rather than with a promise: see [`window::AgentAsks`].
//!
//! `moon launch <command>` reaches the moon whose shell it was typed in.
//!
//! The moon those reach may be a `moon serve`: it is no window and is written down as none,
//! and listens on a socket named the same way for what is asked about its agents and for a
//! program to start - see [`server`]. One that gives each person a Unix user listens where
//! all of them reach it instead, and tells its shells where: see [`SOCKET_ENV`].

pub(crate) mod server;
pub(crate) mod window;

#[cfg(test)]
mod tests;

use std::{
    ffi::OsStr,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{api::AgentKind, terminal::Shown, unix_users::Person};

const SETTINGS_DIR_NAME: &str = ".moonreview";
const INSTANCES_DIR_NAME: &str = "instances";

/// The pid of the window a shell was started by, which is how a `moon open` typed in one of
/// the window's own shells reaches that window rather than whichever other one holds the
/// file. Written into every shell the window starts - see [`crate::terminal`].
pub(crate) const WINDOW_ENV: &str = "MOON_INSTANCE";

/// The socket of the moon a shell was started by, when it is not beside the records in the
/// shell's own home: a server that gives each person a Unix user runs its shells as users who
/// cannot enter its home, so it listens in [`SHARED_SOCKETS_DIR`] and writes where into every
/// shell it starts - see [`socket_for_other_unix_users`]. Unset everywhere else.
pub(crate) const SOCKET_ENV: &str = "MOON_INSTANCE_SOCKET";

/// The GitHub login of the person a shell was started for, on a server that gives each person
/// a Unix user: what a command typed in the shell signs with, where it writes the board's
/// files itself and no server is there to say who is asking - see [`shell_person`]. Unset
/// everywhere else.
pub(crate) const PERSON_ENV: &str = "MOON_PERSON";

/// Where a server that gives each person a Unix user listens. Root's to write in, like the
/// rest of `/run`, and nobody's home.
const SHARED_SOCKETS_DIR: &str = "/run/moon";

/// Anybody may look in that folder and connect to the socket in it. Who is answered is
/// decided once they have connected, by whose Unix user they run as - see [`server`].
const SHARED_SOCKETS_DIR_MODE: u32 = 0o755;
const SHARED_SOCKET_MODE: u32 = 0o666;

/// How long a window is given to answer before the ask is taken as unanswerable and the next
/// window is tried. It is a local socket and the answer is written the moment the ask is
/// read, so this is only ever hit by a window that is not really there any more.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a moon is given to say how the start of a program went, which it only says once
/// it knows: a desktop may have to be started for the program first, and the program is then
/// given a second and a half to open a window or fail - see `crate::display::started`.
const LAUNCH_ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// What a moon does with a program a shell asked it to start: `moon launch`.
///
/// Answered on the thread the socket is read on, with how the start went, as what is asked
/// about an agent is - see [`window::AgentAsks`]: whoever asked is a command waiting to say
/// so.
pub(crate) trait StartsApplications: Send + Sync {
    /// Start a line of shell for its windows, run in `folder`, and answer once the start is
    /// known to have gone one way or the other: with where its windows open, in the words the
    /// shell that asked prints, or with what the program said when it ended in failure.
    fn start(&self, command: &str, folder: &Path) -> Result<String>;

    /// The same, asked from a shell of one person of a server that gives each of them a Unix
    /// user: the program is theirs, and runs as their user. Refused by a moon with no way to
    /// do that - nothing a person starts runs as the server's own user.
    fn start_for_person(
        &self,
        person: &Person,
        _command: &str,
        _folder: &Path,
    ) -> Result<String> {
        bail!(
            "this moon starts no program as the Unix user of {}",
            person.github_login
        )
    }
}

/// One running window: what it is open on, and where to reach it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Instance {
    /// The window's process, which is also the name of both of its files.
    pub(crate) pid: u32,
    /// Which of the executables it is, for what the CLI prints.
    pub(crate) program: String,
    /// The project the window is open on, absolute and with symlinks followed - the paths a
    /// file is measured against have been through the same resolution.
    pub(crate) project_path: String,
    /// When this window was last brought to the front, in seconds since the epoch, and 0 for
    /// a window that has not been in front since it opened. It is what decides where a file
    /// goes when no window is open on its project: the window being looked at.
    #[serde(default)]
    pub(crate) focused_at_unix: u64,
}

/// What a shell asks a window for.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub(crate) enum Ask {
    /// Open this file in a tab, at this line when one was named. `wait` is `moon edit --wait`:
    /// the shell will keep asking [`Ask::StillOpen`] about the file until its tab is closed.
    OpenFile {
        path: String,
        line: Option<usize>,
        #[serde(default)]
        wait: bool,
    },
    /// Whether the tab a `moon edit --wait` opened on this file is still open.
    StillOpen { path: String },
    /// Open a shell in this folder, in a tab: `moon shell <folder>`.
    OpenShell { folder: String },
    /// Bring the file picker up on this folder, for a file of it to be picked and opened:
    /// `moon open <folder>`. An ask of its own rather than an [`Ask::OpenFile`] on a folder:
    /// the shell that asks has read the path against the disk and knows which it is, so the
    /// window is told rather than left to find out.
    PickFile { folder: String },
    /// Type a direct message of the wire into one of this window's shells and send it:
    /// `moon wire post @handle …`. It is carried in its parts rather than as the line to
    /// type, because the window writes it down as well, and the two read differently - see
    /// `typed_line` and `logged_line` in [`crate::moontasks::wire`].
    Wire {
        /// The shell of the agent it is for, by the id this window's moon knows it under.
        terminal_id: String,
        /// The handles of the task it was posted from and of the task it is for.
        sender: String,
        recipient: String,
        message: String,
    },
    /// Start an agent on a task of the board in this repo, in a shell this window holds:
    /// `moon agent start`.
    StartAgent {
        /// The repo the board is in, which has to be the project this window is on.
        repo_path: String,
        task_id: String,
        agent: AgentKind,
    },
    /// Type a line into one of this window's shells, exactly as it is written here, and send
    /// it: `moon agent tell`.
    Tell { terminal_id: String, line: String },
    /// Say what one of this window's shells is showing: `moon agent view`.
    Shown { terminal_id: String, wanted: Shown },
    /// Start a program for its windows: `moon launch`. Asked of the moon whose shell it was
    /// typed in, which a `moon serve` can be - see [`launch`].
    Launch {
        /// A line of shell.
        command: String,
        /// The folder the shell that asked is in, which is where the program is run.
        folder: String,
    },
}

/// What the window answers.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub(crate) enum Answer {
    /// The window has the file or the folder and is opening it - a tab, a shell, or its
    /// file picker.
    Opened,
    /// The window will not open it, and says why - it has no project open yet, or the repo
    /// it is open on is on another machine, so the path the shell named is not one it reads.
    Refused { reason: String },
    /// The file a `moon edit --wait` is waiting on is still open, or about to be.
    StillOpen,
    /// Its tab has been closed, and the shell can stop waiting.
    Closed,
    /// The window has the line of the wire, and types it into the shell as soon as nothing
    /// there is in the way - see `told` in [`crate::terminal`].
    Wired,
    /// The agent is started, in a run of this name.
    Started { run: String },
    /// The window's shell has the line, and it is typed as soon as nothing there is in the way
    /// - the same wait a line of the wire makes.
    Told,
    /// What the shell is showing, one line per row.
    Shown { text: String },
    /// The program is started: a window of it has opened, or it has not ended in failure in
    /// the time its start is given. `on` is where its windows open, for the shell to print.
    Launched { on: String },
}

impl Instance {
    /// Ask this window to do something, and wait for its answer.
    pub(crate) fn ask(&self, ask: &Ask) -> Result<Answer> {
        ask_process(self.pid, ask, ANSWER_TIMEOUT)
    }
}

/// Ask the moon running as this process to do something, and wait `within` for its answer.
fn ask_process(pid: u32, ask: &Ask, within: Duration) -> Result<Answer> {
    ask_over(&reach(pid)?, ask, within)
}

/// Connect to the socket the moon running as this process is asked on. Apart from the asking
/// so that a moon that is not there can be told from one that is there and says nothing - see
/// [`hand_to_a_window`].
fn reach(pid: u32) -> Result<UnixStream> {
    let socket_path = socket_path(pid).context("no home directory to reach a window in")?;
    UnixStream::connect(&socket_path)
        .with_context(|| format!("failed to reach {}", socket_path.display()))
}

/// Ask a moon something over a connection to it, and wait `within` for its answer.
fn ask_over(stream: &UnixStream, ask: &Ask, within: Duration) -> Result<Answer> {
    stream.set_read_timeout(Some(within))?;
    stream.set_write_timeout(Some(within))?;

    let mut writing = stream;
    writeln!(writing, "{}", serde_json::to_string(ask)?)?;
    writing.flush()?;

    let mut answer = String::new();
    BufReader::new(stream)
        .read_line(&mut answer)
        .context("the window said nothing")?;
    serde_json::from_str(answer.trim())
        .with_context(|| format!("the window answered with {answer:?}"))
}

/// Where the records live. `None` when this account has no home directory, which is the one
/// case where a window cannot be written down and a shell cannot find one.
///
/// Under test it is a scratch directory per test, for the same reason the settings file is:
/// a test must neither read the windows the developer running it has open nor leave records
/// of its own behind in their home directory.
fn dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        // Named by a digest of the test rather than by the test, and kept short: a unix
        // socket path has about a hundred characters to fit in, and a temporary directory
        // with a sentence of a test name in it does not.
        use sha1::Digest;
        let test = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let digest = sha1::Sha1::digest(test.as_bytes());
        Some(std::env::temp_dir().join(format!(
            "moon-i-{}-{:x}{:x}{:x}{:x}",
            std::process::id(),
            digest[0],
            digest[1],
            digest[2],
            digest[3]
        )))
    }
    #[cfg(not(test))]
    {
        home_instances_dir()
    }
}

/// Where the records really live: beside the settings, in the directory this program already
/// keeps what belongs to the person in.
fn home_instances_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(SETTINGS_DIR_NAME)
            .join(INSTANCES_DIR_NAME),
    )
}

fn record_path(pid: u32) -> Option<PathBuf> {
    Some(dir()?.join(format!("{pid}.json")))
}

/// Where the moon running as this process listens, as this process finds it.
fn socket_path(pid: u32) -> Option<PathBuf> {
    socket_path_from(
        pid,
        pid == std::process::id() && crate::unix_users::each_person_has_one(),
        std::env::var_os(SOCKET_ENV).map(PathBuf::from),
        dir(),
    )
}

/// [`socket_path`], from what it reads: whether `pid` is a server whose shells run as other
/// Unix users, the socket this shell was told - see [`SOCKET_ENV`] - and where the records
/// are.
///
/// The socket a shell was told is believed for the process it is named after and no other.
/// A window opened from a shell of such a server gives its own shells its own process, and
/// they are still told the server's socket.
fn socket_path_from(
    pid: u32,
    serves_other_unix_users: bool,
    told: Option<PathBuf>,
    records_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    let name = format!("{pid}.sock");
    if serves_other_unix_users {
        return Some(Path::new(SHARED_SOCKETS_DIR).join(name));
    }
    if let Some(told) = told.filter(|told| told.file_name() == Some(OsStr::new(&name))) {
        return Some(told);
    }
    Some(records_dir?.join(name))
}

/// The socket this process listens on, for the shells it starts to be told - see
/// [`SOCKET_ENV`]. `None` for every moon but a server that gives each person a Unix user: the
/// shells of any other find its socket in their own home, which is the moon's.
pub(crate) fn socket_for_other_unix_users() -> Option<PathBuf> {
    crate::unix_users::each_person_has_one()
        .then(|| socket_path(std::process::id()))
        .flatten()
}

/// Open the socket this process is asked on. It is named after the process, so a shell that
/// knows which moon started it knows where to ask.
fn listen_on_own_socket() -> Result<UnixListener> {
    let path = socket_path(std::process::id()).context("no home directory to listen in")?;
    let dir = path.parent().expect("the socket sits in a directory");
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    // A pid this process was handed again may have left its socket behind; binding to a
    // path that already exists fails, and that file cannot belong to anyone else.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("failed to listen on {}", path.display()))?;
    if crate::unix_users::each_person_has_one() {
        // Said outright rather than left to the mask the server runs with: connecting takes
        // leave to write to the socket, and the people's users are not in root's group.
        for (opened, mode) in [(dir, SHARED_SOCKETS_DIR_MODE), (&*path, SHARED_SOCKET_MODE)] {
            std::fs::set_permissions(opened, std::fs::Permissions::from_mode(mode))
                .with_context(|| format!("failed to open {} to every user", opened.display()))?;
        }
        remove_sockets_of_servers_that_are_gone(dir);
    }
    Ok(listener)
}

/// Take away the sockets of servers that were killed rather than stopped, which leave theirs
/// behind in a folder only root clears. Each is named after its server's process.
fn remove_sockets_of_servers_that_are_gone(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let server = path
            .file_stem()
            .and_then(|pid| pid.to_str())
            .and_then(|pid| pid.parse::<u32>().ok());
        if let Some(server) = server
            && !is_running(server)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Read the one ask a connection carries.
fn read_ask(stream: &UnixStream) -> Result<Ask> {
    let mut asked = String::new();
    BufReader::new(stream)
        .read_line(&mut asked)
        .context("failed to read the ask")?;
    serde_json::from_str(asked.trim())
        .with_context(|| format!("failed to read {asked:?} as an ask"))
}

/// Write the answer to an ask back to the shell waiting on it.
fn write_answer(mut stream: &UnixStream, answer: &Answer) -> Result<()> {
    writeln!(stream, "{}", serde_json::to_string(answer)?)?;
    stream.flush().context("failed to answer the shell")
}

/// What came of an ask, as its answer: a failure is the refusal, in the words of whatever
/// failed.
fn answered(came_of_it: Result<Answer>) -> Answer {
    came_of_it.unwrap_or_else(|error| Answer::Refused {
        reason: format!("{error:#}"),
    })
}

/// Start the program a `moon launch` asked for, and answer with how its start went: one that
/// failed is refused in the words of whatever failed, the program's own among them.
///
/// `asker` is the person whose shell asked, on a server that gives each person a Unix user,
/// and nobody everywhere else - see [`StartsApplications::start_for_person`].
fn launched(
    applications: &dyn StartsApplications,
    asker: Option<&Person>,
    command: &str,
    folder: &str,
) -> Answer {
    let folder = Path::new(folder);
    let started = match asker {
        Some(person) => applications.start_for_person(person, command, folder),
        None => applications.start(command, folder),
    };
    answered(started.map(|on| Answer::Launched { on }))
}

/// Write down that this process's window is open on this project, replacing what it said
/// before - a window that opens another project is the same window on somewhere else.
fn write_record(instance: &Instance) -> Result<()> {
    let path = record_path(instance.pid).context("no home directory to write the window in")?;
    let dir = path.parent().expect("a record sits in the instances dir");
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let text = serde_json::to_string_pretty(instance)?;
    std::fs::write(&path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Take a window's record and socket away, which is what closing one does and what reading
/// the records does to a window that is no longer running.
fn remove_records(pid: u32) {
    for path in [record_path(pid), socket_path(pid)].into_iter().flatten() {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the process behind a record is still there. A pid belonging to somebody else
/// answers `EPERM` rather than `ESRCH`, and that is still a process, so signal 0 succeeding
/// is not the only way to be alive - what matters here is that it is not gone.
fn is_running(pid: u32) -> bool {
    // Safety: signal 0 sends nothing. It is the ordinary way to ask whether a pid exists.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Every window running on this machine, newest record first, with the records of windows
/// that are gone cleared away as they are read.
pub(crate) fn running() -> Vec<Instance> {
    let Some(dir) = dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut found: Vec<(std::time::SystemTime, Instance)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(instance) = serde_json::from_str::<Instance>(&text) else {
            continue;
        };
        if !is_running(instance.pid) {
            remove_records(instance.pid);
            continue;
        }
        let written = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        found.push((written, instance));
    }

    found.sort_by_key(|(written, _)| std::cmp::Reverse(*written));
    found.into_iter().map(|(_, instance)| instance).collect()
}

/// The windows that could open this path - a file, or a folder to start a shell in - the
/// likeliest first: the window whose shell the ask was typed in, then every window whose
/// project holds the path, the innermost project first - a window on a submodule is a better
/// answer than one on the repo around it - and then the rest of the windows, the one most
/// recently in front first.
///
/// The rest are there because a path whose project no window is open on still has to land
/// somewhere: the window being looked at opens a session on that project and puts the tab
/// in it, which beats an error telling somebody to open a window first.
///
/// `path` has to be absolute and resolved, as the project paths in the records are, or a
/// project holding it cannot be recognised.
pub(crate) fn windows_for(
    path: &Path,
    shell_window: Option<u32>,
    mut running: Vec<Instance>,
) -> Vec<Instance> {
    running.sort_by_key(|instance| {
        let holds_the_path = path.starts_with(&instance.project_path);
        (
            Some(instance.pid) != shell_window,
            !holds_the_path,
            // The innermost project wins among the windows that hold the path, and says
            // nothing about the ones that do not - they are ordered by their time in front.
            match holds_the_path {
                true => usize::MAX - instance.project_path.len(),
                false => 0,
            },
            std::cmp::Reverse(instance.focused_at_unix),
        )
    });
    running
}

/// The window a `moon open` was typed in, when it was typed in one of a window's shells.
pub(crate) fn shell_window() -> Option<u32> {
    std::env::var(WINDOW_ENV).ok()?.parse().ok()
}

/// The GitHub login of the person this shell was started for - see [`PERSON_ENV`].
pub(crate) fn shell_person() -> Option<String> {
    std::env::var(PERSON_ENV).ok()
}

/// Hand a file to a window: the first one that takes it, in the order [`windows_for`] puts
/// them in.
pub(crate) fn open_file(file: &Path, line: Option<usize>, wait: bool) -> Result<Instance> {
    let ask = Ask::OpenFile {
        path: file.display().to_string(),
        line,
        wait,
    };
    hand_to_a_window(file, &ask)
}

/// Hand a folder to a window for a shell to be started in: the first one that takes it, in
/// the order [`windows_for`] puts them in. `None` when no window is open on this machine at
/// all, which is the one case a shell has somewhere else to go - a window of its own.
pub(crate) fn open_shell(folder: &Path) -> Result<Option<Instance>> {
    if running().is_empty() {
        return Ok(None);
    }
    let ask = Ask::OpenShell {
        folder: folder.display().to_string(),
    };
    hand_to_a_window(folder, &ask).map(Some)
}

/// Hand a folder to a window for its file picker to be brought up on: the first one that
/// takes it, in the order [`windows_for`] puts them in - the same windows, in the same order,
/// that a file of that folder would be handed to.
pub(crate) fn pick_file(folder: &Path) -> Result<Instance> {
    let ask = Ask::PickFile {
        folder: folder.display().to_string(),
    };
    hand_to_a_window(folder, &ask)
}

/// Ask the windows that could take `path` in turn, and say which one did. A window that
/// refuses says why, and the last of those reasons is what is reported when no window took
/// it - it is the nearest thing to an explanation there is.
///
/// A window that is there and hangs up without an answer is refusing too: it was started by
/// a moon older than what is asked, and reads the ask as none it knows. Its record and its
/// socket are left alone - it still answers everything it does know.
fn hand_to_a_window(path: &Path, ask: &Ask) -> Result<Instance> {
    let candidates = windows_for(path, shell_window(), running());
    if candidates.is_empty() {
        bail!(
            "no moon window is open on this machine to put {} in",
            path.display()
        );
    }

    let mut refusal = None;
    for instance in candidates {
        // A window that cannot be reached is one that went away between the record being
        // read and the socket being opened. The next window is the answer, not an error.
        let Ok(stream) = reach(instance.pid) else {
            remove_records(instance.pid);
            continue;
        };
        match ask_over(&stream, ask, ANSWER_TIMEOUT) {
            Ok(Answer::Opened) => return Ok(instance),
            Ok(Answer::Refused { reason }) => refusal = Some(reason),
            Ok(other) => bail!("the window answered an open with {other:?}"),
            Err(_) => {
                refusal = Some(format!(
                    "the window on {} (process {}) did not answer - if it was started by an \
                     older moon it does not know what was asked, and has to be restarted",
                    instance.project_path, instance.pid
                ))
            }
        }
    }

    match refusal {
        Some(reason) => bail!("no window opened {}: {reason}", path.display()),
        None => bail!(
            "no window answered about {} - the ones that were open have closed",
            path.display()
        ),
    }
}

/// The window of a moon, by its process - or `None` for a process with no window written
/// down, which is what a `moon serve` is: it holds shells, and the socket it listens on
/// answers for them and for nothing a window opens - see [`server`].
pub(crate) fn window_of(pid: u32) -> Option<Instance> {
    running().into_iter().find(|instance| instance.pid == pid)
}

/// Hand a direct message of the wire to the moon holding the shell it is for - a window or a
/// `moon serve`, by its process - to be typed in there. Only that moon will do, so its
/// refusal is the answer rather than a reason to try another.
pub(crate) fn wire(moon: u32, line: &Ask) -> Result<()> {
    let answer = ask_process(moon, line, ANSWER_TIMEOUT)
        .with_context(|| format!("the moon holding that shell (process {moon}) did not answer"))?;
    match answer {
        Answer::Wired => Ok(()),
        Answer::Refused { reason } => {
            bail!("the moon holding that shell (process {moon}) refused the line: {reason}")
        }
        other => bail!("the moon answered a line of the wire with {other:?}"),
    }
}

/// Ask the one moon that can answer something about an agent, and take its refusal as the
/// answer rather than as a reason to try another: no other moon has a say in where that
/// board's agents run, or holds that shell.
///
/// A moon that says nothing is most often one started before this was something it could be
/// asked: it reads the ask as none it knows, and hangs up.
fn ask_about_an_agent(moon: u32, ask: &Ask) -> Result<Answer> {
    let answer = ask_process(moon, ask, ANSWER_TIMEOUT).with_context(|| {
        format!(
            "the moon (process {moon}) did not answer - one started by an older moon does not \
             know what was asked, and has to be restarted"
        )
    })?;
    match answer {
        Answer::Refused { reason } => bail!("the moon (process {moon}) refused: {reason}"),
        answer => Ok(answer),
    }
}

/// The window an agent of the board in this repo is started in: one open on that repo, the
/// one this shell is in before any other and then the one most recently in front - see
/// [`windows_for`]. `None` when no window is open on it.
///
/// A window on another project is not asked. The shell an agent runs in is held by the
/// window that started it, and the board that shows the run is the one in that window.
fn window_on_board(repo_path: &Path) -> Option<Instance> {
    windows_for(repo_path, shell_window(), running())
        .into_iter()
        .find(|instance| Path::new(&instance.project_path) == repo_path)
}

/// The moon an agent of the board in this repo is started in, by its process: a window open
/// on that repo - see [`window_on_board`] - and, when there is none, the `moon serve` this
/// shell was started by. A server is written down as no window and listens all the same; it
/// holds the shells of every board it is asked about.
pub(crate) fn moon_for_board(repo_path: &Path) -> Option<u32> {
    if let Some(window) = window_on_board(repo_path) {
        return Some(window.pid);
    }
    let moon = shell_window()?;
    let is_a_server = window_of(moon).is_none() && socket_path(moon)?.exists();
    is_a_server.then_some(moon)
}

/// Have a moon start an agent on a task of the board in this repo, and say what the run is
/// called.
pub(crate) fn start_agent(
    moon: u32,
    repo_path: &Path,
    task_id: &str,
    agent: AgentKind,
) -> Result<String> {
    let ask = Ask::StartAgent {
        repo_path: repo_path.display().to_string(),
        task_id: task_id.to_string(),
        agent,
    };
    match ask_about_an_agent(moon, &ask)? {
        Answer::Started { run } => Ok(run),
        other => bail!("the moon answered a start with {other:?}"),
    }
}

/// Have the moon holding a shell type a line into it and send it.
pub(crate) fn tell(moon: u32, terminal_id: &str, line: &str) -> Result<()> {
    let ask = Ask::Tell {
        terminal_id: terminal_id.to_string(),
        line: line.to_string(),
    };
    match ask_about_an_agent(moon, &ask)? {
        Answer::Told => Ok(()),
        other => bail!("the moon answered a line for a shell with {other:?}"),
    }
}

/// What the moon holding a shell says it is showing.
pub(crate) fn shown(moon: u32, terminal_id: &str, wanted: Shown) -> Result<String> {
    let ask = Ask::Shown {
        terminal_id: terminal_id.to_string(),
        wanted,
    };
    match ask_about_an_agent(moon, &ask)? {
        Answer::Shown { text } => Ok(text),
        other => bail!("the moon answered a look at a shell with {other:?}"),
    }
}

/// Have the moon this shell was started by start a program for its windows, run in `folder`,
/// and wait to hear how the start went: on the desktop of a `moon serve`, or on the screen a
/// `moon desktop` is the session of. Answers with where the program's windows open.
///
/// Only that moon is asked. The program is started where the shell is, and its windows are
/// shown by the moon the shell is a tab of - no other moon on the machine is either.
pub(crate) fn launch(command: &str, folder: &Path) -> Result<String> {
    let Some(moon) = shell_window() else {
        bail!(
            "this shell was not started by a moon, so there is no moon to start a program in: \
             `moon launch` is typed in a shell tab of `moon serve` or of `moon desktop`"
        );
    };
    launch_in(moon, command, folder)
}

/// The same, of the moon running as this process. Its refusal is the whole of what there is
/// to say: how the program failed as it started, or why this moon starts none.
fn launch_in(moon: u32, command: &str, folder: &Path) -> Result<String> {
    let ask = Ask::Launch {
        command: command.to_string(),
        folder: folder.display().to_string(),
    };
    let answer = ask_process(moon, &ask, LAUNCH_ANSWER_TIMEOUT).with_context(|| {
        format!(
            "the moon this shell was started by (process {moon}) did not answer - it has \
             ended, or it was started by an older moon, which does not know what was asked \
             and has to be restarted"
        )
    })?;
    match answer {
        Answer::Launched { on } => Ok(on),
        Answer::Refused { reason } => bail!("{reason}"),
        other => bail!("the moon answered a launch with {other:?}"),
    }
}

/// How often `moon edit --wait` asks whether the file's tab is still open. Someone closing a
/// tab and looking back at the shell does not notice a quarter of a second.
const STILL_OPEN_POLL: Duration = Duration::from_millis(250);

/// Wait until the window that took a file for `moon edit --wait` has closed its tab - which
/// is what git waits on before it reads the commit message back. A window that goes away
/// took the tab with it, so that is the end of the wait too.
pub(crate) fn wait_until_closed(instance: &Instance, file: &Path) -> Result<()> {
    let ask = Ask::StillOpen {
        path: file.display().to_string(),
    };
    loop {
        match instance.ask(&ask) {
            Ok(Answer::StillOpen) => std::thread::sleep(STILL_OPEN_POLL),
            Ok(Answer::Closed) | Err(_) => return Ok(()),
            Ok(other) => bail!("the window answered a wait with {other:?}"),
        }
    }
}
