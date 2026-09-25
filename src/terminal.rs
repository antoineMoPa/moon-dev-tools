mod answered_queries;
mod naming;
mod registry;
mod routes;
mod spawning;

pub(crate) use naming::{name_for_new_shell, name_for_new_shell_called, rename};
pub(crate) use routes::{
    close_terminal, create_terminal, list_terminals, rename_terminal, run_in_shell,
    start_workspace_shell, start_workspace_shell_in_folder, start_workspace_shell_running,
    terminal_socket, terminal_view, terminals_running_a_command, terminals_wanting_attention,
};

use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use portable_pty::{Child, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::{
    api::{AgentKind, TerminalAttentionView},
    attention::{Asked, Attention},
};

const OUTPUT_CHUNK_SIZE: usize = 8 * 1024;
/// The settings Claude is started with so it asks for a person through the terminal: OSC 9,
/// iTerm2's notification, and a bell with it. Its `auto` channel goes by `TERM_PROGRAM`,
/// which a moon shell does not set, and falls back to the bell alone.
const CLAUDE_NOTIFY_THROUGH_THE_TERMINAL: &str = r#"{"preferredNotifChannel":"iterm2_with_bell"}"#;

/// How long the shell has to have printed nothing before [`TerminalSpec::type_ahead`] is
/// typed.
///
/// An agent's input box only takes keys once its interface has been drawn, and nothing it
/// prints says when that is - but it stops printing once it has drawn one and is waiting.
/// Coming up takes each of the three under a second of drawing, and a quarter of a second of
/// silence after it is the box being ready, rather than a flat wait long enough for the
/// slowest machine.
const TYPE_AHEAD_QUIET: std::time::Duration = std::time::Duration::from_millis(250);
/// How long it waits for that silence before typing anyway.
///
/// An agent that is still drawing at this point - an animated banner, a slow machine - has a
/// box that has been taking keys for a while, so the wait is over.
const TYPE_AHEAD_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);
/// How often the wait looks at whether the shell has gone quiet.
const TYPE_AHEAD_POLL: std::time::Duration = std::time::Duration::from_millis(20);
/// How much shell output we keep so a reopened tab can replay what it missed.
const SCROLLBACK_LIMIT: usize = 256 * 1024;
const BROADCAST_CAPACITY: usize = 256;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Input {
        data: String,
    },
    /// The terminal answering a query the program made of it, which is not somebody typing.
    Reply {
        data: String,
    },
    /// The terminal reporting the pointer to the program, which is not somebody typing either.
    Report {
        data: String,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
}

#[derive(Deserialize)]
pub(crate) struct CreateTerminalRequest {
    command: Option<AgentKind>,
    /// A folder inside the repo to start the shell in, for `moon shell <folder>`. Without it
    /// the shell starts at the repo's root, which is where every other shell starts.
    #[serde(default)]
    folder: Option<String>,
}

/// A command line to start a shell with - see [`start_workspace_shell_running`].
#[derive(Deserialize)]
pub(crate) struct RunInShellRequest {
    command: String,
}

#[derive(Serialize)]
pub(crate) struct TerminalCreated {
    terminal_id: String,
}

#[derive(Serialize)]
pub(crate) struct TerminalList {
    terminal_ids: Vec<String>,
}

/// What runs in a pty: the user's login shell, or one of the agents.
///
/// A commit run is a login shell too, given its command as a `-c` script - see
/// [`crate::committing::start_commit_run`] for why it is a shell rather than `git` itself.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum TerminalProgram {
    LoginShell,
    Agent(AgentKind),
}

impl TerminalProgram {
    /// The login shell either way: [`AgentKind::None`] is "no agent picked" rather than a
    /// program to start.
    pub(crate) fn of_agent(agent: Option<AgentKind>) -> Self {
        match agent {
            None | Some(AgentKind::None) => Self::LoginShell,
            Some(agent) => Self::Agent(agent),
        }
    }

    /// The agent this is one of, for the callers that only deal in agents.
    fn agent(&self) -> Option<AgentKind> {
        match self {
            Self::Agent(agent) => Some(*agent),
            _ => None,
        }
    }

    /// What a shell of this program is called before its number - `claude`, `codex`, `shell`.
    fn label(&self) -> String {
        match self {
            Self::LoginShell => "shell".to_string(),
            Self::Agent(agent) => agent.label().to_lowercase(),
        }
    }
}

/// What to start a shell as. The plain case is a login shell in the reviewed repo; a task's
/// agent adds the program, its arguments, and the environment that tells it which task it is
/// working in.
pub(crate) struct TerminalSpec {
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) program: TerminalProgram,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    /// The task this shell belongs to, if any. An owned shell is the task's to list and to
    /// close, so it stays out of the workspace's own shells.
    pub(crate) owner: Option<String>,
    /// What the shell is called - see [`TerminalSession::name`]. A shell being started is
    /// given its name by [`name_for_new_shell`], or the name its run already had when it is
    /// resumed. `None` only where nothing named it, which is a test spawning by hand.
    pub(crate) name: Option<String>,
    /// Text typed into the shell once the program has come up, as if the user had typed it -
    /// exactly as given, down to whether it ends in a return.
    ///
    /// An agent's opening line ends without one: what that does is leave something written in
    /// the agent's box for the person to send. A build or run command ends with `\r`, because
    /// there is nobody to press return on a command the pane was asked to run. A write that
    /// lands too early - while the agent is still asking whether it trusts the folder - is
    /// lost rather than acted on, which is what makes typing at a program that has not said it
    /// is ready acceptable.
    pub(crate) type_ahead: Option<String>,
}

impl TerminalSpec {
    /// A shell of the workspace's own: no task, and no program unless an agent was asked for.
    pub(crate) fn shell(
        cwd: std::path::PathBuf,
        agent: Option<AgentKind>,
        name: Option<String>,
    ) -> Self {
        Self {
            cwd,
            program: TerminalProgram::of_agent(agent),
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            name,
            type_ahead: None,
        }
    }

    /// The same shell with one command typed into it and sent, which is how a project's build
    /// and run are started.
    ///
    /// The shell outlives the command: what it printed is still there to read, and the same
    /// tab is where the command is run again.
    pub(crate) fn running(cwd: std::path::PathBuf, command: &str) -> Self {
        Self {
            type_ahead: Some(format!("{command}\r")),
            ..Self::shell(cwd, None, None)
        }
    }
}

/// A shell that lives in the server, not in the tab showing it: closing the tab detaches
/// from the pty while it keeps running, so reopening it resumes the same shell.
pub(crate) struct TerminalSession {
    owner: Option<String>,
    /// What it was started as. A task's plain shells are listed from here, because they are
    /// not written down anywhere else.
    program: TerminalProgram,
    /// When it was started, and the order it was started in. A second is coarse enough that
    /// two shells opened together tie, so the order is what actually sorts them.
    started_at_unix: u64,
    order: u64,
    /// What the shell is called on its tab and on the board, if it has been named. A shell is
    /// named as it starts - `write the parser claude - 1`, `shell - 2` - so the task it
    /// belongs to and two runs of the same agent can be told apart. It can be renamed from
    /// its tab. A task's run writes the name down on the task as well, which is what
    /// outlives the shell - see
    /// [`crate::moontasks::store::TaskResource::name`].
    name: Mutex<Option<String>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    output: broadcast::Sender<Vec<u8>>,
    /// Flipped once the shell is gone. Attached tabs watch it so they learn about it even
    /// though they hold this session alive.
    ///
    /// An agent that fell over on its own is the exception: its flag stays false and its
    /// session stays in the registry, so the tabs showing the error stay open rather than
    /// closing over the only account of what went wrong - see the reader thread in
    /// [`TerminalRegistry::spawn`].
    exited: watch::Sender<bool>,
    scrollback: Mutex<Scrollback>,
    /// When the shell last printed anything, which is how the type-ahead tells a program that
    /// is still drawing its interface from one that has drawn it and is waiting.
    last_output: Mutex<Option<Instant>>,
    /// Whether a person has typed into this shell. Once they have, the type-ahead is dropped:
    /// text arriving after someone has started writing lands in the middle of their sentence.
    typed_into: std::sync::atomic::AtomicBool,
    /// The ask for a person the shell last made and nobody has answered - see
    /// [`crate::attention`]. Typing into the shell is the answer, and takes it off.
    attention: Mutex<Option<Attention>>,
    /// Reads the shell's output for those asks, across however many reads a sequence spans.
    attention_scanner: Mutex<crate::attention::Scanner>,
    /// Whether the program is gone while the session is kept - a failed agent held open for
    /// its error to be read. Input is discarded then: nothing reads the pty any more, and a
    /// write once its buffer fills would block whoever is typing.
    child_ended: std::sync::atomic::AtomicBool,
    /// Typing in a shell, and a shell printing output, both keep the server from idling out.
    last_activity: Arc<Mutex<Instant>>,
    /// The process started on the pty: the shell itself, or the agent. Kept here rather than
    /// asked of [`TerminalSession::child`], whose lock is held for as long as a wait on the
    /// program takes - see [`failure_notice`].
    child_pid: Option<u32>,
    /// What a Codex in this terminal has shown - see [`crate::visualizations`]. `None` for any
    /// other program.
    visualizations: Option<Mutex<crate::visualizations::rollout::CodexRollouts>>,
}

impl TerminalSession {
    /// Whether the shell has ended. Set by the reader thread when the pty reaches EOF.
    pub(crate) fn has_exited(&self) -> bool {
        *self.exited.borrow()
    }

    // A window on this machine drives the pty directly; a remote one goes through the
    // websocket.
    pub(crate) fn write_input(&self, data: &[u8]) -> anyhow::Result<()> {
        crate::api::mark_activity(&self.last_activity);
        self.typed_into();
        self.write_to_child(data)
    }

    /// Somebody typed into the shell: the type-ahead is off, and whatever the shell was
    /// asking for, it has been seen to.
    fn typed_into(&self) {
        self.typed_into.store(true, Ordering::Relaxed);
        *self.attention.lock().unwrap() = None;
    }

    /// The shell asked for a person. A bell after a notification is the same ask - Claude's
    /// `iterm2_with_bell` sends both - so it does not take the message's place; a
    /// notification says more than a bell and takes any bell's.
    fn asked_for_attention(&self, asked: Asked) {
        let mut attention = self.attention.lock().unwrap();
        let standing_notification = attention
            .as_ref()
            .is_some_and(|standing| matches!(standing.asked, Asked::Notification(_)));
        if asked == Asked::Bell && standing_notification {
            return;
        }
        *attention = Some(Attention::now(asked));
    }

    /// What the shell is asking for, as the window shows it.
    fn attention_view(&self, terminal_id: &str) -> Option<TerminalAttentionView> {
        let attention = self.attention.lock().unwrap().clone()?;
        Some(TerminalAttentionView {
            terminal_id: terminal_id.to_string(),
            name: self.name.lock().unwrap().clone(),
            asked: attention.asked,
            at_unix: attention.at_unix,
        })
    }

    /// The terminal answering the program's own query. It goes to the same place a keystroke
    /// does, but it is not one: nobody typed it, so it does not call off the type-ahead.
    pub(crate) fn write_reply(&self, data: &[u8]) -> anyhow::Result<()> {
        crate::api::mark_activity(&self.last_activity);
        self.write_to_child(data)
    }

    /// The terminal reporting the pointer to a program tracking it. Not typing either: it
    /// neither calls off the type-ahead nor answers what the shell is asking for - see
    /// [`crate::attention`] - since a pointer resting on a question is not an answer to it.
    pub(crate) fn write_report(&self, data: &[u8]) -> anyhow::Result<()> {
        crate::api::mark_activity(&self.last_activity);
        self.write_to_child(data)
    }

    /// Write to the program, unless it is gone and the session is only being kept for its
    /// error: nothing reads the pty then, and a write once its buffer fills would block.
    fn write_to_child(&self, data: &[u8]) -> anyhow::Result<()> {
        if self.child_ended.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.writer.lock().unwrap().write_all(data)?;
        Ok(())
    }

    /// Whether something is running in this shell right now, as opposed to a prompt waiting
    /// to be typed at. This is what quitting would actually take down with it.
    ///
    /// The pty's foreground process group is the answer for a login shell: it is the shell's
    /// own while the prompt is up, and the job's while one runs. A job someone sent to the
    /// background is not the foreground group and so does not count - it is also not what
    /// anybody means by the shell being busy.
    ///
    /// An agent is the program on the pty rather than something a shell was told to run, so
    /// it counts for as long as it is alive.
    pub(crate) fn is_running_a_command(&self) -> bool {
        if self.has_exited() || self.child_ended.load(Ordering::Relaxed) {
            return false;
        }
        if self.program.agent().is_some() {
            return true;
        }
        let (Some(shell), Some(foreground)) = (
            self.child_pid,
            self.master.lock().unwrap().process_group_leader(),
        ) else {
            // Nothing to compare, so nothing to say the shell is idle: the warning is the
            // safe answer when the pty cannot be read.
            return true;
        };
        u32::try_from(foreground).is_ok_and(|foreground| foreground != shell)
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

/// Output chunks kept for replay, oldest dropped once the byte budget is spent.
#[derive(Default)]
struct Scrollback {
    chunks: Vec<PrintedChunk>,
    bytes: usize,
}

/// A chunk of what the shell printed, and whether a window was attached to read it as it
/// came - which says whether the questions in it were answered. See
/// [`answered_queries`].
struct PrintedChunk {
    bytes: Vec<u8>,
    seen_live: bool,
}

impl Scrollback {
    fn push(&mut self, chunk: &[u8], seen_live: bool) {
        self.chunks.push(PrintedChunk {
            bytes: chunk.to_vec(),
            seen_live,
        });
        self.bytes += chunk.len();
        while self.bytes > SCROLLBACK_LIMIT && self.chunks.len() > 1 {
            self.bytes -= self.chunks.remove(0).bytes.len();
        }
    }

    /// Everything kept, for a window attaching now, with the questions an attached window
    /// already answered taken out - a second answer would reach the program as typing.
    /// Stretches seen live are joined before they are filtered, so a question split across
    /// two chunks is still found.
    fn replay(&self) -> Vec<u8> {
        let mut replay = Vec::with_capacity(self.bytes);
        for stretch in self
            .chunks
            .chunk_by(|before, after| before.seen_live == after.seen_live)
        {
            let printed: Vec<u8> = stretch
                .iter()
                .flat_map(|chunk| chunk.bytes.iter().copied())
                .collect();
            if stretch[0].seen_live {
                replay.extend(answered_queries::without_answered_queries(&printed));
            } else {
                replay.extend(printed);
            }
        }
        replay
    }
}

/// One of a task's live shells, as the board needs it listed.
pub(crate) struct OwnedShell {
    pub(crate) terminal_id: String,
    /// What it was renamed to, if it was - see [`TerminalSession::name`].
    pub(crate) name: Option<String>,
    pub(crate) started_at_unix: u64,
    /// Where it falls among the task's shells. Only meaningful against its siblings.
    pub(crate) order: u64,
}

pub(crate) struct TerminalRegistry {
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
    /// Tells this run of the server's shells from a previous one's. A task writes down the
    /// shell its agent is in, and that record outlives the process, so a counter starting
    /// over at zero would let a new shell answer to an old task's name.
    run: String,
    next_id: AtomicU64,
    last_activity: Arc<Mutex<Instant>>,
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
