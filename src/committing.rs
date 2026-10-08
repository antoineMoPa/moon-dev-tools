//! Committing - commits what the review has staged, and pushes it.
//!
//! Both run as `git` in a pty rather than as a captured process: commits here are signed, and
//! the only pinentry many machines have is a terminal one, so the passphrase prompt needs a
//! terminal to appear on. See [`crate::terminal::TerminalProgram::LoginShell`].

// Reading the repo and running in it are the server's: the window in a browser only has the
// types it is told about them in.
#[cfg(not(target_arch = "wasm32"))]
mod repo_side;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use repo_side::{
    commit_run_outcome, commit_run_outcome_personal, commit_state, start_commit_run,
    start_commit_run_personal,
};

use serde::{Deserialize, Serialize};

use crate::api::FileChangeKind;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct StagedFile {
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
}

/// What the commit pane needs to know about the repo: what a commit would take in, and where
/// a push would send it.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub(crate) struct CommitState {
    /// `None` on a detached HEAD, which is the one state neither action works from.
    pub(crate) branch_name: Option<String>,
    /// Whether that branch is the repo's default one, which a pull request is opened *from*
    /// never: it would have nothing to compare against.
    pub(crate) on_default_branch: bool,
    /// The branch this one tracks, e.g. `origin/main`, once it has one.
    pub(crate) upstream_ref: Option<String>,
    /// Where a plain `git push` would send this branch, when git can tell from its config:
    /// `None` with no upstream, and under `push.default=simple` when the upstream is not
    /// named like the branch.
    pub(crate) push_ref: Option<String>,
    /// Commits this branch has that its upstream does not, and the other way round. Both zero
    /// when there is no upstream.
    pub(crate) ahead: usize,
    pub(crate) behind: usize,
    pub(crate) staged_files: Vec<StagedFile>,
    /// How many files have changes that are not staged, untracked ones included. What "stage
    /// all" would take in.
    pub(crate) unstaged_count: usize,
    /// Whether `gh` is installed on this machine, which is what the pull request button needs:
    /// without it there is nothing to offer.
    pub(crate) gh_installed: bool,
    /// Whether `opencode` is installed, which is what writes the suggested message. Without it
    /// the pane never asks for one, rather than asking and showing what went wrong.
    pub(crate) opencode_installed: bool,
}

/// What the pane can ask for.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "action", rename_all = "lowercase")]
pub(crate) enum CommitAction {
    Commit {
        message: String,
    },
    /// Push the branch. With a message, what is staged is committed with it first, in the
    /// same shell, and nothing is pushed if the commit fails.
    Push {
        message: Option<String>,
    },
    /// Open the pull request for the branch, in the browser, through `gh`. With a message, what
    /// is staged is committed and pushed first; without one, `pushes_first` says whether the
    /// branch has commits that still need pushing.
    OpenPr {
        message: Option<String>,
        pushes_first: bool,
    },
}

impl CommitAction {
    /// The message of the commit this action makes, when it makes one.
    pub(crate) fn commit_message(&self) -> Option<&str> {
        match self {
            CommitAction::Commit { message } => Some(message),
            CommitAction::Push { message } | CommitAction::OpenPr { message, .. } => {
                message.as_deref()
            }
        }
    }
}
