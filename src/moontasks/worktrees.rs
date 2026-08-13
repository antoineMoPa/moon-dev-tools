//! A task's own checkout, and putting it back in the repo to be looked at.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    api::AppState,
    git,
    moontasks::{
        TaskReviewPayload, TaskWorktreeView,
        store::{self, TaskMetadata, TaskWorktree},
    },
};

/// The prefix every task's branch is named with, so a `git branch` listing says which of them
/// came from the board and which card each one is.
const BRANCH_PREFIX: &str = "moontask/";

/// Where a task's shells and agents run: its own checkout once it has one, and the repo the
/// board belongs to until then.
pub(crate) fn work_dir_of(repo_path: &Path, metadata: &TaskMetadata) -> PathBuf {
    match &metadata.worktree {
        Some(worktree) => PathBuf::from(&worktree.path),
        None => repo_path.to_path_buf(),
    }
}

/// What the card draws, which needs one `git status` the metadata cannot answer.
pub(crate) fn view_of(metadata: &TaskMetadata) -> Option<TaskWorktreeView> {
    let worktree = metadata.worktree.as_ref()?;
    Some(TaskWorktreeView {
        path: worktree.path.clone(),
        branch: worktree.branch.clone(),
        // A checkout that has gone missing from under us reads as dirty rather than as an
        // error: the card still draws, and the review step is where that is refused with
        // something to read.
        is_clean: git::is_clean(Path::new(&worktree.path)).unwrap_or(false),
    })
}

/// Where a task's checkout is made: outside the repo, under the directory the settings live in.
fn path_for(repo_path: &Path, task_id: &str) -> Result<PathBuf> {
    let root =
        crate::settings::worktrees_root().context("no home directory to keep task worktrees in")?;
    let repo_name = repo_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    Ok(root.join(repo_name).join(task_id))
}

/// The branch a task's work goes on: the task id, whole, behind the board's prefix.
fn branch_for(task_id: &str) -> String {
    format!("{BRANCH_PREFIX}{task_id}")
}

/// Give a task a checkout of its own, on a new branch off the repo's HEAD.
pub(crate) fn create(
    state: &AppState,
    session_id: &str,
    task_id: &str,
) -> Result<TaskWorktreeView> {
    let repo_path = super::service::repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    if let Some(worktree) = &metadata.worktree {
        bail!("this task already works in {}", worktree.path);
    }

    let branch = branch_for(task_id);
    // The repo is a working tree too, and git will not have one branch in two of them. This is
    // the ordinary case after a review — the branch was put in the repo so it could be QA'd —
    // so it says what to do rather than passing git's word for it up.
    if git::current_branch_name(&repo_path)?.as_deref() == Some(branch.as_str()) {
        bail!(
            "the repo is on {branch} — switch it to another branch first, and this task can \
             have a checkout of its own again"
        );
    }

    let path = path_for(&repo_path, task_id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let base_commit = git::add_worktree(&repo_path, &path, &branch)?;

    metadata.worktree = Some(TaskWorktree {
        path: path.display().to_string(),
        branch,
        base_commit: Some(base_commit),
    });
    store::write_task(&repo_path, task_id, &metadata)?;
    // A checkout that has just been made off HEAD has nothing uncommitted in it, but the view
    // is read the same way the card reads it rather than assumed.
    view_of(&metadata).context("expected the worktree just written")
}

/// Give a task's checkout back, leaving its branch where it is.
pub(crate) fn discard(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    force: bool,
) -> Result<()> {
    let repo_path = super::service::repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    let Some(worktree) = metadata.worktree.clone() else {
        bail!("this task has no worktree");
    };

    git::remove_worktree(&repo_path, Path::new(&worktree.path), force)?;
    metadata.worktree = None;
    store::write_task(&repo_path, task_id, &metadata)
}

/// Let go of a task's checkout without the caller having to care whether it had one.
pub(crate) fn discard_quietly(repo_path: &Path, metadata: &mut TaskMetadata) {
    let Some(worktree) = metadata.worktree.take() else {
        return;
    };
    if let Err(error) = git::remove_worktree(repo_path, Path::new(&worktree.path), true) {
        eprintln!("[moonreview] could not remove {}: {error}", worktree.path);
    }
}

/// Put a task's branch in the repo, so its work can be QA'd where everything is already built.
pub(crate) fn review(
    state: &AppState,
    session_id: &str,
    task_id: &str,
) -> Result<TaskReviewPayload> {
    let repo_path = super::service::repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let Some(worktree) = metadata.worktree.clone() else {
        return Ok(TaskReviewPayload {
            repo_path: repo_path.display().to_string(),
            title: metadata.title.clone(),
            base_branch: None,
        });
    };

    let worktree_path = Path::new(&worktree.path);
    if !worktree_path.is_dir() {
        bail!("{} is not there any more", worktree.path);
    }
    if !git::is_clean(worktree_path)? {
        bail!(
            "commit the work in {} first — only what is on {} can be checked out here",
            worktree.path,
            worktree.branch
        );
    }
    if !git::is_clean(&repo_path)? {
        bail!("the repo has local changes — commit or stash them before reviewing this task");
    }
    // A branch with nothing on it is a card whose work never happened. Reviewing it would show
    // an empty diff and cost the card its checkout on the way, so it is refused while there is
    // still somewhere for the work to be done.
    if let Some(base) = &worktree.base_commit
        && git::commits_ahead(&repo_path, base, &worktree.branch)? == 0
    {
        bail!(
            "there is nothing on {} yet — the work has to be committed there before it can be \
             looked at here",
            worktree.branch
        );
    }

    // The checkout goes before the branch can come here: git will not have one branch in two
    // working trees at once, and the worktree has done its job — the work is on the branch, and
    // the branch is about to be in the repo. Sending the card back for more work makes another
    // checkout on the same branch, which `add_worktree` handles.
    git::remove_worktree(&repo_path, worktree_path, false)?;
    metadata.worktree = None;
    store::write_task(&repo_path, task_id, &metadata)?;

    git::checkout_branch(&repo_path, &worktree.branch)?;

    Ok(TaskReviewPayload {
        repo_path: repo_path.display().to_string(),
        title: metadata.title.clone(),
        // Once the branch is checked out the working tree is clean, so a review of what is
        // uncommitted would be empty. What is wanted is everything the card added, which is the
        // branch read against the commit it grew from.
        base_branch: worktree.base_commit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A branch names exactly one card, which two cards called the same thing could not do if
    /// the branch were named after the title.
    #[test]
    fn a_branch_names_the_card_it_belongs_to() {
        let one = branch_for("fix-the-login-page-6f9c1e2a-1111-4222-8333-444455556666");
        let two = branch_for("fix-the-login-page-aaaaaaaa-1111-4222-8333-444455556666");

        assert!(one.starts_with("moontask/fix-the-login-page-"), "got {one}");
        assert_ne!(one, two, "two cards with one title need two branches");
    }

    #[test]
    fn a_task_without_a_worktree_works_in_the_repo() {
        let metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: store::ColumnId::new("todo"),
            created_at_unix: 0,
            position: 0,
            tags: Vec::new(),
            worktree: None,
            resources: Vec::new(),
        };

        assert_eq!(
            work_dir_of(Path::new("/repo"), &metadata),
            PathBuf::from("/repo")
        );
    }

    #[test]
    fn a_task_with_a_worktree_works_in_it() {
        let metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: store::ColumnId::new("todo"),
            created_at_unix: 0,
            position: 0,
            tags: Vec::new(),
            worktree: Some(TaskWorktree {
                path: "/elsewhere/fix-the-login-page".to_string(),
                branch: "moontask/fix-the-login-page".to_string(),
                base_commit: None,
            }),
            resources: Vec::new(),
        };

        assert_eq!(
            work_dir_of(Path::new("/repo"), &metadata),
            PathBuf::from("/elsewhere/fix-the-login-page")
        );
    }

    /// A test that made a worktree must never leave one in a real home directory.
    #[test]
    fn worktrees_are_kept_out_of_the_home_directory_under_test() {
        let path = path_for(Path::new("/repo"), "task").expect("expected a path");

        assert!(path.starts_with(std::env::temp_dir()), "got {path:?}");
        assert!(path.ends_with("repo/task"), "got {path:?}");
    }
}
