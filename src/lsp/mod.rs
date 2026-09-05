//! Language servers for the files the editor has open: where a definition is, and what can
//! be typed next.
//!
//! A server has to run where the files are. A session may be reviewing a repo on another
//! machine, so a server started in the window would be reading the wrong disk - or no disk
//! at all. So this lives repo-side, exactly as the shells in [`crate::terminal`] do: the
//! registry hangs off [`crate::api::AppState`], the window reaches it through
//! [`crate::backend::Backend`], and a `--remote` session gets the same answers over HTTP as
//! a local one gets by calling straight through.
//!
//! The client itself is [`moon_lsp`], which knows nothing about reviews: it takes a
//! [`Workspace`] - a repo root and an opaque key the servers are held under - and answers
//! about files in it. This module is the layer that turns a session into one of those, and
//! [`routes`] is the same thing again over HTTP.

pub(crate) mod routes;

use std::path::{Path, PathBuf};

use anyhow::Result;
use moon_lsp::Workspace;

use crate::api::{AppState, LspCompletion, LspLocation, LspPosition, LspStatus, LspWork};

/// The servers are keyed per review session rather than per repo. Two sessions on the same
/// repo get one each: a session is what a window is looking at, and closing it takes its
/// servers with it rather than leaving another window's indexing half done.
fn workspace<'a>(session_id: &'a str, repo_root: &'a Path) -> Workspace<'a> {
    Workspace {
        key: session_id,
        root: repo_root,
    }
}

fn repo_root(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

/// Whether a language server is behind this file, and whether it has finished starting.
///
/// The session is not looked up: the answer is about what is running rather than about what
/// is on disk, so a pane asking as it draws costs nothing but a map read.
pub(crate) fn status(state: &AppState, session_id: &str, file_path: &str) -> Result<LspStatus> {
    Ok(state.lsp.status(session_id, file_path))
}

/// What every language server running for this session is doing right now, for the status
/// bar along the bottom of the window.
pub(crate) fn working(state: &AppState, session_id: &str) -> Vec<LspWork> {
    state.lsp.working(session_id)
}

/// Tell the server a file is open and what is in it, starting the server if this is the
/// first file of its language.
pub(crate) fn did_open(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    text: &str,
) -> Result<()> {
    let repo_root = repo_root(state, session_id)?;
    state
        .lsp
        .did_open(&workspace(session_id, &repo_root), file_path, text)
}

/// The whole text again, as it stands.
///
/// **The caller debounces.** On a `--remote` session this is a network round trip, and one
/// per keystroke would flood it - the editor sends this after the typing has paused, not
/// while it is going on.
pub(crate) fn did_change(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    text: &str,
) -> Result<()> {
    let repo_root = repo_root(state, session_id)?;
    state
        .lsp
        .did_change(&workspace(session_id, &repo_root), file_path, text)
}

/// Tell the server the window is done with a file.
pub(crate) fn did_close(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    let repo_root = repo_root(state, session_id)?;
    state
        .lsp
        .did_close(&workspace(session_id, &repo_root), file_path)
}

/// Where the name at this place is defined.
pub(crate) fn definition(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspLocation>> {
    let repo_root = repo_root(state, session_id)?;
    state
        .lsp
        .definition(&workspace(session_id, &repo_root), file_path, at)
}

/// What could be typed at this place.
pub(crate) fn completion(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspCompletion>> {
    let repo_root = repo_root(state, session_id)?;
    state
        .lsp
        .completion(&workspace(session_id, &repo_root), file_path, at)
}
