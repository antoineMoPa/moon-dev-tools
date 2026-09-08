//! moonreview, as one library behind one executable.
//!
//! `moon` opens a window on one of three things - a review, the task board, or a shell -
//! which are three [`cli::Frame`]s over the same everything else; and it reaches the windows
//! that are already open, which is what `moon open` does. `src/bin/moon.rs` is a single call
//! to [`run`].

mod agent;
mod agent_sessions;
mod api;
mod backend;
mod cli;
mod comments;
mod commit_suggestion;
mod committing;
mod git;
mod instances;
mod lsp;
mod moontasks;
mod moved_hunks;
mod native;
mod project;
mod search;
mod server;
#[cfg(test)]
mod server_tests;
mod service;
mod settings;
mod shell_locale;
mod shell_path;
mod terminal;

use anyhow::Result;

/// Run `moon`: the command line decides whether that is a window, the server behind one, or
/// a word to a window that is already open.
pub fn run() -> Result<()> {
    cli::run()
}
