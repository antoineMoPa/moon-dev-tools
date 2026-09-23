//! The HTTP and websocket routes a window reaches the server's shells through.

use std::sync::Arc;

use axum::{
    Json,
    extract::{
        Path as AxumPath, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use portable_pty::PtySize;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::api::{
    AgentKind, AppError, AppState, TerminalAttentionView, TerminalNameRequest, TerminalView,
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
    let program = TerminalProgram::of_agent(command);
    // A workspace shell belongs to no task, so its name is the program and the number alone.
    let name = name_for_new_shell(state, &repo_path, None, &program)?;
    state
        .terminals
        .spawn(TerminalSpec::shell(repo_path, command, Some(name)))
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
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    let name = name_for_new_shell(state, &repo_path, None, &TerminalProgram::LoginShell)?;
    state.terminals.spawn(TerminalSpec {
        name: Some(name),
        ..TerminalSpec::running(repo_path, command)
    })
}

pub(crate) async fn create_terminal(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateTerminalRequest>,
) -> Result<impl IntoResponse, AppError> {
    let terminal_id = start_workspace_shell(&state, &session_id, request.command)?;
    Ok(Json(TerminalCreated { terminal_id }))
}

pub(crate) async fn run_in_shell(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<RunInShellRequest>,
) -> Result<impl IntoResponse, AppError> {
    let terminal_id = start_workspace_shell_running(&state, &session_id, &request.command)?;
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

/// The shells with something running in them, which is what a quit would interrupt.
pub(crate) async fn terminals_running_a_command(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    Ok(Json(TerminalList {
        terminal_ids: state.terminals.terminals_running_a_command(),
    }))
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalAttentionList {
    pub(crate) terminals: Vec<TerminalAttentionView>,
}

/// The shells asking for a person - see [`crate::attention`].
pub(crate) async fn terminals_wanting_attention(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    crate::api::with_session(&state, &session_id, |_| Ok(()))?;
    Ok(Json(TerminalAttentionList {
        terminals: state.terminals.wanting_attention(),
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
    rename(&state, &session_id, &terminal_id, &request.name)?;
    Ok("ok")
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
                    if socket_sender
                        .send(Message::Binary(chunk.into()))
                        .await
                        .is_err()
                    {
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
