//! Language servers for the files the editor has open: where a definition is, and what can
//! be typed next.
//!
//! A server has to run where the files are. A session may be reviewing a repo on another
//! machine, so a server started in the window would be reading the wrong disk - or no disk
//! at all. So this lives repo-side, exactly as the shells in [`crate::terminal`] do: the
//! servers hang off [`crate::api::AppState`], the window reaches them through
//! [`crate::backend::Backend`], and a `--remote` session gets the same answers over HTTP as
//! a local one gets by calling straight through.
//!
//! The client itself is [`moon_lsp`], which knows nothing about reviews: it takes a
//! [`Workspace`] - a repo root and an opaque key the servers are held under - and answers
//! about files in it. This module is the layer that turns a session into one of those, and
//! [`routes`] is the same thing again over HTTP.
//!
//! On a server that gives each person a Unix user - see [`crate::unix_users`] - a language
//! server is one more program started for a person: it runs as them, found on their login
//! `PATH`, and what is read to answer about it is read as them. See [`LanguageServers`].

pub(crate) mod routes;

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use moon_lsp::{LspRegistry, Workspace};

use crate::{
    api::{AppState, LspCompletion, LspFileEdit, LspLocation, LspPosition, LspStatus, LspWork},
    unix_users::{Person, UnixUser},
};

/// The language servers this process runs.
///
/// One registry of them where everything runs as the user moon itself runs as, which is
/// everywhere but a server that gives each person a Unix user. There each person has a
/// registry of their own, whose servers run as them and are looked for on their login `PATH`:
/// two people on one repo are answered by two servers, each reading its own person's home,
/// and neither out of the other's.
pub(crate) struct LanguageServers {
    of_this_user: Arc<LspRegistry>,
    /// By the name of each person's Unix user. Made the first time a session of theirs asks
    /// about a file, and kept until [`LanguageServers::end_for`].
    of_each_person: Mutex<HashMap<String, PersonsServers>>,
}

impl LanguageServers {
    pub(crate) fn new() -> Self {
        Self {
            of_this_user: Arc::new(registry_on(
                crate::shell_path::installed_tools_path().to_string(),
            )),
            of_each_person: Mutex::default(),
        }
    }

    /// The registry of one person's servers, made when they have none yet - and made again
    /// when their login `PATH` is no longer the one their servers were looked for on: a
    /// language server they have just installed is then found without the server being
    /// restarted, at the price of their running servers starting over.
    fn of_person(&self, user: &UnixUser) -> Result<Arc<LspRegistry>> {
        // Asked with the lock let go of: their login shell may have to be asked, and nobody
        // else's question waits on that.
        let search_path = user.login_path()?;
        if let Some(theirs) = self.each_persons().get(&user.name)
            && theirs.search_path == search_path
        {
            return Ok(Arc::clone(&theirs.registry));
        }
        let registry = Arc::new(registry_of(user, search_path.clone()));
        self.each_persons().insert(
            user.name.clone(),
            PersonsServers {
                search_path,
                registry: Arc::clone(&registry),
            },
        );
        Ok(registry)
    }

    /// Stop every language server running as `user`: dropping a registry shuts its servers
    /// down, once the questions being asked of them right now have been answered.
    pub(crate) fn end_for(&self, user: &UnixUser) {
        let theirs = self.each_persons().remove(&user.name);
        drop(theirs);
    }

    fn each_persons(&self) -> std::sync::MutexGuard<'_, HashMap<String, PersonsServers>> {
        self.of_each_person
            .lock()
            .expect("nothing panics holding the language servers")
    }
}

/// A registry whose servers are looked for on `search_path`.
///
/// The servers are told they are talking to this application rather than to the client crate
/// they are reached through: `clientInfo` is what a server writes into its log, and a report
/// about rust-analyzer under a review is only findable if the log says which program was
/// asking.
fn registry_on(search_path: String) -> LspRegistry {
    LspRegistry::new(search_path).identifying_as(moon_lsp::ClientIdentity::new(
        "moonreview",
        env!("CARGO_PKG_VERSION"),
    ))
}

/// A registry whose servers run as `user`: looked for on their login `PATH`, and started
/// through [`UnixUser::command`], which is what gives each their `HOME` and none of the
/// server's environment.
fn registry_of(user: &UnixUser, search_path: String) -> LspRegistry {
    let user = user.clone();
    let as_them = move |server: &Path| {
        let server = server
            .to_str()
            .with_context(|| format!("the path {server:?} is not UTF-8"))?;
        user.command(server)
    };
    registry_on(search_path).starting_servers_with(as_them)
}

/// One person's language servers, and the login `PATH` they were looked for on.
struct PersonsServers {
    search_path: String,
    registry: Arc<LspRegistry>,
}

/// The registry the servers of a session are in, and the person they run as - nobody, where
/// every server runs as the user moon itself does.
///
/// Such a moon does not look the session up at all: what it answers with is the same for
/// every session, and a pane asking for a file's status as it draws costs a map read.
fn servers_of(state: &AppState, session_id: &str) -> Result<(Arc<LspRegistry>, Option<Person>)> {
    if !crate::unix_users::each_person_has_one() {
        return Ok((Arc::clone(&state.lsp.of_this_user), None));
    }
    let person = crate::api::person_of(state, session_id)?.context(
        "this session is nobody's, on a server that runs each person's language servers as \
         their own Unix user",
    )?;
    let theirs = state.lsp.of_person(&person.unix_user)?;
    Ok((theirs, Some(person)))
}

/// Ask `question` of the servers of a session. On a server that gives each person a Unix
/// user it is asked as the session's person - see [`crate::unix_users::as_user`] - so the
/// files read for the answer, a dependency's source among them, are read as them.
fn asked<T>(
    state: &AppState,
    session_id: &str,
    question: impl FnOnce(&LspRegistry) -> T,
) -> Result<T> {
    let (servers, person) = servers_of(state, session_id)?;
    match &person {
        Some(person) => crate::unix_users::as_user(person, || question(&servers)),
        None => Ok(question(&servers)),
    }
}

/// Ask `question` about the repo of a session - see [`asked`].
fn asked_about_the_repo<T>(
    state: &AppState,
    session_id: &str,
    question: impl FnOnce(&LspRegistry, &Workspace<'_>) -> Result<T>,
) -> Result<T> {
    let repo_root = repo_root(state, session_id)?;
    asked(state, session_id, |servers| {
        question(servers, &workspace(session_id, &repo_root))
    })?
}

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
/// The answer is about what is running rather than about what is on disk, so a pane asking
/// as it draws costs nothing but a map read.
pub(crate) fn status(state: &AppState, session_id: &str, file_path: &str) -> Result<LspStatus> {
    asked(state, session_id, |servers| {
        servers.status(session_id, file_path)
    })
}

/// What every language server running for this session is doing right now, for the status
/// bar along the bottom of the window.
pub(crate) fn working(state: &AppState, session_id: &str) -> Result<Vec<LspWork>> {
    asked(state, session_id, |servers| servers.working(session_id))
}

/// Tell the server a file is open and what is in it, starting the server if this is the
/// first file of its language.
pub(crate) fn did_open(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    text: &str,
) -> Result<()> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.did_open(repo, file_path, text)
    })
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
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.did_change(repo, file_path, text)
    })
}

/// Tell the server the window is done with a file.
pub(crate) fn did_close(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.did_close(repo, file_path)
    })
}

/// The places of one kind the server names for the name at this place: where it is defined,
/// where its type is, where it is implemented, or everywhere it is used.
///
/// The answer is also what says which files outside the repo this session may read: a
/// definition in a dependency or in the standard library is a file the pane has to be able to
/// open, and a server answering the person's own question is the only thing that legitimately
/// names one - see [`FilesNamedOutsideTheRepo`].
pub(crate) fn places(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
    which: crate::api::LspPlaces,
) -> Result<Vec<LspLocation>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        let locations = servers.places(repo, file_path, at, which)?;
        remember_files_named(state, session_id, &locations)?;
        Ok(locations)
    })
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
) -> Result<Vec<char>> {
    asked(state, session_id, |servers| {
        servers.trigger_characters(session_id, file_path)
    })
}

/// What the name at this place is called, as the server would rename it.
pub(crate) fn prepare_rename(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Option<String>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.prepare_rename(repo, file_path, at)
    })
}

/// Everything calling the name at this place `new_name` would change, file by file.
///
/// Unlike a definition, nothing here is remembered as readable: [`moon_lsp`] refuses a rename
/// that would edit a file outside the repo whole, so every file in the answer is one of the
/// repo's own.
pub(crate) fn rename(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
    new_name: &str,
) -> Result<Vec<LspFileEdit>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.rename(repo, file_path, at, new_name)
    })
}

/// The edits that format the whole of one file, indented the way the repo says.
pub(crate) fn format(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    options: moon_lsp::LspFormatting,
) -> Result<Vec<moon_lsp::LspTextEdit>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.format(repo, file_path, options)
    })
}

/// What the server says about the name at this place, as markdown.
pub(crate) fn hover(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Option<String>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.hover(repo, file_path, at)
    })
}

/// What the server last said is wrong with a file open in it.
pub(crate) fn diagnostics(
    state: &AppState,
    session_id: &str,
    file_path: &str,
) -> Result<Vec<moon_lsp::LspDiagnostic>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        Ok(servers.diagnostics(repo, file_path))
    })
}

/// Tell the server a file open in it was written to disk.
pub(crate) fn did_save(state: &AppState, session_id: &str, file_path: &str) -> Result<()> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.did_save(repo, file_path)
    })
}

/// What the server offers to do to the code at this place.
pub(crate) fn code_actions(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<moon_lsp::LspCodeAction>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.code_actions(repo, file_path, at)
    })
}

/// The signature of the call around this place.
pub(crate) fn signature_help(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Option<moon_lsp::LspSignature>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.signature_help(repo, file_path, at)
    })
}

/// What could be typed at this place.
pub(crate) fn completion(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    at: LspPosition,
) -> Result<Vec<LspCompletion>> {
    asked_about_the_repo(state, session_id, |servers, repo| {
        servers.completion(repo, file_path, at)
    })
}
