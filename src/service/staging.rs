//! What is staged in a review's repo: hunks, selections and files staged, unstaged and
//! discarded.

use anyhow::Result;

use crate::{
    api::AppState,
    git::{apply_patch, build_partial_patch_from_selection, run_git_no_output},
};

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
