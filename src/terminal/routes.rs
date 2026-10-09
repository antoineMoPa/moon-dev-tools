//! The HTTP and websocket routes a window reaches the server's shells through.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Extension, Json,
    extract::{
        Path as AxumPath, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use portable_pty::PtySize;
use tokio::sync::broadcast;

use crate::{
    api::{
        AgentKind, AppError, AppState, TerminalAttentionList, TerminalNameRequest, TerminalView,
    },
    server::users::{UserId, Users},
};

use super::naming::{name_for_new_shell, rename};
use super::{
    ClientMessage, CreateTerminalRequest, RunInShellRequest, TerminalCreated, TerminalList,
    TerminalProgram, TerminalSession, TerminalSpec,
};

/// Start a shell of the workspace's own in the reviewed repo, and answer with which.
pub(crate) fn start_workspace_shell(
    state: &AppState,
    session_id: &str,
    command: Option<AgentKind>,
) -> anyhow::Result<String> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    spawn_workspace_shell(
        state,
        session_id,
        repo_path.clone(),
        repo_path,
        command,
        Vec::new(),
    )
}

/// A login shell started in a folder of the repo rather than at its root: what
/// `moon shell <folder>` typed in a terminal asks the window for. The folder has to be inside
/// the session's repo - a folder outside it belongs to another session, and the window opens
/// that one first.
pub(crate) fn start_workspace_shell_in_folder(
    state: &AppState,
    session_id: &str,
    folder: &Path,
) -> anyhow::Result<String> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    validate_workspace_folder(&repo_path, folder)?;
    spawn_workspace_shell(
        state,
        session_id,
        repo_path,
        folder.to_path_buf(),
        None,
        Vec::new(),
    )
}

fn validate_workspace_folder(repo: &Path, folder: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        folder.starts_with(repo),
        "{} is outside {}, so no shell of that repo starts there",
        folder.display(),
        repo.display()
    );
    anyhow::ensure!(folder.is_dir(), "{} is not a folder", folder.display());
    Ok(())
}

/// A shell of the workspace's own, named after its program and started in `cwd`.
fn spawn_workspace_shell(
    state: &AppState,
    session_id: &str,
    repo_path: PathBuf,
    cwd: PathBuf,
    command: Option<AgentKind>,
    env: Vec<(String, String)>,
) -> anyhow::Result<String> {
    let program = TerminalProgram::of_agent(command);
    // A workspace shell belongs to no task, so its name is the program and the number alone.
    let name = name_for_new_shell(state, &repo_path, None, &program)?;
    state.terminals.spawn(TerminalSpec {
        owner: workspace_owner(state, session_id)?,
        env,
        runs_as: crate::api::person_of(state, session_id)?,
        ..TerminalSpec::shell(cwd, command, Some(name))
    })
}

/// The same shell with one command line typed into it and sent: what an extension opens when
/// what it has to show is a program of its own, like a container's logs followed as they come.
///
/// The shell outlives the command, the way a project's build does - see
/// [`TerminalSpec::running`].
pub(crate) fn start_workspace_shell_running(
    state: &AppState,
    session_id: &str,
    command: &str,
) -> anyhow::Result<String> {
    start_workspace_shell_running_with_environment(state, session_id, command, Vec::new())
}

fn start_workspace_shell_running_with_environment(
    state: &AppState,
    session_id: &str,
    command: &str,
    env: Vec<(String, String)>,
) -> anyhow::Result<String> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    let name = name_for_new_shell(state, &repo_path, None, &TerminalProgram::LoginShell)?;
    state.terminals.spawn(TerminalSpec {
        owner: workspace_owner(state, session_id)?,
        name: Some(name),
        env,
        runs_as: crate::api::person_of(state, session_id)?,
        ..TerminalSpec::running(repo_path, command)
    })
}

fn workspace_owner(state: &AppState, session_id: &str) -> anyhow::Result<Option<String>> {
    crate::api::with_session(state, session_id, |session| {
        Ok(session
            .namespace
            .as_ref()
            .map(|namespace| format!("workspace:{namespace}")))
    })
}

/// Local sessions retain the host's shell settings; personal web sessions get only
/// their own commit identity. Shared task shells never pass through these routes.
fn workspace_git_environment(
    state: &AppState,
    session_id: &str,
    profiles: &crate::server::profiles::Profiles,
    user: &UserId,
) -> anyhow::Result<Vec<(String, String)>> {
    crate::api::with_session(state, session_id, |session| {
        match session.namespace.as_deref() {
            None => Ok(Vec::new()),
            Some(namespace) => {
                anyhow::ensure!(
                    namespace == profiles.namespace(user),
                    "Account changed; reload this window"
                );
                Ok(profiles.shell_git_environment(user))
            }
        }
    })
}

fn workspace_terminal_ids(state: &AppState, session_id: &str) -> anyhow::Result<Vec<String>> {
    let owner = workspace_owner(state, session_id)?;
    Ok(state.terminals.terminal_ids_for_owner(owner.as_deref()))
}

pub(crate) async fn create_terminal(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    State(profiles): State<crate::server::profiles::Profiles>,
    Extension(user): Extension<UserId>,
    Json(request): Json<CreateTerminalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let env = workspace_git_environment(&state, &session_id, &profiles, &user)?;
    let repo_path =
        crate::api::with_session(&state, &session_id, |session| Ok(session.repo_path.clone()))?;
    let command = if request.folder.is_some() {
        None
    } else {
        request.command
    };
    let cwd = request
        .folder
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_path.clone());
    validate_workspace_folder(&repo_path, &cwd)?;
    let terminal_id = spawn_workspace_shell(&state, &session_id, repo_path, cwd, command, env)?;
    Ok(Json(TerminalCreated { terminal_id }))
}

pub(crate) async fn run_in_shell(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    State(profiles): State<crate::server::profiles::Profiles>,
    Extension(user): Extension<UserId>,
    Json(request): Json<RunInShellRequest>,
) -> Result<impl IntoResponse, AppError> {
    let env = workspace_git_environment(&state, &session_id, &profiles, &user)?;
    let terminal_id =
        start_workspace_shell_running_with_environment(&state, &session_id, &request.command, env)?;
    Ok(Json(TerminalCreated { terminal_id }))
}

pub(crate) async fn list_terminals(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    Ok(Json(TerminalList {
        terminal_ids: workspace_terminal_ids(&state, &session_id)?,
    }))
}

/// The shells with something running in them, which is what a quit would interrupt.
pub(crate) async fn terminals_running_a_command(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    // Waits for the session lock and asks the ptys, both on a clock: kept off the async workers.
    let terminal_ids = tokio::task::spawn_blocking(move || {
        crate::api::with_session(&state, &session_id, |_| Ok(()))?;
        let owner = workspace_owner(&state, &session_id)?;
        anyhow::Ok(
            state
                .terminals
                .terminals_running_a_command()
                .into_iter()
                .filter(|id| state.terminals.activity_visible_to(id, owner.as_deref()))
                .collect(),
        )
    })
    .await??;
    Ok(Json(TerminalList { terminal_ids }))
}

/// The shells asking for a person - see [`crate::attention`].
pub(crate) async fn terminals_wanting_attention(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let terminals = tokio::task::spawn_blocking(move || {
        crate::api::with_session(&state, &session_id, |_| Ok(()))?;
        let owner = workspace_owner(&state, &session_id)?;
        anyhow::Ok(
            state
                .terminals
                .wanting_attention()
                .into_iter()
                .filter(|terminal| {
                    state
                        .terminals
                        .activity_visible_to(&terminal.terminal_id, owner.as_deref())
                })
                .collect(),
        )
    })
    .await??;
    Ok(Json(TerminalAttentionList { terminals }))
}

pub(crate) async fn close_terminal(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    state.terminals.remove(&terminal_id);
    Ok(Json(TerminalList {
        terminal_ids: workspace_terminal_ids(&state, &session_id)?,
    }))
}

pub(crate) async fn terminal_view(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    if !state.terminals.is_live(&terminal_id) {
        return Err(AppError(anyhow::anyhow!("unknown terminal {terminal_id}")));
    }
    Ok(Json(TerminalView {
        name: state.terminals.name(&terminal_id),
        terminal_id,
    }))
}

pub(crate) async fn rename_terminal(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TerminalNameRequest>,
) -> Result<impl IntoResponse, AppError> {
    // The name is written on the run's card, as whoever renamed it.
    crate::server::session_work(state, session_id, move |state, session_id| {
        rename(state, session_id, &terminal_id, &request.name)
    })
    .await?;
    Ok("ok")
}

/// The socket is admitted once, as it opens - see `crate::server::auth` - so it is told who
/// opened it and hangs up when that user is kicked - see [`Users::kicks`].
pub(crate) async fn terminal_socket(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    State(users): State<Users>,
    Extension(user): Extension<UserId>,
    upgrade: WebSocketUpgrade,
    Query(stream): Query<TerminalStreamQuery>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    let session = state
        .terminals
        .get(&terminal_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("unknown terminal {terminal_id}")))?;

    Ok(upgrade.on_upgrade(move |socket| async move {
        if let Err(error) = attach_terminal(socket, session, users, user, stream).await {
            eprintln!("[moonreview] terminal attachment ended: {error}");
        }
    }))
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct TerminalStreamQuery {
    #[serde(default)]
    moon_terminal_stream: bool,
    moon_terminal_cursor: Option<u64>,
}

async fn attach_terminal(
    socket: WebSocket,
    session: Arc<TerminalSession>,
    users: Users,
    user: UserId,
    stream: TerminalStreamQuery,
) -> anyhow::Result<()> {
    // Subscribe before replaying so nothing written in between is lost.
    let mut output = session.output.subscribe();
    let mut exited = session.exited.subscribe();
    let mut kicks = users.kicks();
    let replay = session
        .scrollback
        .lock()
        .unwrap()
        .attachment(if stream.moon_terminal_stream {
            stream.moon_terminal_cursor
        } else {
            None
        });
    let (mut socket_sender, mut socket_receiver) = socket.split();
    let replay = match replay {
        Ok(replay) => replay,
        Err(error) => {
            socket_sender.send(stream_error(&error)).await?;
            return Ok(());
        }
    };
    let mut cursor = replay.sequence;
    if stream.moon_terminal_stream || !replay.bytes.is_empty() {
        socket_sender
            .send(output_message(&replay, stream.moon_terminal_stream))
            .await?;
    }
    let output_session = Arc::clone(&session);
    let mut pump_output = tokio::spawn(async move {
        loop {
            let chunk = match output.recv().await {
                Ok(chunk) => chunk,
                Err(broadcast::error::RecvError::Lagged(_)) if stream.moon_terminal_stream => {
                    let resumed = output_session
                        .scrollback
                        .lock()
                        .unwrap()
                        .attachment(Some(cursor));
                    match resumed {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            let _ = socket_sender.send(stream_error(&error)).await;
                            return;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            // A chunk pushed while replay was captured is already in that replay. This
            // also skips queued broadcasts following recovery from a lagged receiver.
            if chunk.sequence <= cursor {
                continue;
            }
            cursor = chunk.sequence;
            if socket_sender
                .send(output_message(&chunk, stream.moon_terminal_stream))
                .await
                .is_err()
            {
                return;
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
            // Someone was kicked - this user, or everyone: the socket closes rather than
            // going on past its admission.
            kicked = kicks.changed() => {
                if kicked.is_err() || users.is_kicked(&user) {
                    break;
                }
            }
            incoming = socket_receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                let Message::Text(text) = message else { continue };
                crate::api::mark_activity(&session.last_activity);
                match serde_json::from_str::<ClientMessage>(&text)? {
                    ClientMessage::Input { data } => {
                        session.typed_into();
                        session.write_to_child(data.as_bytes())?;
                    }
                    ClientMessage::Reply { data } => {
                        session.write_to_child(data.as_bytes())?;
                    }
                    ClientMessage::Report { data } => {
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

fn output_message(chunk: &super::output::OutputChunk, framed: bool) -> Message {
    Message::Binary(
        if framed {
            crate::api::terminal_stream::encode(chunk.sequence, &chunk.bytes)
        } else {
            chunk.bytes.clone()
        }
        .into(),
    )
}
fn stream_error(error: &anyhow::Error) -> Message {
    Message::Text(
        serde_json::json!({"terminal_error": error.to_string()})
            .to_string()
            .into(),
    )
}
