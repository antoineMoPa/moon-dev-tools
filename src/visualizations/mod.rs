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

// Finding and serving visualizations is the server's: the window in a browser only lists and
// shows what the server has found.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod codex_launch;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod directives;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod on_tasks;
#[cfg(not(target_arch = "wasm32"))]
mod open_files;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod page;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod rollout;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod routes;

use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
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

/// Every visualization the server's terminals have announced, as the window asks for them.
#[derive(Serialize, Deserialize)]
pub(crate) struct VisualizationList {
    pub(crate) visualizations: Vec<VisualizationView>,
}

/// The page one visualization is shown on - see `page`.
#[derive(Serialize, Deserialize)]
pub(crate) struct VisualizationPage {
    pub(crate) html: String,
}

/// What a visualization is called: its file's stem, with dashes read as spaces.
pub(crate) fn title_of(fragment_path: &Path) -> String {
    fragment_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Visualization")
        .replace('-', " ")
}

/// Where Codex keeps its sessions and visualizations: `$CODEX_HOME`, else `~/.codex` - how
/// Codex itself finds it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn codex_home() -> PathBuf {
    match std::env::var_os("CODEX_HOME") {
        Some(codex_home) if !codex_home.is_empty() => PathBuf::from(codex_home),
        _ => PathBuf::from(std::env::var_os("HOME").expect("HOME is set for a user's process"))
            .join(".codex"),
    }
}
