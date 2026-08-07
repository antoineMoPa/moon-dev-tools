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
/// How long [`TerminalSpec::type_ahead`] waits before typing.
///
/// An agent's input box only takes keys once its interface has been drawn, and nothing it
/// prints says when that is. Measured on this machine: Claude Code's box takes them by 2s,
/// and OpenCode does not paint at all until 2.3s. Three seconds clears both, and being late
/// costs nothing — the text lands in a box that has been waiting.
const TYPE_AHEAD_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
/// How much shell output we keep so a reopened tab can replay what it missed.
const SCROLLBACK_LIMIT: usize = 256 * 1024;
const BROADCAST_CAPACITY: usize = 256;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Input { data: String },
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

/// What to start a shell as. The plain case is a login shell in the reviewed repo; a task's
/// agent adds the program, its arguments, and the environment that tells it which task it is
/// working in.
pub(crate) struct TerminalSpec {
    pub(crate) cwd: std::path::PathBuf,
    /// The program to run. `None` is the login shell.
    pub(crate) program: Option<AgentKind>,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    /// The task this shell belongs to, if any. An owned shell is the task's to list and to
    /// close, so it stays out of the workspace's own shells.
    pub(crate) owner: Option<String>,
    /// Text typed into the shell once the program has come up, as if the user had typed it.
    ///
    /// Never ends in a newline, and nothing here sends one: what this does is leave something
    /// written in an agent's box for the person to send. A write that lands too early — while
    /// the agent is still asking whether it trusts the folder — is lost rather than acted on,
    /// which is what makes typing at a program that has not said it is ready acceptable.
    pub(crate) type_ahead: Option<String>,
}

impl TerminalSpec {
    /// A shell of the workspace's own: no program, no task.
    pub(crate) fn shell(cwd: std::path::PathBuf, program: Option<AgentKind>) -> Self {
        Self {
            cwd,
            program,
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            type_ahead: None,
        }
    }
}

/// A shell that lives in the server, not in the browser tab: closing the tab detaches
/// the websocket while the pty keeps running, so reopening it resumes the same shell.
pub(crate) struct TerminalSession {
    owner: Option<String>,
    /// What it was started as: `None` for the login shell, an agent otherwise. A task's plain
    /// shells are listed from here, because they are not written down anywhere else.
    program: Option<AgentKind>,
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
    exited: watch::Sender<bool>,
    scrollback: Mutex<Scrollback>,
    /// Typing in a shell, and a shell printing output, both keep the server from idling out.
    last_activity: Arc<Mutex<Instant>>,
}

impl TerminalSession {
    /// Whether the shell has ended. Set by the reader thread when the pty reaches EOF.
    #[cfg(feature = "native")]
    pub(crate) fn has_exited(&self) -> bool {
        *self.exited.borrow()
    }

    // The native window drives a pty directly; a web tab goes through the websocket.
    #[cfg(feature = "native")]
    pub(crate) fn write_input(&self, data: &[u8]) -> anyhow::Result<()> {
        crate::api::mark_activity(&self.last_activity);
        self.writer.lock().unwrap().write_all(data)?;
        Ok(())
    }

    #[cfg(feature = "native")]
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
    #[cfg(feature = "native")]
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

    /// The shells the workspace has of its own — a task's shells are the task's to show.
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
    /// agent run from one that ended — or that died with a previous run of the server.
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
                    && matches!(session.program, None | Some(AgentKind::None))
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

        let mut command = match spec.program {
            None | Some(AgentKind::None) => {
                let mut command = CommandBuilder::new(crate::shell_path::login_shell());
                command.arg("-l");
                command
            }
            Some(AgentKind::Claude) => CommandBuilder::new("claude"),
            Some(AgentKind::Codex) => CommandBuilder::new("codex"),
            Some(AgentKind::OpenCode) => CommandBuilder::new("opencode"),
        };
        for argument in &spec.args {
            command.arg(argument);
        }
        command.cwd(&spec.cwd);
        command.env("TERM", "xterm-256color");
        // The agent is started by name, so it has to be looked up on the PATH the user's shell
        // has rather than the one a desktop launcher hands this process.
        command.env("PATH", crate::shell_path::agent_path());
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
            program: spec.program,
            started_at_unix: crate::moontasks::store::now_unix(),
            order,
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
            child: Mutex::new(child),
            output: output.clone(),
            exited: exited.clone(),
            scrollback: Mutex::new(Scrollback::default()),
            last_activity: Arc::clone(&self.last_activity),
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(terminal_id.clone(), Arc::clone(&session));

        // Typed from a thread of its own, so starting a shell answers straight away rather
        // than after the wait. It goes through the same writer a keystroke does, so a person
        // typing at the same time interleaves with it instead of racing it.
        if let Some(text) = spec.type_ahead {
            let typing_into = Arc::clone(&session);
            std::thread::spawn(move || {
                std::thread::sleep(TYPE_AHEAD_DELAY);
                // The shell may have ended in the meantime, and a dead pty refuses writes.
                if !*typing_into.exited.borrow() {
                    let _ = typing_into.writer.lock().unwrap().write_all(text.as_bytes());
                }
            });
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
                        session.scrollback.lock().unwrap().push(chunk);
                        // No attached tab is normal: the shell keeps running regardless.
                        let _ = output.send(chunk.to_vec());
                    }
                }
            }
            // `send` is refused when nothing is subscribed, and would leave the flag false for
            // whoever asks later; the value has to be set whether or not anyone is listening.
            exited.send_replace(true);
            registry.remove(&reaped_id);
        });

        Ok(terminal_id)
    }
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
                        session.writer.lock().unwrap().write_all(data.as_bytes())?;
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
mod tests {
    use std::time::Duration;

    use super::*;

    /// Type-ahead reaches the program as keystrokes, and reaches it whole.
    ///
    /// A login shell stands in for an agent here: it echoes what is typed at it, which is the
    /// same evidence an agent's input box gives — and unlike an agent it is on every machine
    /// this runs on. What it must not do is run anything, so nothing here sends a newline.
    #[test]
    fn type_ahead_is_typed_into_the_shell_and_left_unsent() {
        let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
        let terminal_id = registry
            .spawn(TerminalSpec {
                cwd: std::env::temp_dir(),
                program: None,
                args: Vec::new(),
                env: Vec::new(),
                owner: None,
                type_ahead: Some("moonreview-typed-this".to_string()),
            })
            .expect("expected a shell");
        let session = registry.get(&terminal_id).expect("expected the shell");

        // The wait before it is typed is the point of it, so this waits out that wait.
        let deadline = Instant::now() + TYPE_AHEAD_DELAY + Duration::from_secs(7);
        let mut printed = String::new();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            // The pty is 80 columns wide and a prompt takes some of them, so what was typed
            // comes back wrapped: the line break the shell inserted is dropped rather than
            // matched against.
            printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay())
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            if printed.contains("moonreview-typed-this") {
                break;
            }
        }
        registry.remove(&terminal_id);

        assert!(
            printed.contains("moonreview-typed-this"),
            "the shell never echoed what was typed at it, printed: {printed:?}"
        );
        // Left at the prompt, not run: a shell that had been sent the line would have gone
        // looking for a command by that name.
        assert!(
            !printed.contains("notfound"),
            "type-ahead must not be sent, printed: {printed:?}"
        );
    }
}
