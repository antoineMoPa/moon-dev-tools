//! Review operations, wherever they are asked for.
//!
//! The window in [`crate::native`] calls these directly, and the axum routes in
//! [`crate::server`] are the same calls for a window on another machine. Everything here is
//! synchronous and takes `&AppState`, so a local window never talks HTTP to itself.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    agent::{agent_is_available, agent_options},
    api::{
        AgentKind, AgentLogPayload, AppState, BlamePayload, CommitHistoryPayload, CommitView,
        ContentMatchesPayload, DiffTarget, FileContentPayload, FileMatchesPayload, HunkView,
        OpenSessionRequest, PatchPayload, RepoSession, RepoStatusView, SessionOpened,
        SessionPayload, SubmoduleHubPayload,
    },
    comments::{
        agent_dispatch_log, anchored_comment_key, anchored_comments_only,
        build_anchored_comment_value, build_export_text, build_review_comments,
        cancel_comment_dispatch, comment_dispatch_view, parse_anchored_comments,
        plan_batched_comment_dispatches, plan_comment_dispatches, spawn_comment_dispatch,
    },
    git::{
        apply_patch, branch_commits_since_default, build_partial_patch_from_selection,
        collect_session_hunks, commit_history_page, commit_view, current_branch_name,
        list_submodule_repos, local_change_summary_from_status, preview_patch, project_root,
        read_repo_file, run_git, run_git_no_output,
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

fn read_git_review(session: &RepoSession) -> Result<GitReview> {
    let hunks = collect_session_hunks(session)?;
    let (commit_base, commits) = branch_commits_since_default(&session.repo_path)?;
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
    let local_change_summary = if session.diff_target.comparison.is_some() {
        Default::default()
    } else {
        local_change_summary_from_status(
            &session.repo_path,
            session.diff_target.pathspec.as_deref(),
        )?
    };

    Ok(GitReview {
        hunks,
        branch_name: current_branch_name(&session.repo_path)?,
        commit_base,
        commits,
        history_commits,
        history_has_more,
        local_change_summary,
    })
}

pub(crate) fn session_state(state: &AppState, session_id: &str) -> Result<SessionPayload> {
    let available_agents = agent_options(state.agent_availability);
    crate::api::with_session(state, session_id, |session| {
        let GitReview {
            hunks,
            branch_name,
            commit_base,
            commits,
            history_commits,
            history_has_more,
            local_change_summary,
        } = if crate::git::is_git_repo(&session.repo_path) {
            read_git_review(session)?
        } else {
            GitReview::default()
        };
        let full_file_path = unchanged_file_path(
            &session.repo_path,
            &session.diff_target,
            session.active_commit.as_deref(),
            !hunks.is_empty(),
        );
        let move_hints = crate::moved_hunks::detect_hunk_moves(&hunks);
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
            available_agents: available_agents.clone(),
            selected_agent: session.selected_agent,
            full_file_path,
            review_comments: build_review_comments(session, &views),
            export_text: build_export_text(session_id, &views),
            hunks: views,
        })
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
            root: repo_status_view(&repo_path, 0),
            submodules: Vec::new(),
        });
    }

    let root = repo_status_view(&repo_path, crate::git::changed_file_count(&repo_path)?);
    let submodules = list_submodule_repos(&repo_path)?
        .into_iter()
        .map(|submodule| repo_status_view(&submodule.repo_path, submodule.changed_file_count))
        .collect();
    Ok(SubmoduleHubPayload { root, submodules })
}

fn repo_status_view(repo_path: &std::path::Path, changed_files: usize) -> RepoStatusView {
    RepoStatusView {
        name: repo_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.display().to_string()),
        repo_path: repo_path.display().to_string(),
        changed_files,
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

/// The text of one file, for a tab that is showing it.
///
/// Two kinds of file reach this: a file of the repo, named relative to its root, and a file
/// outside the repo that a language server named as where something is defined - a
/// dependency's source, or the standard library. The second kind is only ever read when that
/// session's own allow-list holds it, which is what keeps this from being a way to read any
/// file on the machine hosting the repo.
pub(crate) fn session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
) -> Result<FileContentPayload> {
    crate::api::with_session(state, session_id, |session| {
        // A file a language server named outside the repo is read from where it is, and read
        // only - see [`crate::lsp::FilesNamedOutsideTheRepo`]. Every other path is a path in
        // the repo and is refused if it turns out not to be one, exactly as it always was.
        if let Some(real_path) = session.files_named_outside_the_repo.allows(file_path) {
            return Ok(FileContentPayload {
                file_path: file_path.to_string(),
                content: crate::git::read_file_named_outside_the_repo(&real_path)?,
                outside_the_repo: true,
                committed: None,
            });
        }
        Ok(FileContentPayload {
            file_path: file_path.to_string(),
            content: read_repo_file(&session.repo_path, file_path)?,
            outside_the_repo: false,
            committed: Some(crate::git::read_committed_file(
                &session.repo_path,
                file_path,
            )?),
        })
    })
}

/// Who last touched each stretch of a file, taken with `content` as its working-tree version -
/// the tab's buffer, so a line typed a moment ago reads as not committed. Runs where the repo
/// is. A file outside the repo has no history here to ask about, and says so rather than
/// having git say something less clear about a path it cannot see.
pub(crate) fn blame_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    content: &str,
) -> Result<BlamePayload> {
    crate::api::with_session(state, session_id, |session| {
        if session
            .files_named_outside_the_repo
            .allows(file_path)
            .is_some()
        {
            bail!("{file_path} is outside the repo, and has no history here");
        }
        Ok(BlamePayload {
            file_path: file_path.to_string(),
            chunks: crate::git::blame_file(&session.repo_path, file_path, content)?,
        })
    })
}

/// The files of the repo whose names match a search. Runs where the repo is, which is what
/// makes it work on a `--remote` connection as well as a repo on this machine.
pub(crate) fn find_session_files(
    state: &AppState,
    session_id: &str,
    query: &str,
) -> Result<FileMatchesPayload> {
    crate::api::with_session(state, session_id, |session| {
        crate::search::file_names::matching_paths(&session.repo_path, query)
    })
}

/// The lines of the repo that hold what was searched for. Runs where the repo is, the same
/// as the file-name search beside it.
pub(crate) fn search_session_contents(
    state: &AppState,
    session_id: &str,
    query: &str,
) -> Result<ContentMatchesPayload> {
    crate::api::with_session(state, session_id, |session| {
        crate::search::file_contents::matching_lines(&session.repo_path, query)
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

/// Create a file of the repo nothing is at yet - see [`crate::git::create_repo_file`].
pub(crate) fn create_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    content: &str,
) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    crate::api::with_session(state, session_id, |session| {
        crate::git::create_repo_file(&session.repo_path, file_path, content)
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
    Ok(())
}

pub(crate) fn stage_file(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    run_git_no_output(&repo_path, &["add", "--", file_path])?;
    Ok(())
}

/// Stage the whole working tree, untracked files included - the one sweep the commit pane
/// offers.
pub(crate) fn stage_all(state: &AppState, session_id: &str) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    let (repo_path, pathspec) = crate::api::with_session(state, session_id, |session| {
        Ok((
            session.repo_path.clone(),
            session.diff_target.pathspec.clone(),
        ))
    })?;
    // Only what the review is pointed at, when it is pointed at part of the repo.
    let mut args = vec!["add", "-A"];
    crate::git::append_pathspec(&mut args, pathspec.as_deref());
    run_git_no_output(&repo_path, &args)?;
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
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::Instant,
    };

    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    /// A throwaway repo with a session open on it, plus the directory next to that repo where
    /// the fixtures that are deliberately *not* in it live - a stand-in for `~/.cargo` and for
    /// everything else on disk that a read must not reach.
    struct ServedRepo {
        enclosing: PathBuf,
        repo_path: PathBuf,
        state: AppState,
        session_id: String,
    }

    impl Drop for ServedRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.enclosing);
        }
    }

    fn served_repo(name: &str) -> ServedRepo {
        let enclosing = std::env::temp_dir().join(format!(
            "moonreview-outside-{}-{}-{name}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let repo_path = enclosing.join("repo");
        fs::create_dir_all(&repo_path).expect("failed to create the fixture repo");
        run_git_no_output(&repo_path, &["init"]).expect("failed to init the fixture repo");
        fs::write(repo_path.join("lib.rs"), "fn one() {}\n").expect("failed to write in the repo");

        let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
        let session_id = open_session(
            &state,
            OpenSessionRequest {
                repo_path: repo_path.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
        )
        .expect("failed to open the fixture session")
        .session_id;

        ServedRepo {
            enclosing,
            repo_path,
            state,
            session_id,
        }
    }

    impl ServedRepo {
        /// Write a file that is not in the repo, and hand back the absolute path of it - the
        /// shape of path a language server hands back when a definition lands in a dependency.
        fn write_outside(&self, relative: &str, content: &str) -> String {
            let path = self.enclosing.join(relative);
            fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
                .expect("failed to create the fixture directory");
            fs::write(&path, content).expect("failed to write outside the repo");
            path.display().to_string()
        }

        /// Say that a language server named this file as where a definition is, which is the
        /// only thing that ever puts a path outside the repo within reach.
        fn a_server_named(&self, file_path: &str) {
            crate::lsp::remember_files_named(
                &self.state,
                &self.session_id,
                &[crate::api::LspLocation {
                    file_path: file_path.to_string(),
                    line_number: 1,
                    line_text: None,
                }],
            )
            .expect("failed to record what the server named");
        }

        fn read(&self, file_path: &str) -> Result<FileContentPayload> {
            session_file(&self.state, &self.session_id, file_path)
        }
    }

    /// The refusal that guards the machine hosting the repo. `--remote` serves this read over
    /// HTTP, so a path nobody named is refused however it is written: an absolute path to a
    /// credential file, a path next to the repo, and a walk out of the repo with `..`.
    #[test]
    fn a_path_no_language_server_named_is_refused_however_it_is_written() {
        let served = served_repo("refused");
        let secret = served.write_outside("secret.txt", "a private key\n");

        // Each of these is refused, and which refusal it gets says which route it took: a path
        // that names a real file is refused for being outside the repo, and one that resolves
        // to nothing on disk is asked of HEAD instead and refused for not being in the repo's
        // history either. Neither route reads anything.
        for (path, refusal) in [
            ("/etc/passwd", "file path is outside the repository"),
            ("../secret.txt", "file path is outside the repository"),
            (secret.as_str(), "file path is outside the repository"),
            (
                "../../../etc/passwd",
                "file is not available in the working tree or HEAD",
            ),
        ] {
            let refused = served.read(path);
            assert_eq!(
                refused
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default(),
                refusal,
                "{path} was not named by any language server and must be refused"
            );
        }

        // The repo's own files are read exactly as they always were.
        let inside = served
            .read("lib.rs")
            .expect("expected the repo's file to read");
        assert_eq!(inside.content, "fn one() {}\n");
        assert!(!inside.outside_the_repo);
    }

    /// A file a language server named as where a definition is reads, and says it is outside
    /// the repo so the pane showing it offers no save.
    #[test]
    fn a_file_a_language_server_named_reads_and_says_it_is_outside_the_repo() {
        let served = served_repo("allowed");
        let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");

        assert!(
            served.read(&dependency).is_err(),
            "nothing may be read out there before a server has named it"
        );
        served.a_server_named(&dependency);

        let payload = served
            .read(&dependency)
            .expect("expected the file the server named to read");
        assert_eq!(payload.content, "pub fn dep() {}\n");
        assert!(payload.outside_the_repo);
    }

    /// The list is of resolved files, not of the strings that were handed in: a `..` walk off
    /// the named file and a symlink pointing away from it are both refused, while a symlink
    /// onto the named file is the named file and reads.
    #[test]
    fn a_dotdot_or_a_symlink_that_leaves_the_named_file_is_refused() {
        let served = served_repo("resolved");
        let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");
        served.write_outside("registry/secret.txt", "a private key\n");
        served.a_server_named(&dependency);

        let walked = served
            .enclosing
            .join("registry/dep/src/../../secret.txt")
            .display()
            .to_string();
        assert!(
            served.read(&walked).is_err(),
            "a walk off the named file lands somewhere nobody named"
        );

        let pointed_away = served.enclosing.join("registry/dep/src/away.rs");
        std::os::unix::fs::symlink(served.enclosing.join("registry/secret.txt"), &pointed_away)
            .expect("failed to link the fixture");
        assert!(
            served.read(&pointed_away.display().to_string()).is_err(),
            "a link out of the named file is the file it points at, which nobody named"
        );

        let pointed_at_it = served.enclosing.join("registry/dep/src/same.rs");
        std::os::unix::fs::symlink(&dependency, &pointed_at_it)
            .expect("failed to link the fixture");
        assert_eq!(
            served
                .read(&pointed_at_it.display().to_string())
                .expect("a link onto the named file is the named file")
                .content,
            "pub fn dep() {}\n"
        );
    }

    /// Reading a dependency to understand it is the whole point; editing one in place is not.
    /// The write keeps its containment check whatever any server has named.
    #[test]
    fn writing_to_a_file_a_language_server_named_is_still_refused() {
        let served = served_repo("read-only");
        let dependency = served.write_outside("registry/dep/src/lib.rs", "pub fn dep() {}\n");
        served.a_server_named(&dependency);

        let refused = write_session_file(
            &served.state,
            &served.session_id,
            &dependency,
            "pub fn theirs() {}\n",
        );

        assert!(refused.is_err(), "a file outside the repo is read-only");
        assert_eq!(
            fs::read_to_string(&dependency).expect("failed to read the dependency back"),
            "pub fn dep() {}\n",
            "and nothing may have been written to it"
        );
        // The repo's own files are still written the way the file pane writes them.
        write_session_file(&served.state, &served.session_id, "lib.rs", "fn two() {}\n")
            .expect("expected the repo's own file to be written");
        assert_eq!(
            fs::read_to_string(served.repo_path.join("lib.rs")).expect("failed to read back"),
            "fn two() {}\n"
        );
    }

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

    /// A shell and a task board are as much use in a plain folder as in a repo, so a window
    /// opens on one - and everything the review is made of comes back empty rather than as
    /// the failure of a `git status` run where there is no git.
    #[test]
    fn a_folder_that_is_no_repo_opens_with_an_empty_review() {
        let enclosing = std::env::temp_dir().join(format!(
            "moonreview-no-repo-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let folder = enclosing.join("notes");
        fs::create_dir_all(&folder).expect("failed to create the fixture folder");
        fs::write(folder.join("todo.md"), "- write it down\n").expect("failed to write a file");

        let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
        let session_id = open_session(
            &state,
            OpenSessionRequest {
                repo_path: folder.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
        )
        .expect("a folder that is no repo should still open")
        .session_id;

        let payload = session_state(&state, &session_id).expect("the state should still read");
        assert_eq!(payload.repo_name, "notes");
        assert_eq!(payload.branch_name, None);
        assert!(payload.hunks.is_empty(), "no repo means nothing to review");
        assert!(payload.commits.is_empty());
        assert!(
            !payload.read_only,
            "the files of the folder are still there to edit"
        );

        let hub = session_submodules(&state, &session_id).expect("the hub should still read");
        assert_eq!(hub.root.changed_files, 0);
        assert!(hub.submodules.is_empty());

        // And a `git init` in that same folder is what turns the review on.
        run_git_no_output(&folder, &["init"]).expect("failed to init the folder");
        let payload = session_state(&state, &session_id).expect("the state should read again");
        assert!(
            payload.branch_name.is_some(),
            "a folder that has become a repo is one with a branch"
        );

        fs::remove_dir_all(&enclosing).expect("failed to remove test directory");
    }
}
