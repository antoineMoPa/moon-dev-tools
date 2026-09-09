//! The windows running on this machine, and how a shell reaches one.
//!
//! `moon open <file>` is meant to land in a window that is already open rather than start
//! another one, so every window writes down where it is and listens on a socket of its own:
//! `~/.moonreview/instances/<pid>.json` says which project the window with that pid is on
//! and when it was last in front, and `<pid>.sock` beside it is where it is asked to open a
//! file. Both are written when the window opens a project and taken away when it closes; a
//! window that was killed leaves them behind, and the next read clears those out.

pub(crate) mod window;

#[cfg(test)]
mod tests;

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const SETTINGS_DIR_NAME: &str = ".moonreview";
const INSTANCES_DIR_NAME: &str = "instances";

/// The pid of the window a shell was started by, which is how a `moon open` typed in one of
/// the window's own shells reaches that window rather than whichever other one holds the
/// file. Written into every shell the window starts - see [`crate::terminal`].
pub(crate) const WINDOW_ENV: &str = "MOON_INSTANCE";

/// How long a window is given to answer before the ask is taken as unanswerable and the next
/// window is tried. It is a local socket and the answer is written the moment the ask is
/// read, so this is only ever hit by a window that is not really there any more.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Open this file in a tab, at this line when one was named.
    OpenFile { path: String, line: Option<usize> },
}

/// What the window answers.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub(crate) enum Answer {
    /// The window has the file and is opening it.
    Opened,
    /// The window will not open it, and says why - it has no project open yet, or the repo
    /// it is open on is on another machine, so the file the shell named is not one it reads.
    Refused { reason: String },
}

impl Instance {
    /// Ask this window to do something, and wait for its answer.
    pub(crate) fn ask(&self, ask: &Ask) -> Result<Answer> {
        let socket_path =
            socket_path(self.pid).context("no home directory to reach a window in")?;
        let stream = UnixStream::connect(&socket_path)
            .with_context(|| format!("failed to reach {}", socket_path.display()))?;
        stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
        stream.set_write_timeout(Some(ANSWER_TIMEOUT))?;

        let mut writing = &stream;
        writeln!(writing, "{}", serde_json::to_string(ask)?)?;
        writing.flush()?;

        let mut answer = String::new();
        BufReader::new(&stream)
            .read_line(&mut answer)
            .context("the window said nothing")?;
        serde_json::from_str(answer.trim())
            .with_context(|| format!("the window answered with {answer:?}"))
    }
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

fn socket_path(pid: u32) -> Option<PathBuf> {
    Some(dir()?.join(format!("{pid}.sock")))
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

/// The windows that could open this file, the likeliest first: the window whose shell the
/// ask was typed in, then every window whose project holds the file, the innermost project
/// first - a window on a submodule is a better answer than one on the repo around it - and
/// then the rest of the windows, the one most recently in front first.
///
/// The rest are there because a file whose project no window is open on still has to land
/// somewhere: the window being looked at opens a session on that project and puts the file
/// in a tab of it, which beats an error telling somebody to open a window first.
///
/// `file` has to be absolute and resolved, as the project paths in the records are, or a
/// project holding it cannot be recognised.
pub(crate) fn windows_for(
    file: &Path,
    shell_window: Option<u32>,
    mut running: Vec<Instance>,
) -> Vec<Instance> {
    running.sort_by_key(|instance| {
        let holds_the_file = file.starts_with(&instance.project_path);
        (
            Some(instance.pid) != shell_window,
            !holds_the_file,
            // The innermost project wins among the windows that hold the file, and says
            // nothing about the ones that do not - they are ordered by their time in front.
            match holds_the_file {
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

/// Hand a file to a window: the first one that takes it, in the order [`windows_for`] puts
/// them in. A window that refuses says why, and the last of those reasons is what is
/// reported when no window took the file - it is the nearest thing to an explanation there is.
pub(crate) fn open_file(file: &Path, line: Option<usize>) -> Result<Instance> {
    let candidates = windows_for(file, shell_window(), running());
    if candidates.is_empty() {
        bail!(
            "no moon window is open on this machine to put {} in",
            file.display()
        );
    }

    let ask = Ask::OpenFile {
        path: file.display().to_string(),
        line,
    };
    let mut refusal = None;
    for instance in candidates {
        match instance.ask(&ask) {
            Ok(Answer::Opened) => return Ok(instance),
            Ok(Answer::Refused { reason }) => refusal = Some(reason),
            // A window that cannot be reached is one that went away between the record being
            // read and the socket being opened. The next window is the answer, not an error.
            Err(_) => remove_records(instance.pid),
        }
    }

    match refusal {
        Some(reason) => bail!("no window opened {}: {reason}", file.display()),
        None => bail!(
            "no window answered about {} - the ones that were open have closed",
            file.display()
        ),
    }
}
