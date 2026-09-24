//! moonreview, as one library behind one executable.
//!
//! `moon` opens a window on one of three things - a review, the task board, or a shell -
//! which are three [`cli::Frame`]s over the same everything else; and it reaches the windows
//! that are already open, which is what `moon open` does. `src/bin/moon.rs` is a single call
//! to [`run`].

// The modules gated off wasm32 are the server side - the HTTP server, the shells, the socket a
// `moon open` reaches a window through, and the git, searches, agents and scripts only they
// run - which the build for the browser reaches over HTTP instead. See `web`. The modules
// left in keep their server halves in submodules gated the same way.
#[cfg(not(target_arch = "wasm32"))]
mod agent;
mod agent_sessions;
mod api;
pub(crate) mod attention;
mod backend;
mod cli;
mod comments;
mod commit_suggestion;
mod committing;
#[cfg(not(target_arch = "wasm32"))]
mod extensions;
#[cfg(not(target_arch = "wasm32"))]
mod git;
#[cfg(not(target_arch = "wasm32"))]
mod instances;
#[cfg(not(target_arch = "wasm32"))]
mod lsp;
mod moontasks;
#[cfg(not(target_arch = "wasm32"))]
mod moved_hunks;
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod pass_keys;
mod project;
#[cfg(not(target_arch = "wasm32"))]
mod search;
#[cfg(not(target_arch = "wasm32"))]
mod server;
#[cfg(test)]
mod server_tests;
#[cfg(not(target_arch = "wasm32"))]
mod service;
mod settings;
#[cfg(not(target_arch = "wasm32"))]
mod shell_locale;
#[cfg(not(target_arch = "wasm32"))]
mod shell_path;
#[cfg(not(target_arch = "wasm32"))]
mod terminal;
mod visualizations;
#[cfg(target_arch = "wasm32")]
mod web;

/// Run `moon`: the command line decides whether that is a window, the server behind one, or
/// a word to a window that is already open.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> anyhow::Result<()> {
    pass_keys::handoff::take_from_environment();
    cli::run()
}
