//! The HTTP surface a remote window reviews through. Every route is a thin wrapper over
//! [`crate::service`], which a window on this machine calls directly.

mod board_routes;
mod review_routes;

use board_routes::{
    add_column, attach_task_resource, create_task, delete_column, delete_task,
    delete_task_resource, explain_task_changes, link_task_file, list_agent_sessions, list_columns,
    list_tasks, open_task_notes, open_work_log, place_column, place_tasks, project_commands,
    rename_column, rename_task, resume_task_resource, run_project_command, set_column_arrivals,
    set_column_sort, set_project_config, set_task_tags, start_task_resource, stop_task_resource,
};
use review_routes::{
    agent_dispatch_log_request, blame_session_file, cancel_comment_dispatch_request,
    commit_history, commit_run_outcome, commit_state, create_session_file, discard_hunk,
    discard_hunks, find_session_files, hunk_patch, open_session, resolve_comment,
    resolve_comment_by_key, search_session_contents, send_comment_batch, session_file,
    session_file_at, session_state, session_submodules, stage_all, stage_file, stage_hunk,
    stage_selection, start_commit_run, suggest_commit_message, unstage_file, unstage_hunk,
    update_agent, update_comment, update_commit_view, write_session_file,
};

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
};

use crate::{
    agent::detect_agent_availability,
    api::{AppState, ServerState, bind_host, port, server_url},
};

const SERVER_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// The state the window and the server share. The app builds this once and hands a clone to
/// the server it carries, so a remote window reviews the same sessions this one does.
pub(crate) fn build_state(last_activity: Arc<Mutex<Instant>>) -> AppState {
    AppState {
        inner: Arc::new(Mutex::new(ServerState::default())),
        agent_availability: detect_agent_availability(),
        last_activity: Arc::clone(&last_activity),
        terminals: Arc::new(crate::terminal::TerminalRegistry::new(last_activity)),
        // The servers are told they are talking to this application rather than to the
        // client crate they are reached through: `clientInfo` is what a server writes into
        // its log, and a report about rust-analyzer under a review is only findable if the
        // log says which program was asking.
        lsp: Arc::new(
            moon_lsp::LspRegistry::new(crate::shell_path::installed_tools_path().to_string())
                .identifying_as(moon_lsp::ClientIdentity::new(
                    "moonreview",
                    env!("CARGO_PKG_VERSION"),
                )),
        ),
    }
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route(
            "/api/session/{session_id}/resolve/{hunk_id}/{comment_index}",
            get(resolve_comment),
        )
        .route(
            "/api/session/{session_id}/resolve-key/{hunk_id}/{comment_key}",
            get(resolve_comment_by_key),
        )
        .route("/api/session/open", post(open_session))
        .route("/api/session/{session_id}/state", get(session_state))
        .route(
            "/api/session/{session_id}/submodules",
            get(session_submodules),
        )
        .route("/api/session/{session_id}/history", get(commit_history))
        .route("/api/session/{session_id}/agent", post(update_agent))
        .route("/api/session/{session_id}/commit", post(update_commit_view))
        .route("/api/session/{session_id}/hunk/{hunk_id}", get(hunk_patch))
        .route(
            "/api/session/{session_id}/file",
            get(session_file).post(write_session_file),
        )
        .route(
            "/api/session/{session_id}/file/new",
            post(create_session_file),
        )
        .route("/api/session/{session_id}/blame", post(blame_session_file))
        .route("/api/session/{session_id}/file-at", get(session_file_at))
        .route("/api/session/{session_id}/files", get(find_session_files))
        .route(
            "/api/session/{session_id}/content",
            get(search_session_contents),
        )
        .route("/api/session/{session_id}/comment", post(update_comment))
        .route(
            "/api/session/{session_id}/comment-batch",
            post(send_comment_batch),
        )
        .route(
            "/api/session/{session_id}/comment-dispatch/cancel",
            post(cancel_comment_dispatch_request),
        )
        .route(
            "/api/session/{session_id}/agent-dispatch/log",
            get(agent_dispatch_log_request),
        )
        .route("/api/session/{session_id}/stage", post(stage_hunk))
        .route("/api/session/{session_id}/stage-file", post(stage_file))
        .route(
            "/api/session/{session_id}/stage-selection",
            post(stage_selection),
        )
        .route("/api/session/{session_id}/discard", post(discard_hunk))
        .route(
            "/api/session/{session_id}/discard-batch",
            post(discard_hunks),
        )
        .route("/api/session/{session_id}/unstage", post(unstage_hunk))
        .route("/api/session/{session_id}/unstage-file", post(unstage_file))
        .route(
            "/api/session/{session_id}/tasks",
            get(list_tasks).post(create_task),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}",
            delete(delete_task),
        )
        .route(
            "/api/session/{session_id}/tasks/placement",
            post(place_tasks),
        )
        .route(
            "/api/session/{session_id}/columns",
            get(list_columns).post(add_column),
        )
        .route(
            "/api/session/{session_id}/project",
            get(project_commands).post(set_project_config),
        )
        .route(
            "/api/session/{session_id}/project/run/{which}",
            post(run_project_command),
        )
        .route(
            "/api/session/{session_id}/run-in-shell",
            post(crate::terminal::run_in_shell),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}",
            delete(delete_column),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/title",
            post(rename_column),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/arrivals",
            post(set_column_arrivals),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/sort",
            post(set_column_sort),
        )
        .route(
            "/api/session/{session_id}/columns/{column_id}/placement",
            post(place_column),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources",
            post(start_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/explanation",
            post(explain_task_changes),
        )
        .route(
            "/api/session/{session_id}/agent-sessions",
            get(list_agent_sessions),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/attach",
            post(attach_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/resume",
            post(resume_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/stop",
            post(stop_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}",
            delete(delete_task_resource),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/title",
            post(rename_task),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/tags",
            post(set_task_tags),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/notes/open",
            post(open_task_notes),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/files",
            post(link_task_file),
        )
        .route(
            "/api/session/{session_id}/work-log/open",
            post(open_work_log),
        )
        .route("/api/session/{session_id}/commit-state", get(commit_state))
        .route("/api/session/{session_id}/stage-all", post(stage_all))
        .route(
            "/api/session/{session_id}/commit-message",
            post(suggest_commit_message),
        )
        .route(
            "/api/session/{session_id}/commit-run",
            post(start_commit_run),
        )
        .route(
            "/api/session/{session_id}/commit-run/{terminal_id}/outcome",
            get(commit_run_outcome),
        )
        .route(
            "/api/session/{session_id}/lsp/status",
            get(crate::lsp::routes::status),
        )
        .route(
            "/api/session/{session_id}/lsp/working",
            get(crate::lsp::routes::working),
        )
        .route(
            "/api/session/{session_id}/lsp/triggers",
            get(crate::lsp::routes::triggers),
        )
        .route(
            "/api/session/{session_id}/lsp/open",
            post(crate::lsp::routes::did_open),
        )
        .route(
            "/api/session/{session_id}/lsp/change",
            post(crate::lsp::routes::did_change),
        )
        .route(
            "/api/session/{session_id}/lsp/close",
            post(crate::lsp::routes::did_close),
        )
        .route(
            "/api/session/{session_id}/lsp/places",
            post(crate::lsp::routes::places),
        )
        .route(
            "/api/session/{session_id}/lsp/completion",
            post(crate::lsp::routes::completion),
        )
        .route(
            "/api/session/{session_id}/lsp/prepare-rename",
            post(crate::lsp::routes::prepare_rename),
        )
        .route(
            "/api/session/{session_id}/lsp/rename",
            post(crate::lsp::routes::rename),
        )
        .route(
            "/api/session/{session_id}/lsp/format",
            post(crate::lsp::routes::format),
        )
        .route(
            "/api/session/{session_id}/lsp/hover",
            post(crate::lsp::routes::hover),
        )
        .route(
            "/api/session/{session_id}/lsp/diagnostics",
            get(crate::lsp::routes::diagnostics),
        )
        .route(
            "/api/session/{session_id}/lsp/save",
            post(crate::lsp::routes::did_save),
        )
        .route(
            "/api/session/{session_id}/lsp/code-actions",
            post(crate::lsp::routes::code_actions),
        )
        .route(
            "/api/session/{session_id}/lsp/signature",
            post(crate::lsp::routes::signature_help),
        )
        .route(
            "/api/session/{session_id}/terminals",
            get(crate::terminal::list_terminals).post(crate::terminal::create_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/running",
            get(crate::terminal::terminals_running_a_command),
        )
        .route(
            "/api/session/{session_id}/terminals/attention",
            get(crate::terminal::terminals_wanting_attention),
        )
        .route(
            "/api/session/{session_id}/terminals/visualizations",
            get(crate::visualizations::routes::announced),
        )
        .route(
            "/api/session/{session_id}/visualizations/page",
            get(crate::visualizations::routes::page),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}",
            get(crate::terminal::terminal_view).delete(crate::terminal::close_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}/name",
            post(crate::terminal::rename_terminal),
        )
        .route(
            "/api/session/{session_id}/terminals/{terminal_id}/socket",
            get(crate::terminal::terminal_socket),
        )
        .with_state(state)
}

pub(crate) async fn run_server() -> Result<()> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let state = build_state(Arc::clone(&last_activity));
    serve(state, Some(last_activity)).await
}

/// Serve the review API. `idle_shutdown` is the clock the standalone server stops on;
/// a window passes `None` because it decides when the process ends itself.
pub(crate) async fn serve(
    state: AppState,
    idle_shutdown: Option<Arc<Mutex<Instant>>>,
) -> Result<()> {
    let port = port()?;
    let host = bind_host();
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .with_context(|| format!("failed to bind {host}:{port}"))?;

    println!("Moon Review listening on {}", server_url());
    serve_on(state, listener, idle_shutdown).await
}

/// Serve on a listener the caller already bound, which is how a test gets a free port.
pub(crate) async fn serve_on(
    state: AppState,
    listener: tokio::net::TcpListener,
    idle_shutdown: Option<Arc<Mutex<Instant>>>,
) -> Result<()> {
    match idle_shutdown {
        Some(last_activity) => axum::serve(listener, router(state))
            .with_graceful_shutdown(shutdown_signal(last_activity))
            .await
            .context("server failed"),
        None => axum::serve(listener, router(state))
            .await
            .context("server failed"),
    }
}

async fn shutdown_signal(last_activity: Arc<Mutex<Instant>>) {
    loop {
        let idle_for = last_activity
            .lock()
            .map(|value| value.elapsed())
            .unwrap_or(SERVER_LIFETIME);
        let remaining = SERVER_LIFETIME.saturating_sub(idle_for);
        let timeout = tokio::time::sleep(remaining);
        tokio::pin!(timeout);

        tokio::select! {
            _ = &mut timeout => {
                let idle_for = last_activity
                    .lock()
                    .map(|value| value.elapsed())
                    .unwrap_or(SERVER_LIFETIME);
                if idle_for >= SERVER_LIFETIME {
                    eprintln!(
                        "[moonreview] shutting down after {} minutes of inactivity",
                        SERVER_LIFETIME.as_secs() / 60
                    );
                    return;
                }
            }
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    eprintln!("[moonreview] failed to listen for shutdown signal: {error}");
                }
                return;
            }
        }
    }
}

fn mark_activity(state: &AppState) {
    crate::api::mark_activity(&state.last_activity);
}

async fn root(State(state): State<AppState>) -> impl IntoResponse {
    mark_activity(&state);
    Html(
        "<!doctype html><title>Moon Review</title><p>A review server. Point a window at it with `moonreview --remote`.</p>",
    )
}

async fn healthz(State(state): State<AppState>) -> &'static str {
    mark_activity(&state);
    "ok"
}
