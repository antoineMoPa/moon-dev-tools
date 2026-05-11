use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    response::{Html, IntoResponse},
    routing::{get, post},
};

use crate::{
    api::{
        AgentKind, AppError, AppState, CommitReviewStatus, CommitSelectionRequest,
        FileContentPayload, FileQuery, FileReviewedRequest, HunkView, OpenSessionRequest,
        PatchPayload, RepoSession, ReviewedRequest, SelectionRequest, ServerState, SessionOpened,
        SessionPayload, bind_host, port, server_url,
    },
    comments::{
        anchored_comment_key, anchored_comments_only, build_anchored_comment_value,
        build_export_text, build_sidebar_comments, comment_dispatch_view, parse_anchored_comments,
        plan_batched_comment_dispatches, plan_comment_dispatches, spawn_comment_dispatch,
    },
    git::{
        agent_is_available, agent_options, apply_patch, branch_commits_since_default,
        build_partial_patch_from_selection, canonicalize_repo, collect_commit_hunks,
        collect_session_hunks, current_branch_name, detect_agent_availability, preview_patch,
        read_repo_file, run_git_no_output,
    },
    reviewed_cache::{
        hunk_patch_hash, mark_hunk_patch_reviewed, read_reviewed_hunk_hashes,
        unmark_hunk_patch_reviewed,
    },
};

const PATCH_PREVIEW_LINE_LIMIT: usize = 500;
const SERVER_LIFETIME: Duration = Duration::from_secs(30 * 60);
const INDEX_HTML: &str = include_str!("index.html");
const APP_JS: &str = include_str!("../web/dist/app.js");
const APP_CSS: &str = include_str!("../web/dist/app.css");

fn commit_review_status(
    session: &RepoSession,
    cached_reviewed: &HashSet<String>,
    commit: &str,
) -> Result<CommitReviewStatus> {
    let hunks = collect_commit_hunks(&session.repo_path, commit)?;
    if hunks.is_empty() {
        return Ok(CommitReviewStatus::Unreviewed);
    }

    let reviewed_count = hunks
        .iter()
        .filter(|hunk| {
            session.reviewed.contains(&hunk.id)
                || cached_reviewed.contains(&hunk_patch_hash(&hunk.patch))
        })
        .count();
    if reviewed_count == hunks.len() {
        Ok(CommitReviewStatus::Reviewed)
    } else if reviewed_count > 0 {
        Ok(CommitReviewStatus::Partial)
    } else {
        Ok(CommitReviewStatus::Unreviewed)
    }
}

fn diff_line_stats(patch: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;

    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }

    (added, removed)
}

pub(crate) async fn run_server() -> Result<()> {
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let app = Router::new()
        .route("/", get(root))
        .route("/healthz", get(healthz))
        .route("/review/{session_id}", get(review_page))
        .route("/review/{session_id}/file", get(review_page))
        .route("/assets/app.js", get(app_js))
        .route("/assets/app.css", get(app_css))
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
        .route("/api/session/{session_id}/agent", post(update_agent))
        .route("/api/session/{session_id}/commit", post(update_commit_view))
        .route("/api/session/{session_id}/hunk/{hunk_id}", get(hunk_patch))
        .route("/api/session/{session_id}/file", get(session_file))
        .route("/api/session/{session_id}/reviewed", post(toggle_reviewed))
        .route(
            "/api/session/{session_id}/reviewed-file",
            post(update_file_reviewed),
        )
        .route("/api/session/{session_id}/comment", post(update_comment))
        .route(
            "/api/session/{session_id}/comment-batch",
            post(send_comment_batch),
        )
        .route("/api/session/{session_id}/stage", post(stage_hunk))
        .route("/api/session/{session_id}/stage-file", post(stage_file))
        .route(
            "/api/session/{session_id}/stage-selection",
            post(stage_selection),
        )
        .route("/api/session/{session_id}/discard", post(discard_hunk))
        .route("/api/session/{session_id}/unstage", post(unstage_hunk))
        .route("/api/session/{session_id}/unstage-file", post(unstage_file))
        .with_state(AppState {
            inner: Arc::new(Mutex::new(ServerState::default())),
            agent_availability: detect_agent_availability(),
            last_activity: Arc::clone(&last_activity),
        });

    let port = port()?;
    let host = bind_host();
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .with_context(|| format!("failed to bind {host}:{port}"))?;

    println!("Moon Review listening on {}", server_url());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(last_activity))
        .await
        .context("server failed")
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
    if let Ok(mut last_activity) = state.last_activity.lock() {
        *last_activity = Instant::now();
    }
}

async fn root(State(state): State<AppState>) -> impl IntoResponse {
    mark_activity(&state);
    Html("<!doctype html><title>Moon Review</title><p>Open via the CLI in a git repo.</p>")
}

async fn healthz(State(state): State<AppState>) -> &'static str {
    mark_activity(&state);
    "ok"
}

async fn review_page(State(state): State<AppState>) -> Html<&'static str> {
    mark_activity(&state);
    Html(INDEX_HTML)
}

async fn app_js(State(state): State<AppState>) -> impl IntoResponse {
    mark_activity(&state);
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        APP_JS,
    )
}

async fn app_css(State(state): State<AppState>) -> impl IntoResponse {
    mark_activity(&state);
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        APP_CSS,
    )
}

async fn open_session(
    State(state): State<AppState>,
    Json(request): Json<OpenSessionRequest>,
) -> Result<Json<SessionOpened>, AppError> {
    mark_activity(&state);
    let repo_path = canonicalize_repo(PathBuf::from(request.repo_path))?;
    let diff_target = request.diff_target.unwrap_or_default();
    let session_id = crate::api::session_id_for(&repo_path, &diff_target);

    let mut guard = state
        .inner
        .lock()
        .map_err(|_| anyhow!("state lock poisoned"))?;
    match guard.sessions.get_mut(&session_id) {
        Some(session) => {
            session.repo_path = repo_path;
            session.diff_target = diff_target;
            session.active_commit = None;
        }
        None => {
            guard.sessions.insert(
                session_id.clone(),
                RepoSession {
                    repo_path,
                    diff_target,
                    active_commit: None,
                    comments: HashMap::new(),
                    comment_contexts: HashMap::new(),
                    reviewed: HashSet::new(),
                    selected_agent: AgentKind::None,
                    comment_dispatches: HashMap::new(),
                },
            );
        }
    }

    Ok(Json(SessionOpened { session_id }))
}

async fn session_state(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionPayload>, AppError> {
    mark_activity(&state);
    let agent_availability = state.agent_availability;
    let available_agents = agent_options(agent_availability);
    let session = crate::api::with_session(&state, &session_id, |session| {
        let hunks = collect_session_hunks(session)?;
        let cached_reviewed = read_reviewed_hunk_hashes(&session.repo_path)?;
        let (commit_base, mut commits) = branch_commits_since_default(&session.repo_path)?;
        for commit in &mut commits {
            commit.review_status = commit_review_status(session, &cached_reviewed, &commit.sha)?;
        }
        let read_only = session.diff_target.base.is_some() || session.active_commit.is_some();
        let views = hunks
            .into_iter()
            .map(|hunk| {
                let (added_line_count, removed_line_count) = diff_line_stats(&hunk.patch);
                let comment = session
                    .comments
                    .get(&hunk.id)
                    .map(|comment| anchored_comments_only(comment))
                    .unwrap_or_default();
                let comment_dispatches = parse_anchored_comments(&comment)
                    .into_iter()
                    .map(|entry| comment_dispatch_view(session, &hunk.id, &entry))
                    .collect::<Vec<_>>();

                HunkView {
                    reviewed: session.reviewed.contains(&hunk.id)
                        || cached_reviewed.contains(&hunk_patch_hash(&hunk.patch)),
                    id: hunk.id,
                    file_path: hunk.file_path,
                    change_kind: hunk.change_kind,
                    header: hunk.header,
                    staged: hunk.staged,
                    comment,
                    comment_dispatches,
                    patch_preview: preview_patch(&hunk.patch, PATCH_PREVIEW_LINE_LIMIT),
                    patch_line_count: hunk.patch.lines().count(),
                    added_line_count,
                    removed_line_count,
                }
            })
            .collect::<Vec<_>>();

        Ok(SessionPayload {
            repo_name: session
                .repo_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repo")
                .to_string(),
            branch_name: current_branch_name(&session.repo_path)?,
            commit_base,
            commits,
            active_commit: session.active_commit.clone(),
            repo_path: session.repo_path.display().to_string(),
            read_only,
            patch_preview_line_limit: PATCH_PREVIEW_LINE_LIMIT,
            available_agents: available_agents.clone(),
            selected_agent: session.selected_agent,
            sidebar_comments: build_sidebar_comments(session, &views),
            export_text: build_export_text(&session_id, &views),
            hunks: views,
        })
    })?;

    Ok(Json(session))
}

async fn update_agent(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::AgentSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        if !agent_is_available(state.agent_availability, request.agent) {
            bail!("selected agent is not available");
        }
        session.selected_agent = request.agent;
        Ok(())
    })?;

    Ok("ok")
}

async fn update_commit_view(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CommitSelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        session.active_commit = request
            .commit
            .clone()
            .filter(|commit| !commit.trim().is_empty());
        if let Some(commit) = &session.active_commit {
            let _ = collect_session_hunks(session)
                .with_context(|| format!("failed to load commit {commit}"))?;
        }
        Ok(())
    })?;

    Ok("ok")
}

async fn hunk_patch(
    AxumPath((session_id, hunk_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<PatchPayload>, AppError> {
    mark_activity(&state);
    let (_, patch, _) = crate::api::lookup_hunk(&state, &session_id, &hunk_id)?;
    Ok(Json(PatchPayload { patch }))
}

async fn session_file(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<FileContentPayload>, AppError> {
    mark_activity(&state);
    let payload = crate::api::with_session(&state, &session_id, |session| {
        Ok(FileContentPayload {
            file_path: query.file_path.clone(),
            content: read_repo_file(&session.repo_path, &query.file_path)?,
        })
    })?;

    Ok(Json(payload))
}

async fn resolve_comment(
    AxumPath((session_id, hunk_id, comment_index)): AxumPath<(String, String, usize)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        let Some(existing) = session.comments.get(&hunk_id).cloned() else {
            bail!("comment no longer exists");
        };

        let mut anchored = parse_anchored_comments(&existing);
        let Some(entry) = anchored.get_mut(comment_index) else {
            bail!("comment index is out of bounds");
        };
        entry.resolved = true;

        let next = build_anchored_comment_value(&anchored);
        if next.trim().is_empty() {
            session.comments.remove(&hunk_id);
        } else {
            session.comments.insert(hunk_id.clone(), next);
        }
        Ok(())
    })?;

    Ok("ok")
}

async fn resolve_comment_by_key(
    AxumPath((session_id, hunk_id, comment_key)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        let Some(existing) = session.comments.get(&hunk_id).cloned() else {
            bail!("comment no longer exists");
        };

        let mut anchored = parse_anchored_comments(&existing);
        let Some(index) = anchored
            .iter()
            .position(|entry| anchored_comment_key(entry) == comment_key)
        else {
            bail!("comment no longer exists");
        };
        anchored[index].resolved = true;

        let next = build_anchored_comment_value(&anchored);
        if next.trim().is_empty() {
            session.comments.remove(&hunk_id);
        } else {
            session.comments.insert(hunk_id.clone(), next);
        }
        Ok(())
    })?;

    Ok("ok")
}

async fn toggle_reviewed(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<ReviewedRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        let cached_reviewed = read_reviewed_hunk_hashes(&session.repo_path)?;
        let Some(hunk) = collect_session_hunks(session)?
            .into_iter()
            .find(|hunk| hunk.id == request.hunk_id)
        else {
            return Ok(());
        };
        let is_reviewed = session.reviewed.contains(&hunk.id)
            || cached_reviewed.contains(&hunk_patch_hash(&hunk.patch));
        let next_reviewed = request.reviewed.unwrap_or(!is_reviewed);

        if next_reviewed {
            mark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
            session.reviewed.insert(request.hunk_id.clone());
        } else {
            unmark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
            session.reviewed.remove(&request.hunk_id);
        }

        Ok(())
    })?;
    Ok("ok")
}

async fn update_file_reviewed(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<FileReviewedRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::with_session(&state, &session_id, |session| {
        let hunks = collect_session_hunks(session)?
            .into_iter()
            .filter(|hunk| hunk.file_path == request.file_path)
            .collect::<Vec<_>>();
        for hunk in hunks {
            if request.reviewed {
                mark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
                session.reviewed.insert(hunk.id);
            } else {
                unmark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
                session.reviewed.remove(&hunk.id);
            }
        }
        Ok(())
    })?;
    Ok("ok")
}

async fn update_comment(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::CommentRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    let dispatch_jobs = crate::api::with_session(&state, &session_id, |session| {
        plan_comment_dispatches(session, &session_id, &request)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok("ok")
}

async fn send_comment_batch(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    let dispatch_jobs = crate::api::with_session(&state, &session_id, |session| {
        plan_batched_comment_dispatches(session, &session_id)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok("ok")
}

async fn stage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let (repo_path, patch, is_staged) =
        crate::api::lookup_hunk(&state, &session_id, &request.hunk_id)?;
    if is_staged {
        return Ok("ok");
    }
    apply_patch(&repo_path, &patch, true, false)?;
    mark_hunk_patch_reviewed(&repo_path, &patch)?;
    Ok("ok")
}

async fn unstage_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let (repo_path, patch, is_staged) =
        crate::api::lookup_hunk(&state, &session_id, &request.hunk_id)?;
    if !is_staged {
        return Ok("ok");
    }
    apply_patch(&repo_path, &patch, true, true)?;
    Ok("ok")
}

async fn stage_selection(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<SelectionRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let (repo_path, patch, is_staged) =
        crate::api::lookup_hunk(&state, &session_id, &request.hunk_id)?;
    if is_staged {
        return Ok("ok");
    }
    let partial_patch = build_partial_patch_from_selection(&patch, &request.selection)?;
    apply_patch(&repo_path, &partial_patch, true, false)?;
    mark_hunk_patch_reviewed(&repo_path, &patch)?;
    Ok("ok")
}

async fn stage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let (repo_path, patches) = crate::api::with_session(&state, &session_id, |session| {
        let patches = collect_session_hunks(session)?
            .into_iter()
            .filter(|hunk| hunk.file_path == request.file_path && !hunk.staged)
            .map(|hunk| hunk.patch)
            .collect::<Vec<_>>();
        Ok((session.repo_path.clone(), patches))
    })?;
    run_git_no_output(&repo_path, &["add", "--", &request.file_path]).map_err(AppError)?;
    for patch in patches {
        mark_hunk_patch_reviewed(&repo_path, &patch)?;
    }
    Ok("ok")
}

async fn discard_hunk(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::HunkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let (repo_path, patch, is_staged) =
        crate::api::lookup_hunk(&state, &session_id, &request.hunk_id)?;

    apply_patch(&repo_path, &patch, false, true)?;
    if is_staged {
        apply_patch(&repo_path, &patch, true, true)?;
    }

    Ok("ok")
}

async fn unstage_file(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::api::ensure_session_is_writable(&state, &session_id)?;
    let repo_path =
        crate::api::with_session(&state, &session_id, |session| Ok(session.repo_path.clone()))?;
    run_git_no_output(
        &repo_path,
        &["restore", "--staged", "--", &request.file_path],
    )
    .map_err(AppError)?;
    Ok("ok")
}
