//! Desktop routes - the HTTP and websocket routes a window uses to reach the server's desktop.

#[cfg(target_os = "linux")]
use axum::response::IntoResponse;
use axum::{
    Extension, Json,
    extract::{Path as AxumPath, State, WebSocketUpgrade},
};

use crate::{
    api::{
        AppError, AppState,
        display::{DisplayView, StartApplicationRequest},
    },
    server::users::{UserId, Users},
};

/// `POST /api/session/{session_id}/display/applications`: an application started on the
/// server's desktop, in the session's repo - and the desktop started for it, when there was
/// none.
pub(crate) async fn start_application(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(asked): Json<StartApplicationRequest>,
) -> Result<Json<DisplayView>, AppError> {
    let folder =
        crate::api::with_session(&state, &session_id, |session| Ok(session.repo_path.clone()))?;
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(state.display.start_application(
        &asked.command,
        &folder,
        [asked.width, asked.height],
        asked.scale,
    )?))
}

/// `GET /api/display`: the server's desktop, when one is running - which is how a window
/// that did not start it knows there is one to show.
pub(crate) async fn shown(State(state): State<AppState>) -> Json<Option<DisplayView>> {
    Json(state.display.shown())
}

/// `DELETE /api/display`: the desktop ended, and every application on it.
pub(crate) async fn end(State(state): State<AppState>) -> &'static str {
    state.display.end();
    "ok"
}

/// `GET /api/display/socket`: what is drawn on the desktop, down; what the person watching
/// does, up. Admitted once, as it opens, and hung up when its user is kicked - as a shell's
/// socket is, see `crate::terminal::routes`.
#[cfg(target_os = "linux")]
pub(crate) async fn socket(
    State(state): State<AppState>,
    State(users): State<Users>,
    Extension(user): Extension<UserId>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let running = state.display.get().ok_or_else(nothing_running)?;
    let last_activity = std::sync::Arc::clone(&state.last_activity);
    Ok(upgrade.on_upgrade(move |socket| async move {
        if let Err(error) = watching::watch(socket, running, users, user, last_activity).await {
            eprintln!("[moonreview] the display's socket ended: {error}");
        }
    }))
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn socket(
    State(_state): State<AppState>,
    State(_users): State<Users>,
    Extension(_user): Extension<UserId>,
    _upgrade: WebSocketUpgrade,
) -> Result<axum::response::Response, AppError> {
    Err(nothing_running())
}

fn nothing_running() -> AppError {
    AppError(anyhow::anyhow!(
        "no application is running on the server's desktop"
    ))
}

#[cfg(target_os = "linux")]
mod watching {
    use std::{
        sync::{Arc, Mutex},
        time::Instant,
    };

    use axum::extract::ws::{Message, WebSocket};
    use futures::{SinkExt, StreamExt};
    use tokio::sync::broadcast;

    use crate::{
        api::display::DisplayInput,
        display::running::Running,
        server::users::{UserId, Users},
    };

    pub(super) async fn watch(
        socket: WebSocket,
        running: Arc<Running>,
        users: Users,
        user: UserId,
        last_activity: Arc<Mutex<Instant>>,
    ) -> anyhow::Result<()> {
        let (shown, mut patches) = running.watch();
        let mut over = running.over();
        let mut kicks = users.kicks();

        let (mut down, mut up) = socket.split();
        if let Some(shown) = shown {
            down.send(Message::Binary(shown.into())).await?;
        }
        let watched = Arc::clone(&running);
        let mut pump = tokio::spawn(async move {
            loop {
                let message = match patches.recv().await {
                    Ok(patch) => patch.as_ref().clone(),
                    // The socket is slower than the display is drawn on. The patches it
                    // missed are not sent late: it is shown the whole display as it is now,
                    // and goes on from there - so a slow line sees fewer frames, each of them
                    // current, rather than every frame a while after it was drawn.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let (shown, from_now) = watched.watch();
                        patches = from_now;
                        match shown {
                            Some(shown) => shown,
                            None => continue,
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if down.send(Message::Binary(message.into())).await.is_err() {
                    return;
                }
            }
            let _ = down.close().await;
        });

        loop {
            if *over.borrow() {
                break;
            }
            tokio::select! {
                _ = &mut pump => break,
                _ = over.changed() => break,
                kicked = kicks.changed() => {
                    if kicked.is_err() || users.is_kicked(&user) {
                        break;
                    }
                }
                incoming = up.next() => {
                    let Some(Ok(message)) = incoming else { break };
                    let Message::Text(text) = message else { continue };
                    crate::api::mark_activity(&last_activity);
                    running.did(serde_json::from_str::<DisplayInput>(&text)?);
                }
            }
        }

        pump.abort();
        Ok(())
    }
}
