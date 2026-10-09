//! The routes a review is read and changed through: its session, its files and what is in them,
//! the comments on it, and what is staged and committed.
//!
//! Whatever reads or changes the project is done through [`session_work`], as the person the
//! session belongs to. What only changes what the server itself holds of a session - its
//! agent, its comments, a dispatch being stopped - is done where the request is.

use std::convert::Infallible;

use anyhow::Result;
use axum::{
    Extension, Json,
    body::{Body, Bytes},
    extract::{Path as AxumPath, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::{
    api::{
        AgentLogPayload, AgentLogQuery, AppError, AppState, BlamePayload, CommitHistoryPayload,
        CommitHistoryQuery, CommitSelectionRequest, FileAtQuery, FileContentPayload, FileQuery,
        FileSearchQuery, OpenSessionRequest, PatchPayload, SearchLine, SearchProgress, SearchScope,
        SelectionRequest, SessionOpened, SessionPayload, SubmoduleHubPayload,
    },
    search::SearchListener,
    service,
};

use super::{for_person, mark_activity, session_work};

pub(super) async fn session_submodules(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SubmoduleHubPayload>, AppError> {
    mark_activity(&state);
    // Git runs here, and a window polls it: kept off the async workers, which also carry every
    // shell's socket - a worker held by git is a keystroke held with it.
    let hub = session_work(state, session_id, service::session_submodules).await?;
    Ok(Json(hub))
}

pub(super) async fn open_session(
    State(state): State<AppState>,
    State(profiles): State<super::profiles::Profiles>,
    Extension(user): Extension<super::users::UserId>,
    Json(request): Json<OpenSessionRequest>,
) -> Result<Json<SessionOpened>, AppError> {
    mark_activity(&state);
    // There is no session yet to say whose the work is: it is whoever is opening one, and
    // finding the project is already git run as them.
    let owner = profiles.session_owner(&user)?;
    let opened = for_person(owner.person.clone(), move || {
        service::open_session_for_profile(&state, request, Some(owner))
    })
    .await?;
    Ok(Json(opened))
}

pub(super) async fn session_state(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionPayload>, AppError> {
    mark_activity(&state);
    // Kept off the async workers, as `session_submodules` is: this is the longest thing the
    // server does, and a window asks for it every second.
    let payload = session_work(state, session_id, service::session_state).await?;
    Ok(Json(payload))
}

pub(super) async fn commit_history(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<CommitHistoryQuery>,
    State(state): State<AppState>,
) -> Result<Json<CommitHistoryPayload>, AppError> {
    mark_activity(&state);
    let history = session_work(state, session_id, move |state, session_id| {
        service::commit_history(
            state,
            session_id,
            query.offset.unwrap_or(0),
            query.limit.unwrap_or(service::HISTORY_COMMIT_PAGE_SIZE),
        )
    })
    .await?;
    Ok(Json(history))
}

pub(super) async fn update_agent(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::AgentSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::update_agent(&state, &session_id, request.agent)?;
    Ok("ok")
}

pub(super) async fn update_commit_view(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CommitSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::update_commit_view(state, session_id, request.commit)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn hunk_patch(
    AxumPath((session_id, hunk_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<PatchPayload>, AppError> {
    mark_activity(&state);
    let patch = session_work(state, session_id, move |state, session_id| {
        service::hunk_patch(state, session_id, &hunk_id)
    })
    .await?;
    Ok(Json(patch))
}

pub(super) async fn write_session_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::WriteFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::write_session_file(state, session_id, &request.file_path, &request.content)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn create_session_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::WriteFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::create_session_file(state, session_id, &request.file_path, &request.content)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn session_file(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<FileContentPayload>, AppError> {
    mark_activity(&state);
    let file = session_work(state, session_id, move |state, session_id| {
        service::session_file(state, session_id, &query.file_path)
    })
    .await?;
    Ok(Json(file))
}

pub(super) async fn blame_session_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::BlameRequest>,
) -> Result<Json<BlamePayload>, AppError> {
    mark_activity(&state);
    let blame = session_work(state, session_id, move |state, session_id| {
        service::blame_session_file(state, session_id, &request.file_path, &request.of)
    })
    .await?;
    Ok(Json(blame))
}

pub(super) async fn session_file_at(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileAtQuery>,
    State(state): State<AppState>,
) -> Result<Json<FileContentPayload>, AppError> {
    mark_activity(&state);
    let file = session_work(state, session_id, move |state, session_id| {
        service::session_file_at(state, session_id, &query.file_path, &query.revision)
    })
    .await?;
    Ok(Json(file))
}

pub(super) async fn find_session_files(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileSearchQuery>,
    State(state): State<AppState>,
) -> Response {
    mark_activity(&state);
    streamed_search(state, session_id, move |state, session_id, listener| {
        service::find_session_files(
            state,
            session_id,
            &query.query,
            SearchScope::including_ignored(query.include_ignored),
            listener,
        )
    })
}

pub(super) async fn search_session_contents(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileSearchQuery>,
    State(state): State<AppState>,
) -> Response {
    mark_activity(&state);
    streamed_search(state, session_id, move |state, session_id, listener| {
        service::search_session_contents(
            state,
            session_id,
            &query.query,
            SearchScope::including_ignored(query.include_ignored),
            listener,
        )
    })
}

/// A search answered as it runs: a line of JSON per report - see [`SearchLine`] - an empty
/// line on every tick nothing changed, and the reason as the last line if it failed. The
/// search runs on a blocking thread, as the person the session belongs to, and stops when the
/// client goes: the heartbeat is what notices the connection closing.
fn streamed_search<T: Serialize + Send + 'static>(
    state: AppState,
    session_id: String,
    search: impl FnOnce(&AppState, &str, &mut dyn SearchListener<T>) -> Result<()> + Send + 'static,
) -> Response {
    let (lines, streamed) = tokio::sync::mpsc::channel::<Bytes>(64);
    // Not waited for: the answer is the stream, which starts before the search has anything
    // to say.
    tokio::spawn(async move {
        let failures = lines.clone();
        let searched = session_work(state, session_id, move |state, session_id| {
            search(state, session_id, &mut WireListener { lines })
        })
        .await;
        if let Err(AppError(error)) = searched {
            // A failed send is the client gone, with nobody left to tell.
            let _ = failures
                .send(wire_line(&SearchLine::<T>::Failed(format!("{error:#}"))))
                .await;
        }
    });
    let body = Body::from_stream(futures::stream::unfold(
        streamed,
        |mut streamed| async move {
            streamed
                .recv()
                .await
                .map(|line| (Ok::<_, Infallible>(line), streamed))
        },
    ));
    ([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response()
}

/// The search's listener on the server: what it hears goes down the wire, a line at a time.
struct WireListener {
    lines: tokio::sync::mpsc::Sender<Bytes>,
}

/// One report of a search as the wire carries it: its JSON, and the line break that ends it.
fn wire_line<T: Serialize>(line: &SearchLine<T>) -> Bytes {
    let mut bytes = serde_json::to_vec(line).expect("a search line serializes");
    bytes.push(b'\n');
    Bytes::from(bytes)
}

impl<T: Serialize> SearchListener<T> for WireListener {
    /// The heartbeat: an empty line, which fails to send once the client has hung up.
    fn wanted(&mut self) -> bool {
        self.lines.blocking_send(Bytes::from_static(b"\n")).is_ok()
    }

    fn found(&mut self, progress: SearchProgress<T>) {
        // A failed send is the client gone, which the next `wanted` answers.
        let _ = self
            .lines
            .blocking_send(wire_line(&SearchLine::Found(progress)));
    }
}

pub(super) async fn resolve_comment(
    AxumPath((session_id, hunk_id, comment_index)): AxumPath<(String, String, usize)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::resolve_comment(&state, &session_id, &hunk_id, comment_index)?;
    Ok("ok")
}

pub(super) async fn resolve_comment_by_key(
    AxumPath((session_id, hunk_id, comment_key)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::resolve_comment_by_key(&state, &session_id, &hunk_id, &comment_key)?;
    Ok("ok")
}

pub(super) async fn update_comment(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::CommentRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    // A comment may be handed to an agent, which is started as whoever wrote it.
    session_work(state, session_id, move |state, session_id| {
        service::update_comment(state, session_id, &request)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn send_comment_batch(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, service::send_comment_batch).await?;
    Ok("ok")
}

pub(super) async fn cancel_comment_dispatch_request(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::CancelCommentDispatchRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    service::cancel_dispatch(&state, &session_id, &request.hunk_id, request.comment_index)?;
    Ok("ok")
}

pub(super) async fn agent_dispatch_log_request(
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

pub(super) async fn commit_state(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let commit_state = session_work(state, session_id, crate::committing::commit_state).await?;
    Ok(Json(commit_state))
}

/// Write a commit message from what is staged. A POST rather than a GET: it starts an agent
/// rather than reading something that is already there.
pub(super) async fn suggest_commit_message(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let suggestion = session_work(
        state,
        session_id,
        crate::commit_suggestion::suggest_commit_message,
    )
    .await?;
    Ok(Json(suggestion))
}

/// Start `git` on one action. The server only ever spawns git with argv it built itself -
/// what arrives here is which action, not a command line.
pub(super) async fn start_commit_run(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    State(profiles): State<super::profiles::Profiles>,
    Extension(user): Extension<super::users::UserId>,
    Json(action): Json<crate::committing::CommitAction>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let env = profiles.git_environment(&user)?;
    let namespace = profiles.namespace(&user);
    let terminal_id = session_work(state, session_id, move |state, session_id| {
        crate::committing::start_commit_run_personal(
            state,
            session_id,
            &action,
            Some(&namespace),
            env,
        )
    })
    .await?;
    Ok(Json(crate::api::CommitRunStarted { terminal_id }))
}

pub(super) async fn commit_run_outcome(
    AxumPath((session_id, terminal_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    State(profiles): State<super::profiles::Profiles>,
    Extension(user): Extension<super::users::UserId>,
) -> Result<impl IntoResponse, AppError> {
    mark_activity(&state);
    let namespace = profiles.namespace(&user);
    let exit_code = session_work(state, session_id, move |state, session_id| {
        crate::committing::commit_run_outcome_personal(
            state,
            session_id,
            &terminal_id,
            Some(&namespace),
        )
    })
    .await?;
    Ok(Json(crate::api::CommitRunOutcome { exit_code }))
}

pub(super) async fn stage_all(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, service::stage_all).await?;
    Ok("ok")
}

pub(super) async fn stage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::stage_hunk(state, session_id, &request.hunk_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn unstage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::unstage_hunk(state, session_id, &request.hunk_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn stage_selection(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<SelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::stage_selection(state, session_id, &request.hunk_id, &request.selection)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn stage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::stage_file(state, session_id, &request.file_path)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn unstage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::unstage_file(state, session_id, &request.file_path)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn discard_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::discard_hunk(state, session_id, &request.hunk_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn discard_hunks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkBatchRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        service::discard_hunks(state, session_id, &request.hunk_ids)
    })
    .await?;
    Ok("ok")
}
