//! A Codex agent's inline visualizations, shown beside the terminal it runs in.
//!
//! In the Codex app, a model that wants to show something writes an HTML fragment into its
//! thread's folder and says `::codex-inline-vis{file="name.html"}` on a line of its answer; the
//! app draws the fragment inline. Moon does the same thing for a Codex in one of its terminals,
//! without the model knowing it is anywhere else:
//!
//! - [`codex_launch`] starts Codex with the instructions that teach it the directive, which
//!   the Codex CLI does not ship, and lets its sandbox write the folder.
//! - [`rollout`] finds the rollout the terminal's Codex is writing and reads each answer in it
//!   for directives, by Codex's rules - see [`directives`].
//! - [`page`] builds the page a fragment is shown on, the way Codex builds its viewer.
//! - [`on_tasks`] keeps what a task's run announced in the task's folder, listed on the task.
//!
//! The window asks the server which visualizations its terminals have announced, and opens
//! each in a pane - see `crate::native::visualizations`.

pub(crate) mod codex_launch;
pub(crate) mod directives;
pub(crate) mod on_tasks;
mod open_files;
pub(crate) mod page;
pub(crate) mod rollout;
pub(crate) mod routes;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One visualization a terminal's agent has announced, as the window hears of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VisualizationView {
    /// The terminal whose agent announced it, which is what the pane opens beside.
    pub(crate) terminal_id: String,
    /// The fragment, on the server's machine.
    pub(crate) fragment_path: String,
    /// How many times it has been announced. A directive naming a file already shown is the
    /// agent showing it again, which brings its pane forward.
    pub(crate) announced: u64,
    /// When the fragment was last written, in milliseconds since the epoch. An agent that
    /// rewrites the file has changed what the pane should show.
    pub(crate) modified_unix_ms: u64,
}

/// Where Codex keeps its sessions and visualizations: `$CODEX_HOME`, else `~/.codex` - how
/// Codex itself finds it.
pub(crate) fn codex_home() -> PathBuf {
    match std::env::var_os("CODEX_HOME") {
        Some(codex_home) if !codex_home.is_empty() => PathBuf::from(codex_home),
        _ => PathBuf::from(std::env::var_os("HOME").expect("HOME is set for a user's process"))
            .join(".codex"),
    }
}
