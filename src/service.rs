//! Review operations, wherever they are asked for.
//!
//! The window in [`crate::native`] calls these directly, and the axum routes in
//! [`crate::server`] are the same calls for a window on another machine. Everything here is
//! synchronous and takes `&AppState`, so a local window never talks HTTP to itself.

mod comments;
mod files;
mod staging;
#[cfg(test)]
mod tests;

pub(crate) use comments::{
    cancel_dispatch, dispatch_log, resolve_comment, resolve_comment_by_key, send_comment_batch,
    update_comment,
};
pub(crate) use files::{
    blame_session_file, create_session_file, find_session_files, search_session_contents,
    session_file, session_file_at, write_session_file,
};
pub(crate) use staging::{
    discard_hunk, discard_hunks, stage_all, stage_file, stage_hunk, stage_selection, unstage_file,
    unstage_hunk,
};
// Kept beside the wire types, since the window in a browser pages by it too.
pub(crate) use crate::api::HISTORY_COMMIT_PAGE_SIZE;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    agent::{agent_is_available, agent_options},
    api::{
        AgentKind, AppState, CommitHistoryPayload, CommitView, DiffTarget, HunkView,
        OpenSessionRequest, PatchPayload, RepoSession, RepoStatusView, ReviewTarget, SessionOpened,
        SessionPayload, SubmoduleHubPayload,
    },
    comments::{
        anchored_comments_only, build_export_text, build_review_comments, comment_dispatch_view,
        parse_anchored_comments,
    },
    git::{
        branch_commits_since_default, collect_review_hunks, commit_history_page, commit_view,
        current_branch_name, list_submodule_repos, local_change_summary_from_status, preview_patch,
        project_root, run_git,
    },
};

pub(crate) const PATCH_PREVIEW_LINE_LIMIT: usize = 500;

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

fn branch_commit_shas(commits: &[CommitView]) -> HashSet<String> {
    commits.iter().map(|commit| commit.sha.clone()).collect()
}

fn ensure_active_commit_visible(
    repo_path: &Path,
    commits: &[CommitView],
    history_commits: &mut Vec<CommitView>,
    active_commit: Option<&str>,
) -> Result<()> {
    let Some(active_commit) = active_commit else {
        return Ok(());
    };
    if commits.iter().any(|commit| commit.sha == active_commit)
        || history_commits
            .iter()
            .any(|commit| commit.sha == active_commit)
    {
        return Ok(());
    }
    if let Some(commit) = commit_view(repo_path, active_commit)? {
        history_commits.insert(0, commit);
    }
    Ok(())
}

/// A review of one clean file has no hunks to show, so the UI shows the whole file instead.
fn unchanged_file_path(
    repo_path: &Path,
    diff_target: &DiffTarget,
    active_commit: Option<&str>,
    has_hunks: bool,
) -> Option<String> {
    if has_hunks
        || active_commit.is_some()
        || diff_target.base.is_some()
        || diff_target.comparison.is_some()
    {
        return None;
    }

    let pathspec = diff_target.pathspec.as_ref()?;
    repo_path.join(pathspec).is_file().then(|| pathspec.clone())
}

pub(crate) fn open_session(state: &AppState, request: OpenSessionRequest) -> Result<SessionOpened> {
    let repo_path = project_root(PathBuf::from(request.repo_path))?;
    let diff_target = request.diff_target.unwrap_or_default();
    let active_commit = request
        .active_commit
        .clone()
        .filter(|commit| !commit.trim().is_empty());
    if let Some(commit) = &active_commit {
        let commit_ref = format!("{commit}^{{commit}}");
        let _ = run_git(&repo_path, &["rev-parse", "--verify", &commit_ref])
            .with_context(|| format!("failed to load commit {commit}"))?;
    }
    let session_id =
        crate::api::session_id_for_view(&repo_path, &diff_target, active_commit.as_deref());

    let mut guard = state
        .inner
        .lock()
        .map_err(|_| anyhow!("state lock poisoned"))?;
    guard.home_repo.get_or_insert_with(|| repo_path.clone());
    match guard.sessions.get_mut(&session_id) {
        Some(session) => {
            session.repo_path = repo_path;
            session.diff_target = diff_target;
            session.active_commit = active_commit;
        }
        None => {
            guard.sessions.insert(
                session_id.clone(),
                RepoSession {
                    repo_path,
                    diff_target,
                    active_commit,
                    comments: HashMap::new(),
                    comment_contexts: HashMap::new(),
                    selected_agent: AgentKind::None,
                    comment_dispatches: HashMap::new(),
                    files_named_outside_the_repo: crate::lsp::FilesNamedOutsideTheRepo::default(),
                },
            );
        }
    }

    Ok(SessionOpened { session_id })
}

/// Everything the review is made of that comes out of git.
///
/// A window can be open on a folder that is no repo - a shell and a task board need none -
/// and there it is [`GitReview::default`]: no hunks, no branch, no commits. Nothing here is
/// asked of git until the folder is known to be a repo, so that folder runs no git at all.
#[derive(Default)]
struct GitReview {
    hunks: Vec<crate::api::DiffHunk>,
    branch_name: Option<String>,
    commit_base: Option<String>,
    commits: Vec<CommitView>,
    history_commits: Vec<CommitView>,
    history_has_more: bool,
    local_change_summary: crate::api::LocalChangeSummary,
}

/// Everything git is asked for a review, run against a target taken off the session so the
/// server's lock is not held while git works - see [`ReviewTarget`].
fn read_git_review(target: &ReviewTarget) -> Result<GitReview> {
    if !crate::git::is_git_repo(&target.repo_path) {
        return Ok(GitReview::default());
    }
    let hunks = collect_review_hunks(target)?;
    let (commit_base, commits) = branch_commits_since_default(&target.repo_path)?;
    let (mut history_commits, history_has_more) = commit_history_page(
        &target.repo_path,
        &branch_commit_shas(&commits),
        0,
        HISTORY_COMMIT_PAGE_SIZE,
    )?;
    ensure_active_commit_visible(
        &target.repo_path,
        &commits,
        &mut history_commits,
        target.active_commit.as_deref(),
    )?;
    let local_change_summary = if target.diff_target.comparison.is_some() {
        Default::default()
    } else {
        local_change_summary_from_status(&target.repo_path, target.diff_target.pathspec.as_deref())?
    };

    Ok(GitReview {
        hunks,
        branch_name: current_branch_name(&target.repo_path)?,
        commit_base,
        commits,
        history_commits,
        history_has_more,
        local_change_summary,
    })
}

pub(crate) fn session_state(state: &AppState, session_id: &str) -> Result<SessionPayload> {
    let available_agents = agent_options(state.agent_availability);
    // Git runs between two short holds of the lock rather than under one long one: what the
    // review is of is read off the session, git is asked about it, and the answer is put
    // against the session's comments and dispatches once it is back. Every other call the
    // server takes - a shell starting, a hunk being staged - waits on the same lock, and a
    // diff of a repo with a lot changed is the longest thing the server does. Should the
    // session have been pointed elsewhere in the meantime, the answer is about the wrong
    // thing and git is asked again.
    loop {
        let target =
            crate::api::with_session(state, session_id, |session| Ok(session.review_target()))?;
        let review = read_git_review(&target)?;
        // Off git's answer alone, so outside the lock as well: on a diff of hundreds of hunks
        // it is the next longest thing after git itself.
        let move_hints = crate::moved_hunks::detect_hunk_moves(&review.hunks);
        let payload = crate::api::with_session(state, session_id, |session| {
            if session.review_target() != target {
                return Ok(None);
            }
            build_session_payload(session_id, session, review, move_hints, &available_agents)
                .map(Some)
        })?;
        if let Some(payload) = payload {
            return Ok(payload);
        }
    }
}

/// What a review looks like to a window: git's answer about the session's target, put
/// against what the session holds of its own - the comments, and what the agents made of
/// them. Called with the session held, and does no git of its own - nor any reading of the
/// diff: which hunks moved where was worked out before the lock was taken.
fn build_session_payload(
    session_id: &str,
    session: &RepoSession,
    review: GitReview,
    move_hints: crate::moved_hunks::HunkMoveHints,
    available_agents: &[crate::api::AgentOption],
) -> Result<SessionPayload> {
    let GitReview {
        hunks,
        branch_name,
        commit_base,
        commits,
        history_commits,
        history_has_more,
        local_change_summary,
    } = review;
    let full_file_path = unchanged_file_path(
        &session.repo_path,
        &session.diff_target,
        session.active_commit.as_deref(),
        !hunks.is_empty(),
    );
    let read_only = session.diff_target.base.is_some()
        || session.diff_target.comparison.is_some()
        || session.active_commit.is_some();
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
            let moved_from = move_hints.moved_from.get(&hunk.id).cloned();
            let moved_to = move_hints.moved_to.get(&hunk.id).cloned();

            HunkView {
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
                moved_from,
                moved_to,
                image_diff: hunk.image_diff,
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
        branch_name,
        commit_base,
        commits,
        history_commits,
        history_has_more,
        local_change_summary,
        active_commit: session.active_commit.clone(),
        repo_path: session.repo_path.display().to_string(),
        read_only,
        patch_preview_line_limit: PATCH_PREVIEW_LINE_LIMIT,
        available_agents: available_agents.to_vec(),
        selected_agent: session.selected_agent,
        full_file_path,
        review_comments: build_review_comments(session, &views),
        export_text: build_export_text(session_id, &views),
        hunks: views,
    })
}

pub(crate) fn session_submodules(
    state: &AppState,
    session_id: &str,
) -> Result<SubmoduleHubPayload> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;

    // A folder that is no repo has no changed files and no submodules, and neither `git
    // status` nor `git submodule status` has an answer to give in one.
    if !crate::git::is_git_repo(&repo_path) {
        return Ok(SubmoduleHubPayload {
            root: repo_status_view(&repo_path, 0, 0),
            submodules: Vec::new(),
        });
    }

    let root = repo_status_view(
        &repo_path,
        crate::git::changed_file_count(&repo_path)?,
        crate::git::unpushed_commit_count(&repo_path)?,
    );
    let submodules = list_submodule_repos(&repo_path)?
        .into_iter()
        .map(|submodule| {
            repo_status_view(
                &submodule.repo_path,
                submodule.changed_file_count,
                submodule.unpushed_commit_count,
            )
        })
        .collect();
    Ok(SubmoduleHubPayload { root, submodules })
}

fn repo_status_view(
    repo_path: &std::path::Path,
    changed_files: usize,
    unpushed_commits: usize,
) -> RepoStatusView {
    RepoStatusView {
        name: repo_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.display().to_string()),
        repo_path: repo_path.display().to_string(),
        changed_files,
        unpushed_commits,
    }
}

pub(crate) fn commit_history(
    state: &AppState,
    session_id: &str,
    offset: usize,
    limit: usize,
) -> Result<CommitHistoryPayload> {
    crate::api::with_session(state, session_id, |session| {
        if !crate::git::is_git_repo(&session.repo_path) {
            return Ok(CommitHistoryPayload {
                commits: Vec::new(),
                has_more: false,
            });
        }
        let (_, commits) = branch_commits_since_default(&session.repo_path)?;
        let (commits, has_more) = commit_history_page(
            &session.repo_path,
            &branch_commit_shas(&commits),
            offset,
            limit.min(100),
        )?;

        Ok(CommitHistoryPayload { commits, has_more })
    })
}

pub(crate) fn update_agent(state: &AppState, session_id: &str, agent: AgentKind) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        if !agent_is_available(state.agent_availability, agent) {
            bail!("selected agent is not available");
        }
        session.selected_agent = agent;
        Ok(())
    })
}

pub(crate) fn update_commit_view(
    state: &AppState,
    session_id: &str,
    commit: Option<String>,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        session.active_commit = commit.clone().filter(|commit| !commit.trim().is_empty());
        if let Some(commit) = &session.active_commit {
            let commit_ref = format!("{commit}^{{commit}}");
            let _ = run_git(&session.repo_path, &["rev-parse", "--verify", &commit_ref])
                .with_context(|| format!("failed to load commit {commit}"))?;
        }
        Ok(())
    })
}

pub(crate) fn hunk_patch(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
) -> Result<PatchPayload> {
    let (_, patch, _) = crate::api::lookup_hunk(state, session_id, hunk_id)?;
    Ok(PatchPayload { patch })
}
