//! The window's end of `moon open`, `moon shell <folder>` and `moon wire post @handle`: the
//! socket a shell reaches it on, and the files, folders and lines that have come in over it
//! waiting for the next frame to open them or type them.
//!
//! `moon agent start`, `tell` and `view` come in over it too and wait for no frame: each is
//! answered with what came of it, by whoever the window [said](ShellAsks::agents_answered_by)
//! answers about agents - see [`AgentAsks`].
//!
//! `moon launch` comes in over it as well, and is answered the same way by whoever the window
//! [said](ShellAsks::applications_started_by) starts programs - which only a window that is
//! its machine's session does.
//!
//! Only a real window listens. Every other caller of the app is a ui test, and a test must
//! not put itself in the way of a `moon open` typed in the window the developer running it
//! has open - see [`crate::native::app::App::listen_for_shell_asks`].

use std::{
    collections::HashSet,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use super::{
    Answer, Ask, Instance, StartsApplications, launched, listen_on_own_socket, read_ask,
    remove_records, write_answer, write_record,
};
use crate::{api::AgentKind, terminal::Shown};

/// What a window does about the agents of its board for a shell that asked: `moon agent
/// start`, `tell` and `view`.
///
/// These are answered on the thread the socket is read on, with what came of them, where a
/// file to open is left for the window's next frame. Whoever asked is a command waiting to
/// print the answer - the name of the run, the text of the screen - and a window nobody is
/// looking at may draw no frame for a long while. Nothing here needs one: an agent is started
/// by the window's moon and listed by the board as it reads the task again.
pub(crate) trait AgentAsks: Send + Sync {
    /// Start an agent on a task of the board in this repo, and say what its run is called.
    fn start(&self, repo_path: &str, task_id: &str, agent: AgentKind) -> Result<String>;
    /// Type a line into a shell and send it, once nothing there is in the way.
    fn tell(&self, terminal_id: &str, line: &str) -> Result<()>;
    /// What a shell is showing, one line per row.
    fn shown(&self, terminal_id: &str, wanted: Shown) -> Result<String>;
}

/// Who answers about agents, once the window has said - see [`ShellAsks::agents_answered_by`].
type AnswersAboutAgents = Arc<Mutex<Option<Arc<dyn AgentAsks>>>>;

/// Who starts the programs `moon launch` asks this window for, once the window has said - see
/// [`ShellAsks::applications_started_by`].
type StartsWhatIsLaunched = Arc<Mutex<Option<Arc<dyn StartsApplications>>>>;

/// What a window that starts no programs answers a `moon launch` with.
const STARTS_NO_PROGRAMS: &str = "this moon is a window among others on its machine's own \
    screen, which has its own way to start a program: `moon launch` is for a shell tab of \
    `moon serve` or of `moon desktop`";

/// A file a shell asked this window to open.
pub(crate) struct OpenFileAsked {
    pub(crate) path: PathBuf,
    /// The line to put on screen, for `moon open <file>:<line>`.
    pub(crate) line: Option<usize>,
    /// A `moon edit --wait` is waiting on this file's tab to close - see
    /// [`ShellAsks::release`].
    pub(crate) wait: bool,
}

/// A folder a shell asked this window to start a shell in: `moon shell <folder>`.
pub(crate) struct OpenShellAsked {
    /// Absolute and resolved, the way the shell that asked named it.
    pub(crate) folder: PathBuf,
}

/// A direct message of the wire a shell asked this window to type into one of the shells it
/// holds: `moon wire post @handle …`.
pub(crate) struct WiredLine {
    pub(crate) terminal_id: String,
    /// The handles of the task it was posted from and of the task it is for.
    pub(crate) sender: String,
    pub(crate) recipient: String,
    pub(crate) message: String,
}

/// What a window keeps so shells can reach it: the project it is written down as being on,
/// and the asks that have arrived since the last frame.
pub(crate) struct ShellAsks {
    /// Read by the listening thread to decide whether this window has anything to open a
    /// file into yet, and written by the window whenever it opens another project.
    project: Arc<Mutex<Option<String>>>,
    arrived: Arc<Mutex<Vec<OpenFileAsked>>>,
    /// The folders `moon shell <folder>` asked for a shell in, kept apart from the files: a
    /// shell is started rather than opened in a tab, so it goes a different way.
    arrived_shells: Arc<Mutex<Vec<OpenShellAsked>>>,
    /// The lines `moon wire post @handle` asked to have typed, in the order they were asked
    /// for - which is the order the window types them in.
    arrived_wired: Arc<Mutex<Vec<WiredLine>>>,
    /// The files a `moon edit --wait` is waiting on, from the moment the ask is taken until
    /// the window [releases](ShellAsks::release) them. Written into by the listening thread
    /// as the ask arrives, so a shell asking straight after is told the file is still open
    /// even though no frame has opened its tab yet.
    waited_on: Arc<Mutex<HashSet<PathBuf>>>,
    /// Who answers `moon agent start`, `tell` and `view`. Nobody until the window says, which
    /// is what a window in a test never does.
    agents: AnswersAboutAgents,
    /// Who starts what `moon launch` asks for. Nobody until the window says, which only a
    /// window that is its machine's session does: every other window refuses.
    applications: StartsWhatIsLaunched,
    /// What this window is called on the command line, which is what `moon list` prints
    /// beside the project.
    program: String,
    /// When this window was last brought to the front, which is written into its record so a
    /// shell can tell the window being looked at from the ones behind it.
    focused_at_unix: Arc<Mutex<u64>>,
}

impl ShellAsks {
    /// Open the window's socket and start answering on it. The window is not written down
    /// yet: it has nothing to be found by until it is open on a project, which is what
    /// [`ShellAsks::on_project`] says.
    pub(crate) fn listen(
        program: String,
        reads_this_machine: bool,
        ctx: egui::Context,
    ) -> Result<Self> {
        let listener = listen_on_own_socket()?;

        let asks = Self {
            project: Arc::new(Mutex::new(None)),
            arrived: Arc::new(Mutex::new(Vec::new())),
            arrived_shells: Arc::new(Mutex::new(Vec::new())),
            arrived_wired: Arc::new(Mutex::new(Vec::new())),
            waited_on: Arc::new(Mutex::new(HashSet::new())),
            agents: Arc::new(Mutex::new(None)),
            applications: Arc::new(Mutex::new(None)),
            program,
            focused_at_unix: Arc::new(Mutex::new(0)),
        };
        let project = asks.project.clone();
        let arrived = Arrived {
            files: asks.arrived.clone(),
            shells: asks.arrived_shells.clone(),
            wired: asks.arrived_wired.clone(),
            waited_on: asks.waited_on.clone(),
        };
        let agents = asks.agents.clone();
        let applications = asks.applications.clone();
        thread::Builder::new()
            .name("moon-shell-asks".to_string())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    // One ask per connection, and each is answered before the next is read:
                    // a shell waits for its answer, so nothing is gained by doing several at
                    // once, and the window is only ever asked as fast as somebody types.
                    if let Err(error) = answer(
                        stream,
                        reads_this_machine,
                        &project,
                        &arrived,
                        &agents,
                        &applications,
                    ) {
                        eprintln!("[moonreview] could not answer a `moon` ask: {error}");
                        continue;
                    }
                    // The file lands in a tab on the next frame, and a window nobody is
                    // looking at draws no frames until something asks it to.
                    ctx.request_repaint();
                }
            })
            .context("failed to start the thread answering shells")?;

        Ok(asks)
    }

    /// Say who answers `moon agent start`, `tell` and `view` asked of this window. Until it
    /// is said they are refused.
    pub(crate) fn agents_answered_by(&self, agents: Arc<dyn AgentAsks>) {
        *self.agents.lock().expect("the agents lock") = Some(agents);
    }

    /// Say who starts the programs `moon launch` asks this window for. Until it is said they
    /// are refused, with [`STARTS_NO_PROGRAMS`].
    ///
    /// Only a window on Linux is ever its machine's session, so only there - and in the tests
    /// - is there anything to say it with.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn applications_started_by(&self, applications: Arc<dyn StartsApplications>) {
        *self.applications.lock().expect("the applications lock") = Some(applications);
    }

    /// Say which project this window is on, so a shell asking about a file of it finds this
    /// window. Writing it again for the project it already says is harmless and is what a
    /// window that reopened the same project does.
    pub(crate) fn on_project(&self, project_path: &str) -> Result<()> {
        *self.project.lock().expect("the project lock") = Some(project_path.to_string());
        self.write_down(project_path.to_string())
    }

    /// Say this window has just been brought to the front. A file whose project no window is
    /// open on goes to the window that was in front most recently, so the moment it comes
    /// forward is the moment worth writing down.
    ///
    /// Nothing to write before the window is on a project: it has no record until then, and
    /// [`ShellAsks::on_project`] carries the time in with it when it writes the first one.
    pub(crate) fn came_to_the_front(&self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past 1970")
            .as_secs();
        *self.focused_at_unix.lock().expect("the focus lock") = now;

        let Some(project_path) = self.project.lock().expect("the project lock").clone() else {
            return Ok(());
        };
        self.write_down(project_path)
    }

    fn write_down(&self, project_path: String) -> Result<()> {
        write_record(&Instance {
            pid: std::process::id(),
            program: self.program.clone(),
            project_path,
            focused_at_unix: *self.focused_at_unix.lock().expect("the focus lock"),
        })
    }

    /// The tab a `moon edit --wait` was waiting on has closed, or will never open: the shell
    /// asking about it stops waiting.
    pub(crate) fn release(&self, path: &std::path::Path) {
        self.waited_on
            .lock()
            .expect("the waited-on lock")
            .remove(path);
    }

    /// The files asked for since the last time this was called.
    pub(crate) fn drain(&self) -> Vec<OpenFileAsked> {
        std::mem::take(&mut *self.arrived.lock().expect("the arrived lock"))
    }

    /// The folders a shell was asked for in since the last time this was called.
    pub(crate) fn drain_shells(&self) -> Vec<OpenShellAsked> {
        std::mem::take(&mut *self.arrived_shells.lock().expect("the arrived shells lock"))
    }

    /// The lines of the wire asked to be typed since the last time this was called, in the
    /// order they were asked for.
    pub(crate) fn drain_wired(&self) -> Vec<WiredLine> {
        std::mem::take(&mut *self.arrived_wired.lock().expect("the arrived wired lock"))
    }
}

/// Where the listening thread puts what arrives, for the window to drain on its next frame.
struct Arrived {
    files: Arc<Mutex<Vec<OpenFileAsked>>>,
    shells: Arc<Mutex<Vec<OpenShellAsked>>>,
    wired: Arc<Mutex<Vec<WiredLine>>>,
    waited_on: Arc<Mutex<HashSet<PathBuf>>>,
}

impl Drop for ShellAsks {
    fn drop(&mut self) {
        remove_records(std::process::id());
    }
}

/// Read one ask off a connection and answer it.
///
/// A file or folder of another project is taken as readily as one of this window's own: the
/// window opens a session on the project holding it and puts the tab in that - see
/// [`crate::native::open_from_shell`]. Which window is asked first is the shell's business,
/// and it asks the ones open on the path's project before any other.
///
/// What is refused is a window with nothing to open into: one still on its launch screen,
/// and one whose repo is on another machine, where a path typed in a shell here names
/// nothing at all.
///
/// A line of the wire is taken by any window that holds shells, on a project or not: the
/// shell it names is one this window's moon started. It is refused by a window whose repo is
/// on another machine, whose shells are that machine's, and when it is not the one line of
/// text a shell can be told - what arrives here is checked rather than trusted, since the
/// window answers for it before it is typed.
///
/// What is asked about an agent is answered here and now - see [`AgentAsks`] - and a failure
/// is the window's refusal, in the words of whatever failed. An agent is started only by a
/// window open on the board's own repo: the run is listed on the board in that window, and
/// its shell is one only that window can open.
///
/// A program to start is answered here and now as well, once its start is known to have gone
/// one way or the other - see [`StartsApplications`].
fn answer(
    stream: UnixStream,
    reads_this_machine: bool,
    project: &Arc<Mutex<Option<String>>>,
    arrived: &Arrived,
    agents: &AnswersAboutAgents,
    applications: &StartsWhatIsLaunched,
) -> Result<()> {
    let answer = match read_ask(&stream)? {
        Ask::OpenFile { path, line, wait } => match refusal(reads_this_machine, project) {
            Some(refused) => refused,
            None => {
                let path = PathBuf::from(path);
                if wait {
                    arrived
                        .waited_on
                        .lock()
                        .expect("the waited-on lock")
                        .insert(path.clone());
                }
                arrived
                    .files
                    .lock()
                    .expect("the arrived lock")
                    .push(OpenFileAsked { path, line, wait });
                Answer::Opened
            }
        },
        Ask::OpenShell { folder } => match refusal(reads_this_machine, project) {
            Some(refused) => refused,
            None => {
                arrived
                    .shells
                    .lock()
                    .expect("the arrived shells lock")
                    .push(OpenShellAsked {
                        folder: PathBuf::from(folder),
                    });
                Answer::Opened
            }
        },
        Ask::Wire {
            terminal_id,
            sender,
            recipient,
            message,
        } => match wire_refusal(reads_this_machine, &sender, &recipient, &message) {
            Some(refused) => refused,
            None => {
                arrived
                    .wired
                    .lock()
                    .expect("the arrived wired lock")
                    .push(WiredLine {
                        terminal_id,
                        sender,
                        recipient,
                        message,
                    });
                Answer::Wired
            }
        },
        Ask::StartAgent {
            repo_path,
            task_id,
            agent,
        } => match start_refusal(reads_this_machine, project, &repo_path) {
            Some(refused) => refused,
            None => answer_about_agents(agents, |agents| {
                let run = agents.start(&repo_path, &task_id, agent)?;
                Ok(Answer::Started { run })
            }),
        },
        Ask::Tell { terminal_id, line } => answer_about_agents(agents, |agents| {
            agents.tell(&terminal_id, &line)?;
            Ok(Answer::Told)
        }),
        Ask::Shown {
            terminal_id,
            wanted,
        } => answer_about_agents(agents, |agents| {
            let text = agents.shown(&terminal_id, wanted)?;
            Ok(Answer::Shown { text })
        }),
        Ask::StillOpen { path } => {
            if arrived
                .waited_on
                .lock()
                .expect("the waited-on lock")
                .contains(&PathBuf::from(path))
            {
                Answer::StillOpen
            } else {
                Answer::Closed
            }
        }
        Ask::Launch { command, folder } => {
            // Taken out of the lock before the start, which is waited on.
            let applications = applications.lock().expect("the applications lock").clone();
            match applications {
                Some(applications) => launched(applications.as_ref(), &command, &folder),
                None => Answer::Refused {
                    reason: STARTS_NO_PROGRAMS.to_string(),
                },
            }
        }
    };

    write_answer(&stream, &answer)
}

/// Why this window will not open anything a shell here names, or `None` when it will.
fn refusal(reads_this_machine: bool, project: &Arc<Mutex<Option<String>>>) -> Option<Answer> {
    match project.lock().expect("the project lock").clone() {
        Some(project) if !reads_this_machine => Some(Answer::Refused {
            reason: format!("this window is open on {project} on another machine"),
        }),
        Some(_) => None,
        None => Some(Answer::Refused {
            reason: "this window has no project open yet".to_string(),
        }),
    }
}

/// Why this window will not start an agent on a task of the board in `repo_path`, or `None`
/// when it will: it is open on that repo, on this machine.
fn start_refusal(
    reads_this_machine: bool,
    project: &Arc<Mutex<Option<String>>>,
    repo_path: &str,
) -> Option<Answer> {
    if let Some(refused) = refusal(reads_this_machine, project) {
        return Some(refused);
    }
    let project = project.lock().expect("the project lock").clone()?;
    (project != repo_path).then(|| Answer::Refused {
        reason: format!("this window is open on {project}, not on {repo_path}"),
    })
}

/// Answer something asked about an agent with what came of it, and with a refusal that says
/// what went wrong when it failed - or that nobody here answers about agents.
fn answer_about_agents(
    agents: &AnswersAboutAgents,
    ask: impl FnOnce(&dyn AgentAsks) -> Result<Answer>,
) -> Answer {
    let Some(agents) = agents.lock().expect("the agents lock").clone() else {
        return Answer::Refused {
            reason: "this window answers nothing about agents".to_string(),
        };
    };
    ask(agents.as_ref()).unwrap_or_else(|error| Answer::Refused {
        reason: format!("{error:#}"),
    })
}

/// Why this window will not type a line of the wire into a shell, or `None` when it will.
fn wire_refusal(
    reads_this_machine: bool,
    sender: &str,
    recipient: &str,
    message: &str,
) -> Option<Answer> {
    if !reads_this_machine {
        return Some(Answer::Refused {
            reason: "this window's shells run on another machine, so it holds none to type into"
                .to_string(),
        });
    }
    // Every part of what is typed and written down, held to a line of text.
    [sender, recipient, message]
        .into_iter()
        .find_map(|part| crate::moontasks::wire::one_line(part).err())
        .map(|error| Answer::Refused {
            reason: error.to_string(),
        })
}
