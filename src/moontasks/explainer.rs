//! The change explainer: a short PDF about everything that is changed in the repo right now,
//! written by an agent for whoever has to review it.
//!
//! `explain` on a card runs the agent the review's selector is set to the way a review
//! comment is handed to one - see [`crate::comments::spawn_comment_dispatch`]: headless, with
//! nobody to ask, and with a prompt that is the whole of the job - a few sentences, which is
//! the length agents do best on. The agent reads the diff, writes the Typst, compiles it, and
//! opens the PDF; nothing here runs git, `typst` or `open`, and the agent names the files.
//!
//! Unlike a comment's dispatch it runs in a shell of the task, the way a commit run does - see
//! [`crate::committing::start_commit_run`] - so it is a run on the card like any other: click
//! it to watch the agent print, `stop` to end it, the close mark to take it off. The shell
//! outlives the agent, so what it printed is there to read once it is over, and the prompt is
//! a file in the task folder beside the files the agent writes, for running it again by hand.

// Running the agent is the server's: the window in a browser only asks for it.
#[cfg(not(target_arch = "wasm32"))]
mod running;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use running::start_explanation;

use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

/// `explain` on a card asked for: which agent writes it.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct ExplainRequest {
    pub(crate) agent: AgentKind,
}
