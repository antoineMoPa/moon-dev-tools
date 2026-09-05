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
//! The parts: [`languages`] says which server serves which file, [`process`] runs one and
//! carries the JSON-RPC, [`framing`] is the envelope that goes over its stdio, and
//! [`protocol`] is the messages and - the part worth being careful about - the one place a
//! position is converted between what the editor counts and what the server counts.

pub(crate) mod framing;
pub(crate) mod languages;
pub(crate) mod process;
pub(crate) mod protocol;
pub(crate) mod routes;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow, bail};

use crate::api::{AppState, LspCompletion, LspLocation, LspPosition, LspStatus, LspWork};
use languages::{ExtensionSpec, ServerSpec};
use process::LanguageServer;

/// One server, per review session and per server in the table. Two sessions on the same
/// repo get one each: a session is what a window is looking at, and closing it takes its
/// servers with it rather than leaving another window's indexing half done.
type ServerKey = (String, &'static str);

/// The language servers this machine is running for its reviews.
///
/// Held as an `Arc` on [`AppState`] beside [`crate::terminal::TerminalRegistry`], and built
/// once in [`crate::server::build_state`].
#[derive(Default)]
pub(crate) struct LspRegistry {
    servers: Mutex<HashMap<ServerKey, Arc<LanguageServer>>>,
    /// The servers that are on PATH and would not start, which is not the same as not being
    /// installed at all: a `rust-analyzer` on PATH that is really a rustup shim for a
    /// component nobody added exits the moment it is spoken to, which is how this was found.
    /// The first attempt says why; after it, the file reads as having no server rather than
    /// as one that is forever starting.
    would_not_start: Mutex<HashSet<ServerKey>>,
}

impl LspRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn running(&self, key: &ServerKey) -> Option<Arc<LanguageServer>> {
        self.servers.lock().unwrap().get(key).cloned()
    }

    /// What one running server agreed to count columns in. Only a test asks: the rest of
    /// the module has the server in hand by the time it needs to know.
    #[cfg(test)]
    pub(crate) fn agreed_encoding(
        &self,
        session_id: &str,
        server_name: &'static str,
    ) -> Option<process::PositionEncoding> {
        self.running(&(session_id.to_string(), server_name))
            .map(|server| server.encoding())
    }

    /// Every server running for one session, with the name it is known by. What the status
    /// bar is answered out of: the question is about the session rather than about one file,
    /// because a window with a Rust file and a TypeScript file open is waiting on both.
    fn running_for(&self, session_id: &str) -> Vec<(&'static str, Arc<LanguageServer>)> {
        self.servers
            .lock()
            .unwrap()
            .iter()
            .filter(|((session, _), _)| session == session_id)
            .map(|((_, name), server)| (*name, Arc::clone(server)))
            .collect()
    }

    /// Whether this server has already been tried and would not start.
    fn gave_up_on(&self, key: &ServerKey) -> bool {
        self.would_not_start.lock().unwrap().contains(key)
    }

    fn give_up_on(&self, key: &ServerKey) {
        self.would_not_start.lock().unwrap().insert(key.clone());
    }

    /// The server for one session and one language, started if it is not running yet.
    ///
    /// Starting blocks until the server has answered `initialize`, so the lock on the map
    /// is given up for the wait: a pane asking for a file's status while a server comes up
    /// must not wait on it. Two callers starting the same server at once is therefore
    /// possible, and the loser's server is shut down again as it is dropped.
    fn ensure(
        &self,
        key: &ServerKey,
        spec: &'static ServerSpec,
        repo_root: &Path,
    ) -> Result<Arc<LanguageServer>> {
        if let Some(running) = self.running(key) {
            return Ok(running);
        }

        let started = Arc::new(LanguageServer::start(spec, repo_root)?);
        let mut servers = self.servers.lock().unwrap();
        Ok(Arc::clone(servers.entry(key.clone()).or_insert(started)))
    }
}

/// What to tell a pane about a file before any running server is looked at.
///
/// Every way of having no server reads the same to the person: an extension nothing in the
/// table serves, a server that is in the table but is not installed on this machine, and
/// one that is installed and would not start. Anything else has a server behind it, which
/// has at the least started.
fn status_without_a_server(
    language: Option<&ExtensionSpec>,
    has_a_server_to_start: bool,
) -> LspStatus {
    match (language, has_a_server_to_start) {
        (Some(_), true) => LspStatus::Starting,
        _ => LspStatus::Unavailable,
    }
}

/// The row for a file whose server is installed here. `None` covers both of the
/// [`LspStatus::Unavailable`] cases.
fn served_language(file_path: &str) -> Option<&'static ExtensionSpec> {
    let language = languages::for_file(file_path)?;
    languages::installed_at(language.server.command)?;
    Some(language)
}

fn repo_root(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

/// Whether a language server is behind this file, and whether it has finished starting.
pub(crate) fn status(state: &AppState, session_id: &str, file_path: &str) -> Result<LspStatus> {
    let language = languages::for_file(file_path);
    let to_start = language.filter(|language| {
        let key = (session_id.to_string(), language.server.name);
        languages::installed_at(language.server.command).is_some() && !state.lsp.gave_up_on(&key)
    });
    let Some(language) = to_start else {
        return Ok(status_without_a_server(language, false));
    };

    let key = (session_id.to_string(), language.server.name);
    match state.lsp.running(&key) {
        // Nothing started yet, and something to start: the file has a server behind it and
        // it is not answering questions about this file yet, which is what starting is.
        None => Ok(LspStatus::Starting),
        Some(server) if server.is_ready() => Ok(LspStatus::Ready),
        Some(_) => Ok(LspStatus::Starting),
    }
}

/// What every language server running for this session is doing right now.
///
/// Empty means nothing is working: either no server has started for this session, or every
/// one of them has finished what it announced. That is the ordinary state, and the status
/// bar reads it as "nothing to wait for" rather than as an answer it could not get.
///
/// A server that is starting but has announced nothing yet is not in here either. It has
/// said nothing about itself, and inventing a line for it would be the window guessing at
/// what a server is doing - which is the whole thing this exists not to do.
pub(crate) fn working(state: &AppState, session_id: &str) -> Vec<LspWork> {
    let mut working: Vec<LspWork> = state
        .lsp
        .running_for(session_id)
        .into_iter()
        .filter_map(|(name, server)| {
            let doing = server.working()?;
            Some(LspWork {
                server: name.to_string(),
                title: doing.title,
                detail: doing.detail,
                percentage: doing.percentage,
            })
        })
        .collect();
    // By server name, so a window with two of them running does not have its bar swapping
    // between them as a hash map is walked in whatever order it feels like.
    working.sort_by(|left, right| left.server.cmp(&right.server));
    working
}

/// Tell the server a file is open and what is in it, starting the server if this is the
/// first file of its language. A file no server serves is quietly nothing to do.
pub(crate) fn did_open(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    text: &str,
) -> Result<()> {
    let Some(language) = served_language(file_path) else {
        return Ok(());
    };
    let repo_root = repo_root(state, session_id)?;
    let key = (session_id.to_string(), language.server.name);
    if state.lsp.gave_up_on(&key) {
        return Ok(());
    }
    let server = state
        .lsp
        .ensure(&key, language.server, &repo_root)
        .inspect_err(|_| {
            // Said once, and then the file is one with no server behind it - see
            // [`LspRegistry::would_not_start`].
            state.lsp.give_up_on(&key);
        })?;

    let uri = protocol::file_uri(&repo_root.join(file_path));
    if server.has_document(file_path) {
        // Opening the same file twice is a second pane on it, not a new document.
        server.notify(
            "textDocument/didChange",
            protocol::did_change_params(&uri, text),
        )?;
    } else {
        server.notify(
            "textDocument/didOpen",
            protocol::did_open_params(&uri, language.language_id, text),
        )?;
    }
    server.remember_document(file_path, text);
    Ok(())
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
    let Some(language) = served_language(file_path) else {
        return Ok(());
    };
    let key = (session_id.to_string(), language.server.name);
    let Some(server) = state.lsp.running(&key) else {
        // Nothing has this file open, so there is nothing to tell about the change.
        return Ok(());
    };
    if !server.has_document(file_path) {
        return Ok(());
    }

    let repo_root = repo_root(state, session_id)?;
    let uri = protocol::file_uri(&repo_root.join(file_path));
    server.notify(
        "textDocument/didChange",
        protocol::did_change_params(&uri, text),
    )?;
    server.remember_document(file_path, text);
    Ok(())
}

pub(crate) fn did_close(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    let Some(language) = served_language(file_path) else {
        return Ok(());
    };
    let key = (session_id.to_string(), language.server.name);
    let Some(server) = state.lsp.running(&key) else {
        return Ok(());
    };
    if !server.has_document(file_path) {
        return Ok(());
    }

    let repo_root = repo_root(state, session_id)?;
    let uri = protocol::file_uri(&repo_root.join(file_path));
    server.notify("textDocument/didClose", protocol::did_close_params(&uri))?;
    server.forget_document(file_path);
    Ok(())
}

pub(crate) fn definition(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspLocation>> {
    let Some(question) = ask(state, session_id, file_path, &at)? else {
        return Ok(Vec::new());
    };
    let answer = question.server.request(
        "textDocument/definition",
        protocol::position_params(&question.uri, at.line, question.character),
    )?;
    protocol::locations_from(answer, &question.repo_root)
}

pub(crate) fn completion(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspCompletion>> {
    let Some(question) = ask(state, session_id, file_path, &at)? else {
        return Ok(Vec::new());
    };
    let answer = question.server.request(
        "textDocument/completion",
        protocol::position_params(&question.uri, at.line, question.character),
    )?;
    protocol::completions_from(answer)
}

/// What a question about a place in a file needs: the server to ask, the repo the answer is
/// read against, the document's URI, and the position in the units that server agreed to.
struct Question {
    server: Arc<LanguageServer>,
    repo_root: PathBuf,
    uri: String,
    /// The column, already converted - see [`protocol::lsp_character`].
    character: u32,
}

/// Work out all of that for one place in one file.
///
/// `None` for a file no server serves. A file that is not open is an error rather than an
/// empty answer: a question about a document the server has never been told about is a
/// caller that skipped [`did_open`], not a question with no answer.
fn ask(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: &LspPosition,
) -> Result<Option<Question>> {
    let Some(language) = served_language(file_path) else {
        return Ok(None);
    };
    let key = (session_id.to_string(), language.server.name);
    let server = state.lsp.running(&key).ok_or_else(|| {
        anyhow!(
            "no {} server is running for this review",
            language.server.name
        )
    })?;
    let Some(text) = server.document_text(file_path) else {
        bail!(
            "{file_path} is not open in the {} server",
            language.server.name
        );
    };

    let repo_root = repo_root(state, session_id)?;
    let uri = protocol::file_uri(&repo_root.join(file_path));
    let character = protocol::position_in(&text, at, server.encoding())?;
    Ok(Some(Question {
        server,
        repo_root,
        uri,
        character,
    }))
}

#[cfg(test)]
mod tests;
