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

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

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

/// How many files outside the repo one session remembers having been told about.
///
/// Every ⌘-click into a dependency adds one, so the list grows with the reading rather than
/// with anything an attacker controls; a session that spends an afternoon in `~/.cargo` still
/// has a bounded list, and the oldest jump falling off it only means that tab has to be
/// reopened by clicking through to it again.
const FILES_REMEMBERED: usize = 512;

/// The files outside the repo that a language server has named in this session, and the only
/// ones outside it that may be read.
///
/// A definition in a Rust project lands in `~/.cargo/registry` or in the standard library as
/// often as it lands in the repo, so the pane has to be able to open what the jump landed on.
/// But the repo side of a `--remote` session is a server on somebody else's machine, and a
/// read that took whatever absolute path it was handed would be arbitrary file read on that
/// machine - `~/.ssh/id_rsa` for the asking. So the only paths outside the repo that can be
/// read are the ones a server itself named while answering a question the person asked, and
/// each one is remembered here on its way back to the window.
///
/// Resolved paths, never the strings that were handed in: `..` segments and symlinks are how
/// a list like this gets walked around, so both what is remembered and what is later asked
/// for go through [`real_path`] before they are ever compared.
#[derive(Default)]
pub(crate) struct FilesNamedOutsideTheRepo {
    /// Oldest first, which is what a full list drops.
    remembered: VecDeque<PathBuf>,
}

impl FilesNamedOutsideTheRepo {
    /// Remember a file a server named, if it named one outside the repo at all. A path
    /// inside the repo is relative and is read the way every other file of the repo is, so
    /// there is nothing to remember about it.
    fn remember(&mut self, file_path: &str) {
        let Some(real_path) = real_path(file_path) else {
            return;
        };
        if self.remembered.contains(&real_path) {
            return;
        }
        if self.remembered.len() == FILES_REMEMBERED {
            self.remembered.pop_front();
        }
        self.remembered.push_back(real_path);
    }

    /// The file on disk this path names, when it is one a server named. `None` for everything
    /// else, which is every path that has to go on being refused - see
    /// [`crate::git::read_repo_file`].
    pub(crate) fn allows(&self, file_path: &str) -> Option<PathBuf> {
        let real_path = real_path(file_path)?;
        self.remembered.contains(&real_path).then_some(real_path)
    }
}

/// The file a path really names, with every `..` segment and every symlink already followed.
///
/// Only an absolute path is resolved. A relative one is a path in the repo - which is the
/// other read route entirely - and resolving it here would resolve it against whatever
/// directory this process happens to be running in, which is not something either side of
/// the comparison should depend on.
fn real_path(file_path: &str) -> Option<PathBuf> {
    let path = Path::new(file_path);
    if !path.is_absolute() {
        return None;
    }
    path.canonicalize().ok()
}

/// Remember every file outside the repo that this answer names, so the pane can open what the
/// jump landed on. Called with a server's answer on its way back to the window, which is the
/// only moment anything legitimately names a file outside the repo.
pub(crate) fn remember_files_named(
    state: &AppState,
    session_id: &str,
    locations: &[LspLocation],
) -> Result<()> {
    crate::api::with_session(state, session_id, |session| {
        for location in locations {
            session
                .files_named_outside_the_repo
                .remember(&location.file_path);
        }
        Ok(())
    })
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
///
/// The answer is also what says which files outside the repo this session may read: a
/// definition in a dependency or in the standard library is a file the pane has to be able to
/// open, and a server answering the person's own question is the only thing that legitimately
/// names one - see [`FilesNamedOutsideTheRepo`].
pub(crate) fn definition(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspLocation>> {
    let repo_root = repo_root(state, session_id)?;
    let locations = state
        .lsp
        .definition(&workspace(session_id, &repo_root), file_path, at)?;
    remember_files_named(state, session_id, &locations)?;
    Ok(locations)
}

/// The characters the server behind this file said open a completion list on their own.
///
/// Answered without touching a server: the list came out of that server's `initialize`
/// reply and has been sitting on it ever since, so this is a read rather than a question
/// anything waits on. Empty for a file nothing serves and for a server that has not started
/// yet - the window asks once its file is `Ready`, which is when there is a reply to read.
pub(crate) fn trigger_characters(
    state: &AppState,
    session_id: &str,
    file_path: &str,
) -> Vec<char> {
    state.lsp.trigger_characters(session_id, file_path)
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
