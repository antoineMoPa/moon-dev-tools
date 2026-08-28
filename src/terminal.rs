use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use axum::{
    Json,
    extract::{
        Path as AxumPath, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::api::{AgentKind, AppError, AppState};

const OUTPUT_CHUNK_SIZE: usize = 8 * 1024;
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
    Input { data: String },
    /// The terminal answering a query the program made of it, which is not somebody typing.
    Reply { data: String },
    Resize { cols: u16, rows: u16 },
}

#[derive(Deserialize)]
pub(crate) struct CreateTerminalRequest {
    command: Option<AgentKind>,
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
/// A commit run is a login shell too, with the command typed into it - see
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
    /// Text typed into the shell once the program has come up, as if the user had typed it -
    /// exactly as given, down to whether it ends in a return.
    ///
    /// An agent's opening line ends without one: what that does is leave something written in
    /// the agent's box for the person to send. A commit run's line ends with `\r`, because
    /// there is nobody to press return on a command the pane was asked to run. A write that
    /// lands too early - while the agent is still asking whether it trusts the folder - is
    /// lost rather than acted on, which is what makes typing at a program that has not said it
    /// is ready acceptable.
    pub(crate) type_ahead: Option<String>,
}

impl TerminalSpec {
    /// A shell of the workspace's own: no program, no task.
    pub(crate) fn shell(cwd: std::path::PathBuf, agent: Option<AgentKind>) -> Self {
        Self {
            cwd,
            program: TerminalProgram::of_agent(agent),
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            type_ahead: None,
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
    /// Whether the program is gone while the session is kept - a failed agent held open for
    /// its error to be read. Input is discarded then: nothing reads the pty any more, and a
    /// write once its buffer fills would block whoever is typing.
    child_ended: std::sync::atomic::AtomicBool,
    /// Typing in a shell, and a shell printing output, both keep the server from idling out.
    last_activity: Arc<Mutex<Instant>>,
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
        self.typed_into.store(true, Ordering::Relaxed);
        self.write_to_child(data)
    }

    /// The terminal answering the program's own query. It goes to the same place a keystroke
    /// does, but it is not one: nobody typed it, so it does not call off the type-ahead.
    pub(crate) fn write_reply(&self, data: &[u8]) -> anyhow::Result<()> {
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
    chunks: Vec<Vec<u8>>,
    bytes: usize,
}

impl Scrollback {
    fn push(&mut self, chunk: &[u8]) {
        self.chunks.push(chunk.to_vec());
        self.bytes += chunk.len();
        while self.bytes > SCROLLBACK_LIMIT && self.chunks.len() > 1 {
            self.bytes -= self.chunks.remove(0).len();
        }
    }

    fn replay(&self) -> Vec<u8> {
        self.chunks.concat()
    }
}

/// One of a task's live shells, as the board needs it listed.
pub(crate) struct OwnedShell {
    pub(crate) terminal_id: String,
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

impl TerminalRegistry {
    pub(crate) fn new(last_activity: Arc<Mutex<Instant>>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            run: crate::moontasks::store::new_uuid()[..8].to_string(),
            next_id: AtomicU64::new(0),
            last_activity,
        }
    }

    fn get(&self, terminal_id: &str) -> Option<Arc<TerminalSession>> {
        self.sessions.lock().unwrap().get(terminal_id).cloned()
    }

    /// Attach the native window to a shell: everything it has printed so far, then
    /// everything it prints from here on, delivered to whichever thread owns the
    /// terminal emulator. Web tabs attach to the same shell over a websocket.
    pub(crate) fn attach(
        &self,
        terminal_id: &str,
    ) -> anyhow::Result<(std::sync::mpsc::Receiver<Vec<u8>>, Arc<TerminalSession>)> {
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow::anyhow!("unknown terminal {terminal_id}"))?;
        // Subscribe before replaying so nothing written in between is lost.
        let mut output = session.output.subscribe();
        let replay = session.scrollback.lock().unwrap().replay();

        let (sender, receiver) = std::sync::mpsc::channel();
        if !replay.is_empty() {
            let _ = sender.send(replay);
        }

        std::thread::spawn(move || {
            loop {
                match output.blocking_recv() {
                    Ok(chunk) => {
                        if sender.send(chunk).is_err() {
                            return;
                        }
                    }
                    // Lagged: the window fell behind, keep going with what follows.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Ok((receiver, session))
    }

    /// The shells the workspace has of its own - a task's shells are the task's to show.
    pub(crate) fn terminal_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.owner.is_none())
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Whether this shell is still one the server has, which is what tells a task's recorded
    /// agent run from one that ended - or that died with a previous run of the server.
    pub(crate) fn is_live(&self, terminal_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(terminal_id)
    }

    /// The plain shells one task has open right now, oldest first.
    ///
    /// This is the whole record of them: a shell has nothing to come back to once it ends, so
    /// the board lists the ones the server has rather than remembering the ones it had.
    pub(crate) fn owned_shells(&self, owner: &str) -> Vec<OwnedShell> {
        let mut shells: Vec<OwnedShell> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| {
                session.owner.as_deref() == Some(owner)
                    && session.program == TerminalProgram::LoginShell
            })
            .map(|(terminal_id, session)| OwnedShell {
                terminal_id: terminal_id.clone(),
                started_at_unix: session.started_at_unix,
                order: session.order,
            })
            .collect();
        shells.sort_by_key(|shell| shell.order);
        shells
    }

    /// End every shell belonging to one task, which is what finishing the task does.
    pub(crate) fn remove_owned_by(&self, owner: &str) {
        let owned: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.owner.as_deref() == Some(owner))
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect();
        for terminal_id in owned {
            self.remove(&terminal_id);
        }
    }

    pub(crate) fn remove(&self, terminal_id: &str) {
        let removed = self.sessions.lock().unwrap().remove(terminal_id);
        let Some(session) = removed else {
            return;
        };
        let mut child = session.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    pub(crate) fn spawn(self: &Arc<Self>, spec: TerminalSpec) -> anyhow::Result<String> {
        let pty = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut command = match &spec.program {
            TerminalProgram::LoginShell => {
                let mut command = CommandBuilder::new(crate::shell_path::login_shell());
                command.arg("-l");
                command
            }
            TerminalProgram::Agent(AgentKind::None) => {
                unreachable!("no agent picked is the login shell, see TerminalProgram::of_agent")
            }
            TerminalProgram::Agent(AgentKind::Claude) => CommandBuilder::new("claude"),
            TerminalProgram::Agent(AgentKind::Codex) => CommandBuilder::new("codex"),
            TerminalProgram::Agent(AgentKind::OpenCode) => CommandBuilder::new("opencode"),
        };
        for argument in &spec.args {
            command.arg(argument);
        }
        command.cwd(&spec.cwd);
        command.env("TERM", "xterm-256color");
        // Which terminal a passphrase can be asked on. gpg launches pinentry and then fails
        // with "Inappropriate ioctl for device" when this is unset, which is what a signed
        // commit started from a window rather than a shell would otherwise hit: the window
        // inherits no GPG_TTY, and the pty it just opened is the terminal to name.
        if let Some(tty_name) = pty.master.tty_name() {
            command.env("GPG_TTY", tty_name);
        }
        // The agent is started by name, so it has to be looked up on the PATH the user's shell
        // has rather than the one a desktop launcher hands this process.
        command.env("PATH", crate::shell_path::installed_tools_path());
        for (name, value) in &spec.env {
            command.env(name, value);
        }
        let child = pty.slave.spawn_command(command)?;
        // The slave handle must be dropped so the reader sees EOF once the shell exits.
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader()?;
        let writer = pty.master.take_writer()?;
        let (output, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (exited, _) = watch::channel(false);

        let order = self.next_id.fetch_add(1, Ordering::Relaxed);
        let terminal_id = format!("terminal-{}-{order}", self.run);
        let session = Arc::new(TerminalSession {
            owner: spec.owner,
            program: spec.program.clone(),
            started_at_unix: crate::moontasks::store::now_unix(),
            order,
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
            child: Mutex::new(child),
            output: output.clone(),
            exited: exited.clone(),
            scrollback: Mutex::new(Scrollback::default()),
            last_output: Mutex::new(None),
            typed_into: std::sync::atomic::AtomicBool::new(false),
            child_ended: std::sync::atomic::AtomicBool::new(false),
            last_activity: Arc::clone(&self.last_activity),
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(terminal_id.clone(), Arc::clone(&session));

        // Typed from a thread of its own, so starting a shell answers straight away rather
        // than after the wait.
        if let Some(text) = spec.type_ahead {
            let typing_into = Arc::clone(&session);
            std::thread::spawn(move || type_ahead(&typing_into, &text));
        }

        let registry = Arc::clone(self);
        let reaped_id = terminal_id.clone();
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; OUTPUT_CHUNK_SIZE];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let chunk = &buffer[..count];
                        crate::api::mark_activity(&session.last_activity);
                        *session.last_output.lock().unwrap() = Some(Instant::now());
                        session.scrollback.lock().unwrap().push(chunk);
                        // No attached tab is normal: the shell keeps running regardless.
                        let _ = output.send(chunk.to_vec());
                    }
                }
            }
            // An agent that fell over on its own - `claude --resume` on a session id that no
            // longer exists, say - has printed the only account of what went wrong, and the
            // window closes the tab of a shell marked exited. So its session is kept, with a
            // notice saying how it ended, until the user closes it themselves. A shell taken
            // out of the registry already was ended on purpose, and has nothing to explain.
            match failure_notice(&session) {
                Some(notice) if registry.is_live(&reaped_id) => {
                    session.child_ended.store(true, Ordering::Relaxed);
                    session.scrollback.lock().unwrap().push(notice.as_bytes());
                    let _ = output.send(notice.into_bytes());
                }
                _ => {
                    // `send` is refused when nothing is subscribed, and would leave the flag
                    // false for whoever asks later; the value has to be set whether or not
                    // anyone is listening.
                    exited.send_replace(true);
                    registry.remove(&reaped_id);
                }
            }
        });

        Ok(terminal_id)
    }
}

/// The notice a shell's ending earns, if it is one worth keeping on screen: an agent whose
/// process ended in failure. `None` says the shell is done with and can be reaped.
///
/// A plain login shell is never kept: it exits with whatever its last command returned, so a
/// nonzero status there is everyday use rather than the program falling over.
fn failure_notice(session: &TerminalSession) -> Option<String> {
    let program = match session.program.agent() {
        None => return None,
        Some(agent) => agent.label().to_lowercase(),
    };
    let status = session.child.lock().unwrap().wait().ok()?;
    if status.success() {
        return None;
    }

    let ending = match status.signal() {
        Some(signal) => format!("was ended by {signal}"),
        None => format!("exited with code {}", status.exit_code()),
    };
    Some(format!("\r\n\x1b[31m[{program} {ending}]\x1b[0m\r\n"))
}

/// Type text into a shell once the program in it has a box to take it, as if a person had.
///
/// The wait is what makes this work at all: keys written at an agent that has not drawn its
/// input box yet are dropped by it, so the text has to arrive after that and - this is the
/// point - before the person starts writing over it. So it waits for the program to stop
/// drawing rather than for a fixed span, and if the person got there first it types nothing:
/// a title landing in the middle of a sentence someone is writing is worse than no title.
fn type_ahead(session: &TerminalSession, text: &str) {
    let deadline = Instant::now() + TYPE_AHEAD_DEADLINE;
    while Instant::now() < deadline {
        if *session.exited.borrow()
            || session.child_ended.load(Ordering::Relaxed)
            || session.typed_into.load(Ordering::Relaxed)
        {
            return;
        }
        // Silence before the program has printed anything at all is it still starting up, not
        // a box waiting to be typed into.
        let quiet = session
            .last_output
            .lock()
            .unwrap()
            .is_some_and(|last| last.elapsed() >= TYPE_AHEAD_QUIET);
        if quiet {
            break;
        }
        std::thread::sleep(TYPE_AHEAD_POLL);
    }

    // Checked again holding the writer, which is the lock a keystroke takes: a person who has
    // typed by now is seen, and one who types from here on waits and lands after the whole of
    // the text rather than inside it. The shell may also have ended, and a dead pty refuses
    // writes.
    let mut writer = session.writer.lock().unwrap();
    if *session.exited.borrow()
        || session.child_ended.load(Ordering::Relaxed)
        || session.typed_into.load(Ordering::Relaxed)
    {
        return;
    }
    let _ = writer.write_all(text.as_bytes());
}

pub(crate) async fn create_terminal(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateTerminalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let repo_path =
        crate::api::with_session(&state, &session_id, |session| Ok(session.repo_path.clone()))?;
    let terminal_id = state
        .terminals
        .spawn(TerminalSpec::shell(repo_path, request.command))?;
    Ok(Json(TerminalCreated { terminal_id }))
}

pub(crate) async fn list_terminals(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    Ok(Json(TerminalList {
        terminal_ids: state.terminals.terminal_ids(),
    }))
}

pub(crate) async fn close_terminal(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    state.terminals.remove(&terminal_id);
    Ok(Json(TerminalList {
        terminal_ids: state.terminals.terminal_ids(),
    }))
}

pub(crate) async fn terminal_socket(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    let session = state
        .terminals
        .get(&terminal_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("unknown terminal {terminal_id}")))?;

    Ok(upgrade.on_upgrade(move |socket| async move {
        if let Err(error) = attach_terminal(socket, session).await {
            eprintln!("[moonreview] terminal attachment ended: {error}");
        }
    }))
}

async fn attach_terminal(socket: WebSocket, session: Arc<TerminalSession>) -> anyhow::Result<()> {
    // Subscribe before replaying so nothing written in between is lost.
    let mut output = session.output.subscribe();
    let mut exited = session.exited.subscribe();
    let replay = session.scrollback.lock().unwrap().replay();

    let (mut socket_sender, mut socket_receiver) = socket.split();
    if !replay.is_empty() {
        socket_sender.send(Message::Binary(replay.into())).await?;
    }

    let mut pump_output = tokio::spawn(async move {
        loop {
            match output.recv().await {
                Ok(chunk) => {
                    if socket_sender.send(Message::Binary(chunk.into())).await.is_err() {
                        return;
                    }
                }
                // Lagged: the browser fell behind, keep going with what follows.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        let _ = socket_sender.close().await;
    });

    loop {
        if *exited.borrow() {
            break;
        }

        tokio::select! {
            _ = &mut pump_output => break,
            // The shell exited: nothing more will come, so let the tab know.
            _ = exited.changed() => break,
            incoming = socket_receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                let Message::Text(text) = message else { continue };
                crate::api::mark_activity(&session.last_activity);
                match serde_json::from_str::<ClientMessage>(&text)? {
                    ClientMessage::Input { data } => {
                        session.typed_into.store(true, Ordering::Relaxed);
                        session.write_to_child(data.as_bytes())?;
                    }
                    ClientMessage::Reply { data } => {
                        session.write_to_child(data.as_bytes())?;
                    }
                    ClientMessage::Resize { cols, rows } => {
                        session.master.lock().unwrap().resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })?;
                    }
                }
            }
        }
    }

    pump_output.abort();
    Ok(())
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
