//! The window: one of them per process, and - when it is reviewing the machine it runs on -
//! the review server in the same process and the same executable.

pub(crate) mod app;
pub(crate) mod bindings;
pub(crate) mod blame;
pub(crate) mod board;
pub(crate) mod code_actions;
pub(crate) mod commit_pane;
#[cfg(test)]
mod commit_pane_request_tests;
#[cfg(test)]
mod commit_pane_tests;
pub(crate) mod completing;
pub(crate) mod definition;
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
pub(crate) mod diagnostics;
// An extension's script runs programs and reads folders on the project's machine, which a
// browser's window is never on.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod extension_pane;
pub(crate) mod file_pane;
pub(crate) mod find;
pub(crate) mod fonts;
pub(crate) mod formatting;
pub(crate) mod hover;
pub(crate) mod language_source;
// Launchers, and the programs a window starts, are this machine's: a browser's window is a page.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod launchers;
pub(crate) mod logos;
pub(crate) mod lsp_document;
pub(crate) mod menu;
pub(crate) mod messages;
pub(crate) mod model;
// `moon open` reaches a window through a socket on this machine, which a browser has none of.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod open_from_shell;
#[cfg(not(target_arch = "wasm32"))]
mod open_in_web;
#[cfg(not(target_arch = "wasm32"))]
mod pass_key;
pub(crate) mod palette;
pub(crate) mod places;
pub(crate) mod panes;
#[cfg(not(target_arch = "wasm32"))]
mod programs;
pub(crate) mod project_pane;
pub(crate) mod renaming;
pub(crate) mod review;
pub(crate) mod signature;
pub(crate) mod start_pane;
pub(crate) mod status_bar;
pub(crate) mod submodules;
pub(crate) mod tasks;
#[cfg(target_os = "macos")]
mod text_without_a_key;
pub(crate) mod theme;
#[cfg(test)]
pub(crate) mod ui_tests;
pub(crate) mod visualizations;
pub(crate) mod webview_pane;
pub(crate) mod widgets;
pub(crate) mod work_log;
pub(crate) mod workspace;
pub(crate) mod workspace_edits;
pub(crate) mod workspace_color;

use std::sync::Arc;

use crate::{api::OpenSessionRequest, backend::Backend};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use desktop::{launch_local, launch_prompt, launch_remote, run};

pub(crate) struct Launch {
    pub(crate) backend: Arc<dyn Backend>,
    /// The review to open on startup. `None` means ask, which is what a remote connection
    /// does when it was given an address but no path.
    pub(crate) open: Option<OpenSessionRequest>,
    /// What the window opens on: which of the three executables this is.
    pub(crate) frame: crate::cli::Frame,
}
