use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use anyhow::{Result, anyhow};

use crate::{
    agent::run_agent_dispatch,
    api::{
        AgentKind, AgentLog, AppState, CancelToken, CommentDispatchStatus, CommentDispatchView,
        CommentRequest, DiffHunk, HunkCommentContext, HunkView, RepoSession, ReviewCommentView,
        append_to_agent_log, export_server_url, server_url, with_session,
    },
    git::collect_session_hunks,
};

const ANCHOR_OPEN: &str = "[[mr-anchor]]";
const SELECTION_MARK: &str = "[[selection]]";
const RESOLVED_MARK: &str = "[[resolved]]";
const COMMENT_MARK: &str = "[[comment]]";
const ANCHOR_CLOSE: &str = "[[/mr-anchor]]";

#[derive(Clone, Default)]
pub(crate) struct CommentDispatchState {
    pub(crate) status: CommentDispatchStatus,
    pub(crate) detail: String,
    pub(crate) agent: AgentKind,
    pub(crate) cancel_token: Option<CancelToken>,
    pub(crate) log: Option<AgentLog>,
}

pub(crate) struct AnchoredComment {
    pub(crate) selection: String,
    pub(crate) comment: String,
    pub(crate) resolved: bool,
}

pub(crate) struct CommentUpdate {
    pub(crate) previous_anchored: Vec<AnchoredComment>,
    pub(crate) next_anchored: Vec<AnchoredComment>,
}

#[derive(Clone)]
pub(crate) struct DispatchTarget {
    pub(crate) hunk_id: String,
    pub(crate) file_path: String,
    pub(crate) header: String,
    pub(crate) comment_key: String,
    pub(crate) selection: String,
    pub(crate) comment: String,
}

#[derive(Clone)]
pub(crate) struct DispatchJob {
    pub(crate) session_id: String,
    pub(crate) repo_path: PathBuf,
    pub(crate) ui_url: String,
    pub(crate) agent: AgentKind,
    pub(crate) targets: Vec<DispatchTarget>,
    pub(crate) cancel_token: CancelToken,
    pub(crate) log: AgentLog,
}

pub(crate) fn build_anchored_comment_value(comments: &[AnchoredComment]) -> String {
    comments
        .iter()
        .map(|entry| {
            let mut lines = vec![
                ANCHOR_OPEN.to_string(),
                SELECTION_MARK.to_string(),
                entry.selection.trim().to_string(),
            ];
            if entry.resolved {
                lines.push(RESOLVED_MARK.to_string());
            }
            lines.push(COMMENT_MARK.to_string());
            lines.push(entry.comment.trim().to_string());
            lines.push(ANCHOR_CLOSE.to_string());
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn anchored_comments_only(value: &str) -> String {
    let mut blocks = Vec::new();
    let mut remaining = value;

    while let Some(start) = remaining.find(ANCHOR_OPEN) {
        let after_start = &remaining[start..];
        let Some(end_offset) = after_start.find(ANCHOR_CLOSE) else {
            break;
        };
        let end = start + end_offset + ANCHOR_CLOSE.len();
        let block = remaining[start..end].trim();
        if !block.is_empty() {
            blocks.push(block.to_string());
        }
        remaining = &remaining[end..];
    }

    blocks.join("\n\n")
}

pub(crate) fn parse_anchored_comments(value: &str) -> Vec<AnchoredComment> {
    let mut comments = Vec::new();
    let mut remaining = value;

    while let Some(start) = remaining.find(ANCHOR_OPEN) {
        let after_open = &remaining[start + ANCHOR_OPEN.len()..];
        let Some(selection_start) = after_open.find(SELECTION_MARK) else {
            break;
        };
        let after_selection_mark = &after_open[selection_start + SELECTION_MARK.len()..];
        let Some(comment_start) = after_selection_mark.find(COMMENT_MARK) else {
            break;
        };
        let selection = after_selection_mark[..comment_start].trim();
        let resolved = after_selection_mark
            .find(RESOLVED_MARK)
            .is_some_and(|index| index < comment_start);
        let after_comment_mark = &after_selection_mark[comment_start + COMMENT_MARK.len()..];
        let Some(close_start) = after_comment_mark.find(ANCHOR_CLOSE) else {
            break;
        };
        let comment = after_comment_mark[..close_start].trim();

        if !selection.is_empty() || !comment.is_empty() {
            comments.push(AnchoredComment {
                selection: selection.to_string(),
                comment: comment.to_string(),
                resolved,
            });
        }

        remaining = &after_comment_mark[close_start + ANCHOR_CLOSE.len()..];
    }

    comments
}

pub(crate) fn prune_comment_dispatches(
    session: &mut RepoSession,
    hunk_id: &str,
    comments: &[AnchoredComment],
) {
    let allowed = comments
        .iter()
        .map(|entry| dispatch_key(hunk_id, entry))
        .collect::<HashSet<_>>();
    session
        .comment_dispatches
        .retain(|key, _| !key.starts_with(&format!("{hunk_id}:")) || allowed.contains(key));
}

pub(crate) fn anchored_comment_key(entry: &AnchoredComment) -> String {
    crate::api::stable_id(&(entry.selection.trim(), entry.comment.trim()))
}

pub(crate) fn dispatch_key(hunk_id: &str, entry: &AnchoredComment) -> String {
    format!("{hunk_id}:{}", anchored_comment_key(entry))
}

pub(crate) fn spawn_comment_dispatch(state: AppState, job: DispatchJob) {
    thread::spawn(move || {
        let _ = with_session(&state, &job.session_id, |session| {
            for target in &job.targets {
                let Some(dispatch) = session
                    .comment_dispatches
                    .get_mut(&format!("{}:{}", target.hunk_id, target.comment_key))
                else {
                    continue;
                };
                dispatch.status = CommentDispatchStatus::Running;
                dispatch.detail = format!("Running in {}.", job.agent.label());
                dispatch.agent = job.agent;
                dispatch.cancel_token = Some(Arc::clone(&job.cancel_token));
                dispatch.log = Some(Arc::clone(&job.log));
            }
            Ok(())
        });

        eprintln!(
            "[moonreview] dispatch start agent={} repo={} comments={}",
            job.agent.label(),
            job.repo_path.display(),
            job.targets.len(),
        );
        let result = run_agent_dispatch(&job);
        let canceled = job.cancel_token.load(Ordering::SeqCst);
        match &result {
            Ok(detail) => append_to_agent_log(&job.log, &format!("\n[moonreview] {detail}\n")),
            Err(error) => append_to_agent_log(&job.log, &format!("\n[moonreview] {error}\n")),
        }
        match &result {
            Ok(detail) => eprintln!(
                "[moonreview] dispatch done agent={} comments={} detail={}",
                job.agent.label(),
                job.targets.len(),
                detail
            ),
            Err(error) => eprintln!(
                "[moonreview] dispatch failed agent={} comments={} error={}",
                job.agent.label(),
                job.targets.len(),
                error
            ),
        }
        let _ = with_session(&state, &job.session_id, |session| {
            for target in &job.targets {
                let Some(dispatch) = session
                    .comment_dispatches
                    .get_mut(&format!("{}:{}", target.hunk_id, target.comment_key))
                else {
                    continue;
                };
                dispatch.cancel_token = None;
                if canceled {
                    dispatch.status = CommentDispatchStatus::Canceled;
                    dispatch.detail = format!("Stopped {}.", job.agent.label());
                    continue;
                }
                match &result {
                    Ok(detail) => {
                        dispatch.status = CommentDispatchStatus::Completed;
                        dispatch.detail = detail.clone();
                    }
                    Err(error) => {
                        dispatch.status = CommentDispatchStatus::Failed;
                        dispatch.detail = error.to_string();
                    }
                }
            }

            Ok(())
        });
    });
}

pub(crate) fn cancel_comment_dispatch(
    session: &mut RepoSession,
    hunk_id: &str,
    comment_index: usize,
) -> Result<()> {
    let Some(existing) = session.comments.get(hunk_id) else {
        return Err(anyhow!("comment no longer exists"));
    };
    let anchored = parse_anchored_comments(existing);
    let Some(entry) = anchored.get(comment_index) else {
        return Err(anyhow!("comment index is out of bounds"));
    };
    let dispatch_key = dispatch_key(hunk_id, entry);
    let Some(dispatch) = session.comment_dispatches.get_mut(&dispatch_key) else {
        return Err(anyhow!("agent dispatch no longer exists"));
    };

    match dispatch.status {
        CommentDispatchStatus::Queued | CommentDispatchStatus::Running => {
            let Some(cancel_token) = &dispatch.cancel_token else {
                return Err(anyhow!("agent dispatch cannot be stopped yet"));
            };
            cancel_token.store(true, Ordering::SeqCst);
            dispatch.detail = "Stopping agent.".to_string();
            Ok(())
        }
        _ => Err(anyhow!("agent dispatch is not running")),
    }
}

pub(crate) fn build_export_text(session_id: &str, hunks: &[HunkView]) -> String {
    let mut out = String::new();
    out.push_str("Moon Review notes\n");
    out.push_str("=================\n");
    out.push_str("Please fix these code issues and mark as resolved:\n\n");

    for hunk in hunks
        .iter()
        .filter(|h| h.reviewed || !h.comment.trim().is_empty())
    {
        let anchored = parse_anchored_comments(&hunk.comment);
        if anchored.iter().all(|entry| entry.resolved) {
            continue;
        }

        out.push_str(&format!("{} {}\n", hunk.file_path, hunk.header));
        for (comment_index, entry) in anchored.into_iter().enumerate() {
            if entry.resolved {
                continue;
            }
            out.push_str("Selected code: ");
            out.push_str(&entry.selection);
            out.push('\n');
            out.push_str("Issue: ");
            out.push_str(&entry.comment);
            out.push('\n');
            out.push_str("Poke this url when done: ");
            out.push_str(&format!(
                "{}/api/session/{session_id}/resolve/{}/{comment_index}",
                export_server_url(),
                hunk.id
            ));
            out.push('\n');
            out.push('\n');
        }
    }

    if out.trim()
        == "Moon Review notes\n=================\nPlease fix these code issues and mark as resolved:"
    {
        out.push_str("No review notes yet.\n");
    }

    out
}

/// Every comment of the session, whether or not its hunk is still part of the current diff.
/// Feeds both the sidebar comment list and the agent monitor window.
pub(crate) fn build_review_comments(
    session: &RepoSession,
    current_hunks: &[HunkView],
) -> Vec<ReviewCommentView> {
    let mut review_comments = Vec::new();
    let mut seen_hunks = HashSet::new();

    for hunk in current_hunks {
        seen_hunks.insert(hunk.id.clone());
        let anchored = parse_anchored_comments(&hunk.comment);
        for (comment_index, entry) in anchored.into_iter().enumerate() {
            review_comments.push(ReviewCommentView {
                dispatch: comment_dispatch_view(session, &hunk.id, &entry),
                hunk_id: hunk.id.clone(),
                comment_index,
                file_path: hunk.file_path.clone(),
                header: hunk.header.clone(),
                selection: entry.selection.clone(),
                comment: entry.comment.clone(),
                resolved: entry.resolved,
                jumpable: true,
            });
        }
    }

    for (hunk_id, stored_comment) in &session.comments {
        if seen_hunks.contains(hunk_id) {
            continue;
        }

        let Some(context) = session.comment_contexts.get(hunk_id) else {
            continue;
        };
        let anchored = parse_anchored_comments(stored_comment);
        for (comment_index, entry) in anchored.into_iter().enumerate() {
            review_comments.push(ReviewCommentView {
                dispatch: comment_dispatch_view(session, hunk_id, &entry),
                hunk_id: hunk_id.clone(),
                comment_index,
                file_path: context.file_path.clone(),
                header: context.header.clone(),
                selection: entry.selection.clone(),
                comment: entry.comment.clone(),
                resolved: entry.resolved,
                jumpable: false,
            });
        }
    }

    review_comments
}

pub(crate) fn plan_comment_dispatches(
    session: &mut RepoSession,
    session_id: &str,
    request: &CommentRequest,
) -> Result<Vec<DispatchJob>> {
    let update = apply_comment_update(session, request);
    if request.batch {
        plan_batched_comment_save(session, session_id, request, &update)?;
        return Ok(Vec::new());
    }

    if !should_dispatch_comments(session, &update) {
        return Ok(Vec::new());
    }

    let hunk = lookup_dispatch_hunk(session, &request.hunk_id)?;
    let previous_keys = update
        .previous_anchored
        .iter()
        .map(anchored_comment_key)
        .collect::<HashSet<_>>();

    let jobs = update
        .next_anchored
        .iter()
        .filter(|entry| !entry.resolved)
        .filter_map(|entry| {
            queue_dispatch_job(session, session_id, request, &hunk, &previous_keys, entry)
        })
        .collect();

    Ok(jobs)
}

fn plan_batched_comment_save(
    session: &mut RepoSession,
    session_id: &str,
    request: &CommentRequest,
    update: &CommentUpdate,
) -> Result<()> {
    if update.next_anchored.is_empty() {
        return Ok(());
    }

    let hunk = lookup_dispatch_hunk(session, &request.hunk_id)?;
    let previous_keys = update
        .previous_anchored
        .iter()
        .map(anchored_comment_key)
        .collect::<HashSet<_>>();

    for entry in update.next_anchored.iter().filter(|entry| !entry.resolved) {
        let _ = queue_dispatch_job(session, session_id, request, &hunk, &previous_keys, entry);
    }

    Ok(())
}

pub(crate) fn plan_batched_comment_dispatches(
    session: &mut RepoSession,
    session_id: &str,
) -> Result<Vec<DispatchJob>> {
    if session.selected_agent == AgentKind::None {
        return Err(anyhow!("select an agent before sending a batch"));
    }

    let hunks = collect_session_hunks(session)?;
    let mut targets = Vec::new();

    for hunk in hunks {
        let stored_comment = session.comments.get(&hunk.id).cloned().unwrap_or_default();
        for entry in parse_anchored_comments(&stored_comment)
            .into_iter()
            .filter(|entry| !entry.resolved)
        {
            let dispatch_key = dispatch_key(&hunk.id, &entry);
            let Some(dispatch) = session.comment_dispatches.get_mut(&dispatch_key) else {
                continue;
            };
            if dispatch.status != CommentDispatchStatus::Batched {
                continue;
            }

            dispatch.status = CommentDispatchStatus::Queued;
            dispatch.detail = format!("Queued for {}.", session.selected_agent.label());
            dispatch.agent = session.selected_agent;
            targets.push(build_dispatch_target(&hunk.id, &hunk, &entry));
        }
    }

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let cancel_token = Arc::new(AtomicBool::new(false));
    for target in &targets {
        let dispatch_key = format!("{}:{}", target.hunk_id, target.comment_key);
        if let Some(dispatch) = session.comment_dispatches.get_mut(&dispatch_key) {
            dispatch.cancel_token = Some(Arc::clone(&cancel_token));
        }
    }

    Ok(vec![DispatchJob {
        session_id: session_id.to_string(),
        repo_path: session.repo_path.clone(),
        ui_url: format!("{}/review/{session_id}", server_url()),
        agent: session.selected_agent,
        targets,
        cancel_token,
        log: Arc::new(Mutex::new(String::new())),
    }])
}

fn apply_comment_update(session: &mut RepoSession, request: &CommentRequest) -> CommentUpdate {
    let previous_comment = session
        .comments
        .get(&request.hunk_id)
        .cloned()
        .unwrap_or_default();
    let previous_anchored = parse_anchored_comments(&previous_comment);
    remember_hunk_context(session, &request.hunk_id);

    let next_comment = anchored_comments_only(&request.comment);
    if next_comment.trim().is_empty() {
        session.comments.remove(&request.hunk_id);
        session.comment_contexts.remove(&request.hunk_id);
    } else {
        session
            .comments
            .insert(request.hunk_id.clone(), next_comment);
    }

    let stored_comment = session
        .comments
        .get(&request.hunk_id)
        .cloned()
        .unwrap_or_default();
    let next_anchored = parse_anchored_comments(&stored_comment);
    prune_comment_dispatches(session, &request.hunk_id, &next_anchored);

    CommentUpdate {
        previous_anchored,
        next_anchored,
    }
}

fn remember_hunk_context(session: &mut RepoSession, hunk_id: &str) {
    let Ok(Some(hunk)) = collect_session_hunks(session)
        .map(|hunks| hunks.into_iter().find(|hunk| hunk.id == hunk_id))
    else {
        return;
    };

    session.comment_contexts.insert(
        hunk_id.to_string(),
        HunkCommentContext {
            file_path: hunk.file_path,
            header: hunk.header,
        },
    );
}

fn should_dispatch_comments(session: &RepoSession, update: &CommentUpdate) -> bool {
    session.selected_agent != AgentKind::None && !update.next_anchored.is_empty()
}

fn lookup_dispatch_hunk(session: &RepoSession, hunk_id: &str) -> Result<DiffHunk> {
    collect_session_hunks(session)?
        .into_iter()
        .find(|hunk| hunk.id == hunk_id)
        .ok_or_else(|| anyhow!("hunk no longer exists"))
}

fn queue_dispatch_job(
    session: &mut RepoSession,
    session_id: &str,
    request: &CommentRequest,
    hunk: &DiffHunk,
    previous_keys: &HashSet<String>,
    entry: &AnchoredComment,
) -> Option<DispatchJob> {
    let key = anchored_comment_key(entry);
    if previous_keys.contains(&key) {
        return None;
    }

    let dispatch_key = dispatch_key(&request.hunk_id, entry);
    let status = if request.batch {
        CommentDispatchStatus::Batched
    } else {
        CommentDispatchStatus::Queued
    };
    let detail = if request.batch {
        "Waiting for batch send.".to_string()
    } else {
        format!("Queued for {}.", session.selected_agent.label())
    };
    let cancel_token = (!request.batch).then(|| Arc::new(AtomicBool::new(false)));
    session.comment_dispatches.insert(
        dispatch_key,
        CommentDispatchState {
            status,
            detail,
            agent: session.selected_agent,
            cancel_token: cancel_token.as_ref().map(Arc::clone),
            log: None,
        },
    );

    if request.batch {
        return None;
    }

    Some(build_dispatch_job(
        session,
        session_id,
        &request.hunk_id,
        hunk,
        entry,
        cancel_token.expect("non-batch dispatch has a cancel token"),
    ))
}

fn build_dispatch_job(
    session: &RepoSession,
    session_id: &str,
    hunk_id: &str,
    hunk: &DiffHunk,
    entry: &AnchoredComment,
    cancel_token: CancelToken,
) -> DispatchJob {
    DispatchJob {
        session_id: session_id.to_string(),
        repo_path: session.repo_path.clone(),
        ui_url: format!("{}/review/{session_id}", server_url()),
        agent: session.selected_agent,
        targets: vec![build_dispatch_target(hunk_id, hunk, entry)],
        cancel_token,
        log: Arc::new(Mutex::new(String::new())),
    }
}

fn build_dispatch_target(
    hunk_id: &str,
    hunk: &DiffHunk,
    entry: &AnchoredComment,
) -> DispatchTarget {
    DispatchTarget {
        hunk_id: hunk_id.to_string(),
        file_path: hunk.file_path.clone(),
        header: hunk.header.clone(),
        comment_key: anchored_comment_key(entry),
        selection: entry.selection.clone(),
        comment: entry.comment.clone(),
    }
}

pub(crate) fn comment_dispatch_view(
    session: &RepoSession,
    hunk_id: &str,
    entry: &AnchoredComment,
) -> CommentDispatchView {
    let key = dispatch_key(hunk_id, entry);
    let Some(dispatch) = session.comment_dispatches.get(&key) else {
        return CommentDispatchView {
            key,
            status: CommentDispatchStatus::default(),
            detail: String::new(),
            agent: AgentKind::None,
            can_cancel: false,
            has_log: false,
        };
    };

    CommentDispatchView {
        key,
        status: dispatch.status,
        detail: dispatch.detail.clone(),
        agent: dispatch.agent,
        can_cancel: matches!(
            dispatch.status,
            CommentDispatchStatus::Queued | CommentDispatchStatus::Running
        ),
        has_log: dispatch.log.is_some(),
    }
}

pub(crate) fn agent_dispatch_log(session: &RepoSession, dispatch_key: &str) -> Result<String> {
    let Some(dispatch) = session.comment_dispatches.get(dispatch_key) else {
        return Err(anyhow!("agent dispatch no longer exists"));
    };
    let Some(log) = &dispatch.log else {
        return Ok(String::new());
    };

    crate::api::read_agent_log(log)
}
