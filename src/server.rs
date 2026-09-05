//! The HTTP surface a remote window reviews through. Every route is a thin wrapper over
//! [`crate::service`], which a window on this machine calls directly.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    response::{Html, IntoResponse},
    routing::{delete, get, post},
};

use crate::{
    agent::detect_agent_availability,
    api::{
        AgentLogPayload, AgentLogQuery, AppError, AppState, CommitHistoryPayload,
        CommitHistoryQuery, CommitSelectionRequest, ContentMatchesPayload, FileContentPayload,
        FileMatchesPayload, FileQuery, FileSearchQuery, OpenSessionRequest, PatchPayload,
        SelectionRequest, ServerState, SessionOpened, SessionPayload, SubmoduleHubPayload,
        bind_host, port, server_url,
    },
    moontasks::{
        self, AttachResourceRequest, ColumnLabelRequest, ColumnPlacementRequest, CreateTaskRequest,
        LinkFileRequest, StartResourceRequest, TaskNotesPayload, TaskPlacementRequest,
        TaskTitleRequest, TaskView, TerminalOpened,
        store::{BoardColumn, ColumnId},
    },
    service,
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
        lsp: Arc::new(moon_lsp::LspRegistry::new(
            crate::shell_path::installed_tools_path().to_string(),
        )),
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
            get(project_commands).post(set_project_commands),
        )
        .route(
            "/api/session/{session_id}/project/run/{which}",
            post(run_project_command),
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
            "/api/session/{session_id}/columns/{column_id}/placement",
            post(place_column),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/resources",
            post(start_task_resource),
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
            "/api/session/{session_id}/tasks/{task_id}/notes/open",
            post(open_task_notes),
        )
        .route(
            "/api/session/{session_id}/tasks/{task_id}/files",
            post(link_task_file),
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
            "/api/session/{session_id}/lsp/definition",
            post(crate::lsp::routes::definition),
        )
        .route(
            "/api/session/{session_id}/lsp/completion",
            post(crate::lsp::routes::completion),
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

async fn session_submodules(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SubmoduleHubPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::session_submodules(&state, &session_id)?))
}

async fn open_session(
    State(state): State<AppState>,
    Json(request): Json<OpenSessionRequest>,
) -> Result<Json<SessionOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(service::open_session(&state, request)?))
}

async fn session_state(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::session_state(&state, &session_id)?))
}

async fn commit_history(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<CommitHistoryQuery>,
    State(state): State<AppState>,
) -> Result<Json<CommitHistoryPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::commit_history(
        &state,
        &session_id,
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(service::HISTORY_COMMIT_PAGE_SIZE),
    )?))
}

async fn update_agent(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::AgentSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::update_agent(&state, &session_id, request.agent)?;
    Ok("ok")
}

async fn update_commit_view(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CommitSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::update_commit_view(&state, &session_id, request.commit)?;
    Ok("ok")
}

async fn hunk_patch(
    AxumPath((session_id, hunk_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<PatchPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::hunk_patch(&state, &session_id, &hunk_id)?))
}

async fn write_session_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::WriteFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::write_session_file(&state, &session_id, &request.file_path, &request.content)?;
    Ok("ok")
}

async fn session_file(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<FileContentPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::session_file(
        &state,
        &session_id,
        &query.file_path,
    )?))
}

async fn find_session_files(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileSearchQuery>,
    State(state): State<AppState>,
) -> Result<Json<FileMatchesPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::find_session_files(
        &state,
        &session_id,
        &query.query,
    )?))
}

async fn search_session_contents(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileSearchQuery>,
    State(state): State<AppState>,
) -> Result<Json<ContentMatchesPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::search_session_contents(
        &state,
        &session_id,
        &query.query,
    )?))
}

async fn resolve_comment(
    AxumPath((session_id, hunk_id, comment_index)): AxumPath<(String, String, usize)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::resolve_comment(&state, &session_id, &hunk_id, comment_index)?;
    Ok("ok")
}

async fn resolve_comment_by_key(
    AxumPath((session_id, hunk_id, comment_key)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::resolve_comment_by_key(&state, &session_id, &hunk_id, &comment_key)?;
    Ok("ok")
}

async fn update_comment(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::CommentRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::update_comment(&state, &session_id, &request)?;
    Ok("ok")
}

async fn send_comment_batch(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::send_comment_batch(&state, &session_id)?;
    Ok("ok")
}

async fn cancel_comment_dispatch_request(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::CancelCommentDispatchRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::cancel_dispatch(&state, &session_id, &request.hunk_id, request.comment_index)?;
    Ok("ok")
}

async fn agent_dispatch_log_request(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<AgentLogQuery>,
    State(state): State<AppState>,
) -> Result<Json<AgentLogPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(service::dispatch_log(
        &state,
        &session_id,
        &query.dispatch_key,
    )?))
}

async fn commit_state(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    Ok(Json(crate::committing::commit_state(&state, &session_id)?))
}

/// Write a commit message from what is staged. A POST rather than a GET: it starts an agent
/// rather than reading something that is already there.
async fn suggest_commit_message(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    Ok(Json(crate::commit_suggestion::suggest_commit_message(
        &state,
        &session_id,
    )?))
}

/// Start `git` on one action. The server only ever spawns git with argv it built itself -
/// what arrives here is which action, not a command line.
async fn start_commit_run(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(action): Json<crate::committing::CommitAction>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let terminal_id = crate::committing::start_commit_run(&state, &session_id, &action)?;
    Ok(Json(crate::api::CommitRunStarted { terminal_id }))
}

async fn commit_run_outcome(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let exit_code = crate::committing::commit_run_outcome(&state, &session_id, &terminal_id)?;
    Ok(Json(crate::api::CommitRunOutcome { exit_code }))
}

async fn stage_all(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::stage_all(&state, &session_id)?;
    Ok("ok")
}

async fn stage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::stage_hunk(&state, &session_id, &request.hunk_id)?;
    Ok("ok")
}

async fn unstage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::unstage_hunk(&state, &session_id, &request.hunk_id)?;
    Ok("ok")
}

async fn stage_selection(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<SelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::stage_selection(&state, &session_id, &request.hunk_id, &request.selection)?;
    Ok("ok")
}

async fn stage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::stage_file(&state, &session_id, &request.file_path)?;
    Ok("ok")
}

async fn unstage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::unstage_file(&state, &session_id, &request.file_path)?;
    Ok("ok")
}

async fn discard_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::discard_hunk(&state, &session_id, &request.hunk_id)?;
    Ok("ok")
}

async fn list_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskView>>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::list_tasks(&state, &session_id)?))
}

async fn create_task(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<Json<TaskView>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::create_task(
        &state,
        &session_id,
        &request,
    )?))
}

async fn delete_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_task(&state, &session_id, &task_id)?;
    Ok("ok")
}

async fn project_commands(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<crate::project::ProjectCommands>, AppError> {
    mark_activity(&state);
    Ok(Json(crate::project::session_commands(&state, &session_id)?))
}

async fn set_project_commands(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::project::ProjectCommands>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::project::set_session_commands(&state, &session_id, &request)?;
    Ok("ok")
}

async fn run_project_command(
    AxumPath((session_id, which)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let terminal_id = crate::project::run(&state, &session_id, which.parse()?)?;
    Ok(Json(TerminalOpened { terminal_id }))
}

async fn list_columns(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<BoardColumn>>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::list_columns(&state, &session_id)?))
}

async fn add_column(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<ColumnLabelRequest>,
) -> Result<Json<BoardColumn>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::add_column(
        &state,
        &session_id,
        &request.label,
    )?))
}

async fn rename_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnLabelRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::rename_column(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        &request.label,
    )?;
    Ok("ok")
}

async fn set_column_arrivals(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnArrivalsRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::set_column_arrivals(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        request.arrivals,
    )?;
    Ok("ok")
}

async fn delete_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_column(&state, &session_id, &ColumnId::new(column_id))?;
    Ok("ok")
}

async fn place_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::place_column(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        request.position,
    )?;
    Ok("ok")
}

async fn place_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<TaskPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::place_tasks(
        &state,
        &session_id,
        &request.task_ids,
        request.status,
        request.position,
    )?;
    Ok("ok")
}

async fn start_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<StartResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::start_resource(&state, &session_id, &task_id, request)?,
    }))
}

async fn list_agent_sessions(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::agent_sessions::AgentSessionView>>, AppError> {
    mark_activity(&state);
    Ok(Json(crate::agent_sessions::list_for_session(
        &state,
        &session_id,
    )?))
}

async fn attach_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<AttachResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::attach_resource(&state, &session_id, &task_id, &request)?,
    }))
}

async fn resume_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::resume_resource(
            &state,
            &session_id,
            &task_id,
            &resource_id,
        )?,
    }))
}

async fn stop_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::stop_resource(&state, &session_id, &task_id, &resource_id)?;
    Ok("ok")
}

async fn delete_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_resource(&state, &session_id, &task_id, &resource_id)?;
    Ok("ok")
}

async fn rename_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskTitleRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::rename_task(&state, &session_id, &task_id, &request.title)?;
    Ok("ok")
}

async fn open_task_notes(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TaskNotesPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(TaskNotesPayload {
        file_path: moontasks::service::open_notes(&state, &session_id, &task_id)?,
    }))
}

async fn link_task_file(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<LinkFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::link_file(&state, &session_id, &task_id, &request.file_path)?;
    Ok("ok")
}

async fn discard_hunks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkBatchRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::discard_hunks(&state, &session_id, &request.hunk_ids)?;
    Ok("ok")
}
