//! A commit message written for the pane by an agent, from what is staged.
//!
//! The same idea as the `commitwriter` command: hand the staged diff to `opencode`, ask for a
//! conventional-commit subject and a short paragraph, and print them. Here they arrive in the
//! commit pane instead, under the message box, for the `[use]` button beside them to put in it.
//!
//! Only the staged changes go in the prompt. The commit the pane is about to make is what is
//! staged, so that is what the message is written from - unstaged work belongs to the commit
//! after this one.

// Running `opencode` is the server's: the window in a browser is only handed what it wrote.
#[cfg(not(target_arch = "wasm32"))]
mod writing;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use writing::suggest_commit_message;

use serde::{Deserialize, Serialize};

/// A message written for a commit that has not been made yet.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct CommitSuggestion {
    /// One conventional commit subject, the first line of the message.
    pub(crate) subject: String,
    /// A sentence or two under it, which is also what a pull request would say.
    pub(crate) paragraph: String,
}

impl CommitSuggestion {
    /// The two of them as one commit message: subject, blank line, paragraph - which is what
    /// `[use]` puts in the message box.
    pub(crate) fn as_message(&self) -> String {
        format!("{}\n\n{}", self.subject, self.paragraph)
    }
}
