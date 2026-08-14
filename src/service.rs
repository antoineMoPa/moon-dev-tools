//! Review operations shared by both frontends.
//!
//! The axum routes in [`crate::server`] and the native egui app in [`crate::native`] are
//! two skins over this module. Everything here is synchronous and takes `&AppState`, so
//! the native app calls it directly instead of talking HTTP to itself.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    api::{
        AgentKind, AgentLogPayload, AppState, CommitHistoryPayload, CommitReviewStatus, CommitView,
        DiffHunk, DiffTarget, FileContentPayload, HunkView, OpenSessionRequest, PatchPayload,
        RepoSession, SessionOpened, SessionPayload, SubmoduleView,
    },
    comments::{
        agent_dispatch_log, anchored_comment_key, anchored_comments_only,
        build_anchored_comment_value, build_export_text, build_review_comments,
        cancel_comment_dispatch, comment_dispatch_view, parse_anchored_comments,
        plan_batched_comment_dispatches, plan_comment_dispatches, spawn_comment_dispatch,
    },
    git::{
        agent_is_available, agent_options, apply_patch, branch_commits_since_default,
        build_partial_patch_from_selection, canonicalize_repo, collect_session_hunks,
        commit_history_page, commit_view, current_branch_name, list_changed_submodule_repos,
        local_change_summary_from_status, preview_patch, read_repo_file, run_git,
        run_git_no_output,
    },
    reviewed_cache::{
        hunk_patch_hash, mark_hunk_patch_reviewed, read_reviewed_hunk_hashes,
        unmark_hunk_patch_reviewed,
    },
};

pub(crate) const PATCH_PREVIEW_LINE_LIMIT: usize = 500;
pub(crate) const HISTORY_COMMIT_PAGE_SIZE: usize = 30;

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

fn review_status_for_hunks(
    session: &RepoSession,
    cached_reviewed: &HashSet<String>,
    hunks: &[DiffHunk],
) -> CommitReviewStatus {
    if hunks.is_empty() {
        return CommitReviewStatus::Unreviewed;
    }

    let reviewed_count = hunks
        .iter()
        .filter(|hunk| {
            session.reviewed.contains(&hunk.id)
                || cached_reviewed.contains(&hunk_patch_hash(&hunk.patch))
        })
        .count();

    if reviewed_count == hunks.len() {
        CommitReviewStatus::Reviewed
    } else if reviewed_count > 0 {
        CommitReviewStatus::Partial
    } else {
        CommitReviewStatus::Unreviewed
    }
}

fn apply_commit_status(commits: &mut [CommitView], commit_sha: &str, status: CommitReviewStatus) {
    if let Some(commit) = commits.iter_mut().find(|commit| commit.sha == commit_sha) {
        commit.review_status = status;
    }
}

fn apply_cached_commit_statuses(session: &RepoSession, commits: &mut [CommitView]) {
    for commit in commits {
        if let Some(status) = session.commit_statuses.get(&commit.sha) {
            commit.review_status = *status;
        }
    }
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
    let repo_path = canonicalize_repo(PathBuf::from(request.repo_path))?;
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
                    reviewed: HashSet::new(),
                    commit_statuses: HashMap::new(),
                    selected_agent: AgentKind::None,
                    comment_dispatches: HashMap::new(),
                },
            );
        }
    }

    Ok(SessionOpened { session_id })
}

pub(crate) fn session_state(state: &AppState, session_id: &str) -> Result<SessionPayload> {
    let available_agents = agent_options(state.agent_availability);
    crate::api::with_session(state, session_id, |session| {
        let hunks = collect_session_hunks(session)?;
        let full_file_path = unchanged_file_path(
            &session.repo_path,
            &session.diff_target,
            session.active_commit.as_deref(),
            !hunks.is_empty(),
        );
        let move_hints = crate::moved_hunks::detect_hunk_moves(&hunks);
        let cached_reviewed = read_reviewed_hunk_hashes(&session.repo_path)?;
        let (commit_base, mut commits) = branch_commits_since_default(&session.repo_path)?;
        let (mut history_commits, history_has_more) = commit_history_page(
            &session.repo_path,
            &branch_commit_shas(&commits),
            0,
            HISTORY_COMMIT_PAGE_SIZE,
        )?;
        ensure_active_commit_visible(
            &session.repo_path,
            &commits,
            &mut history_commits,
            session.active_commit.as_deref(),
        )?;
        apply_cached_commit_statuses(session, &mut commits);
        apply_cached_commit_statuses(session, &mut history_commits);
        if let Some(active_commit) = session.active_commit.as_deref() {
            let active_commit_status = review_status_for_hunks(session, &cached_reviewed, &hunks);
            session
                .commit_statuses
                .insert(active_commit.to_string(), active_commit_status);
            apply_commit_status(&mut commits, active_commit, active_commit_status);
            apply_commit_status(&mut history_commits, active_commit, active_commit_status);
        }
        let local_change_summary = if session.diff_target.comparison.is_some() {
            Default::default()
        } else {
            local_change_summary_from_status(
                &session.repo_path,
                session.diff_target.pathspec.as_deref(),
            )?
        };
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
            branch_name: current_branch_name(&session.repo_path)?,
            commit_base,
            commits,
            history_commits,
            history_has_more,
            local_change_summary,
            active_commit: session.active_commit.clone(),
            repo_path: session.repo_path.display().to_string(),
            read_only,
            patch_preview_line_limit: PATCH_PREVIEW_LINE_LIMIT,
            available_agents: available_agents.clone(),
            selected_agent: session.selected_agent,
            full_file_path,
            review_comments: build_review_comments(session, &views),
            export_text: build_export_text(session_id, &views),
            hunks: views,
        })
    })
}

pub(crate) fn session_submodules(state: &AppState, session_id: &str) -> Result<Vec<SubmoduleView>> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;

    Ok(list_changed_submodule_repos(&repo_path)?
        .into_iter()
        .map(|path| SubmoduleView {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string()),
            repo_path: path.display().to_string(),
        })
        .collect())
}

pub(crate) fn commit_history(
    state: &AppState,
    session_id: &str,
    offset: usize,
    limit: usize,
) -> Result<CommitHistoryPayload> {
    crate::api::with_session(state, session_id, |session| {
        let (_, commits) = branch_commits_since_default(&session.repo_path)?;
        let (mut commits, has_more) = commit_history_page(
            &session.repo_path,
            &branch_commit_shas(&commits),
            offset,
            limit.min(100),
        )?;
        apply_cached_commit_statuses(session, &mut commits);

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

pub(crate) fn session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
) -> Result<FileContentPayload> {
    crate::api::with_session(state, session_id, |session| {
        Ok(FileContentPayload {
            file_path: file_path.to_string(),
            content: read_repo_file(&session.repo_path, file_path)?,
        })
    })
}

pub(crate) fn write_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    content: &str,
) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    crate::api::with_session(state, session_id, |session| {
        crate::git::write_repo_file(&session.repo_path, file_path, content)
    })
}

pub(crate) fn resolve_comment(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_index: usize,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let Some(existing) = session.comments.get(hunk_id).cloned() else {
            bail!("comment no longer exists");
        };

        let mut anchored = parse_anchored_comments(&existing);
        let Some(entry) = anchored.get_mut(comment_index) else {
            bail!("comment index is out of bounds");
        };
        entry.resolved = true;

        store_anchored_comments(session, hunk_id, &anchored);
        Ok(())
    })
}

pub(crate) fn resolve_comment_by_key(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_key: &str,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let Some(existing) = session.comments.get(hunk_id).cloned() else {
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

        store_anchored_comments(session, hunk_id, &anchored);
        Ok(())
    })
}

fn store_anchored_comments(
    session: &mut RepoSession,
    hunk_id: &str,
    anchored: &[crate::comments::AnchoredComment],
) {
    let next = build_anchored_comment_value(anchored);
    if next.trim().is_empty() {
        session.comments.remove(hunk_id);
    } else {
        session.comments.insert(hunk_id.to_string(), next);
    }
}

pub(crate) fn toggle_reviewed(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    reviewed: Option<bool>,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let cached_reviewed = read_reviewed_hunk_hashes(&session.repo_path)?;
        let Some(hunk) = collect_session_hunks(session)?
            .into_iter()
            .find(|hunk| hunk.id == hunk_id)
        else {
            return Ok(());
        };
        let is_reviewed = session.reviewed.contains(&hunk.id)
            || cached_reviewed.contains(&hunk_patch_hash(&hunk.patch));
        let next_reviewed = reviewed.unwrap_or(!is_reviewed);

        if next_reviewed {
            mark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
            session.reviewed.insert(hunk_id.to_string());
        } else {
            unmark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
            session.reviewed.remove(hunk_id);
        }

        Ok(())
    })
}

pub(crate) fn update_file_reviewed(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    reviewed: bool,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        let hunks = collect_session_hunks(session)?
            .into_iter()
            .filter(|hunk| hunk.file_path == file_path)
            .collect::<Vec<_>>();
        for hunk in hunks {
            if reviewed {
                mark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
                session.reviewed.insert(hunk.id);
            } else {
                unmark_hunk_patch_reviewed(&session.repo_path, &hunk.patch)?;
                session.reviewed.remove(&hunk.id);
            }
        }
        Ok(())
    })
}

pub(crate) fn update_comment(
    state: &AppState,
    session_id: &str,
    request: &crate::api::CommentRequest,
) -> Result<()> {
    let dispatch_jobs = crate::api::with_session(state, session_id, |session| {
        plan_comment_dispatches(session, session_id, request)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok(())
}

pub(crate) fn send_comment_batch(state: &AppState, session_id: &str) -> Result<()> {
    let dispatch_jobs = crate::api::with_session(state, session_id, |session| {
        plan_batched_comment_dispatches(session, session_id)
    })?;

    for job in dispatch_jobs {
        spawn_comment_dispatch(state.clone(), job);
    }

    Ok(())
}

pub(crate) fn cancel_dispatch(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    comment_index: usize,
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        cancel_comment_dispatch(session, hunk_id, comment_index)
    })
}

pub(crate) fn dispatch_log(
    state: &AppState,
    session_id: &str,
    dispatch_key: &str,
) -> Result<AgentLogPayload> {
    crate::api::with_session(state, session_id, |session| {
        Ok(AgentLogPayload {
            dispatch_key: dispatch_key.to_string(),
            text: agent_dispatch_log(session, dispatch_key)?,
        })
    })
}

pub(crate) fn stage_hunk(state: &AppState, session_id: &str, hunk_id: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patch, is_staged) = crate::api::lookup_hunk(state, session_id, hunk_id)?;
    if is_staged {
        return Ok(());
    }
    apply_patch(&repo_path, &patch, true, false)?;
    mark_hunk_patch_reviewed(&repo_path, &patch)?;
    Ok(())
}

pub(crate) fn unstage_hunk(state: &AppState, session_id: &str, hunk_id: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patch, is_staged) = crate::api::lookup_hunk(state, session_id, hunk_id)?;
    if !is_staged {
        return Ok(());
    }
    apply_patch(&repo_path, &patch, true, true)?;
    Ok(())
}

pub(crate) fn stage_selection(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
    selection: &str,
) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patch, is_staged) = crate::api::lookup_hunk(state, session_id, hunk_id)?;
    if is_staged {
        return Ok(());
    }
    let partial_patch = build_partial_patch_from_selection(&patch, selection)?;
    apply_patch(&repo_path, &partial_patch, true, false)?;
    mark_hunk_patch_reviewed(&repo_path, &patch)?;
    Ok(())
}

pub(crate) fn stage_file(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patches) = crate::api::with_session(state, session_id, |session| {
        let patches = collect_session_hunks(session)?
            .into_iter()
            .filter(|hunk| hunk.file_path == file_path && !hunk.staged)
            .map(|hunk| hunk.patch)
            .collect::<Vec<_>>();
        Ok((session.repo_path.clone(), patches))
    })?;
    run_git_no_output(&repo_path, &["add", "--", file_path])?;
    for patch in patches {
        mark_hunk_patch_reviewed(&repo_path, &patch)?;
    }
    Ok(())
}

pub(crate) fn unstage_file(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    run_git_no_output(&repo_path, &["restore", "--staged", "--", file_path])?;
    Ok(())
}

pub(crate) fn discard_hunk(state: &AppState, session_id: &str, hunk_id: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patch, is_staged) = crate::api::lookup_hunk(state, session_id, hunk_id)?;

    apply_patch(&repo_path, &patch, false, true)?;
    if is_staged {
        apply_patch(&repo_path, &patch, true, true)?;
    }

    Ok(())
}

pub(crate) fn discard_hunks(state: &AppState, session_id: &str, hunk_ids: &[String]) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, patches) = crate::api::lookup_hunks(state, session_id, hunk_ids)?;

    for (patch, is_staged) in patches {
        apply_patch(&repo_path, &patch, false, true)?;
        if is_staged {
            apply_patch(&repo_path, &patch, true, true)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn unchanged_file_path_only_selects_clean_local_files() {
        let repo_path = std::env::temp_dir().join(format!(
            "moonreview-service-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(repo_path.join("src")).expect("failed to create test directory");
        fs::write(repo_path.join("src/example.rs"), "fn main() {}\n")
            .expect("failed to write test file");

        let file_target = DiffTarget {
            base: None,
            pathspec: Some("src/example.rs".to_string()),
            comparison: None,
        };
        assert_eq!(
            unchanged_file_path(&repo_path, &file_target, None, false).as_deref(),
            Some("src/example.rs")
        );
        assert_eq!(
            unchanged_file_path(&repo_path, &file_target, None, true),
            None
        );
        assert_eq!(
            unchanged_file_path(&repo_path, &file_target, Some("abc123"), false),
            None
        );

        let directory_target = DiffTarget {
            pathspec: Some("src".to_string()),
            ..Default::default()
        };
        assert_eq!(
            unchanged_file_path(&repo_path, &directory_target, None, false),
            None
        );

        let diff_target = DiffTarget {
            base: Some("main".to_string()),
            pathspec: Some("src/example.rs".to_string()),
            comparison: None,
        };
        assert_eq!(
            unchanged_file_path(&repo_path, &diff_target, None, false),
            None
        );

        fs::remove_dir_all(repo_path).expect("failed to remove test directory");
    }
}
