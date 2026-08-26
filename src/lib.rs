//! moonreview, as one library behind three executables.
//!
//! The three differ only in what the window opens on — a review, the task board, or a shell —
//! so they are three [`Frame`]s over the same everything else. `src/bin` holds one file per
//! executable, and each is a single call to [`run`].

mod agent;
mod agent_sessions;
mod api;
#[cfg(feature = "native")]
mod backend;
mod cli;
mod comments;
mod committing;
mod file_search;
mod git;
mod moontasks;
mod moved_hunks;
#[cfg(feature = "native")]
mod native;
mod reviewed_cache;
mod server;
#[cfg(test)]
mod server_tests;
mod service;
mod settings;
mod shell_path;
mod terminal;

use anyhow::Result;

pub use cli::Frame;

/// Run one of the three executables. `frame` is the whole difference between them.
pub fn run(frame: Frame) -> Result<()> {
    cli::run(frame)
}
