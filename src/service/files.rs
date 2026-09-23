//! A review's files: read as they are or at a commit, blamed, searched for by name and by
//! what is in them, and written.

use anyhow::{Result, bail};

use crate::{
    api::{AppState, BlameOf, BlamePayload, ContentMatch, FileContentPayload, SearchScope},
    git::read_repo_file,
    search::SearchListener,
};

/// The text of one file, for a tab that is showing it.
///
/// Two kinds of file reach this: a file of the repo, named relative to its root, and a file
/// outside the repo that a language server named as where something is defined - a
/// dependency's source, or the standard library. The second kind is only ever read when that
/// session's own allow-list holds it, which is what keeps this from being a way to read any
/// file on the machine hosting the repo.
pub(crate) fn session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
) -> Result<FileContentPayload> {
    crate::api::with_session(state, session_id, |session| {
        // A file a language server named outside the repo is read from where it is, and read
        // only - see [`crate::lsp::FilesNamedOutsideTheRepo`]. Every other path is a path in
        // the repo and is refused if it turns out not to be one, exactly as it always was.
        if let Some(real_path) = session.files_named_outside_the_repo.allows(file_path) {
            return Ok(FileContentPayload {
                file_path: file_path.to_string(),
                content: crate::git::read_file_named_outside_the_repo(&real_path)?,
                outside_the_repo: true,
                committed: None,
            });
        }
        Ok(FileContentPayload {
            file_path: file_path.to_string(),
            content: read_repo_file(&session.repo_path, file_path)?,
            outside_the_repo: false,
            committed: Some(crate::git::read_committed_file(
                &session.repo_path,
                file_path,
            )?),
        })
    })
}

/// Who last touched each stretch of a file, in the version `of` names - the tab's buffer, so
/// a line typed a moment ago reads as not committed, or the file as one commit has it. Runs
/// where the repo is. A file outside the repo has no history here to ask about, and says so
/// rather than having git say something less clear about a path it cannot see.
pub(crate) fn blame_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    of: &BlameOf,
) -> Result<BlamePayload> {
    crate::api::with_session(state, session_id, |session| {
        if session
            .files_named_outside_the_repo
            .allows(file_path)
            .is_some()
        {
            bail!("{file_path} is outside the repo, and has no history here");
        }
        Ok(BlamePayload {
            file_path: file_path.to_string(),
            chunks: crate::git::blame_file(&session.repo_path, file_path, of)?,
        })
    })
}

/// A file of the repo as one commit has it, for a tab that shows an old version of it. Read
/// only, and with nothing to be new against: it is not the working tree.
pub(crate) fn session_file_at(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    revision: &str,
) -> Result<FileContentPayload> {
    crate::api::with_session(state, session_id, |session| {
        Ok(FileContentPayload {
            file_path: file_path.to_string(),
            content: crate::git::read_file_at(&session.repo_path, file_path, revision)?,
            outside_the_repo: false,
            committed: None,
        })
    })
}

/// The files of the repo whose names match a search. Runs where the repo is, which is what
/// makes it work on a `--remote` connection as well as a repo on this machine.
pub(crate) fn find_session_files(
    state: &AppState,
    session_id: &str,
    query: &str,
    scope: SearchScope,
    listener: &mut dyn SearchListener<String>,
) -> Result<()> {
    // The path is read under the sessions lock and the search runs without it: a walk of a
    // large tree takes seconds, and nothing else about the sessions should wait on it.
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    crate::search::file_names::stream_matching_paths(&repo_path, query, scope, listener)
}

/// The lines of the repo that hold what was searched for. Runs where the repo is, the same
/// as the file-name search beside it.
pub(crate) fn search_session_contents(
    state: &AppState,
    session_id: &str,
    query: &str,
    scope: SearchScope,
    listener: &mut dyn SearchListener<ContentMatch>,
) -> Result<()> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    crate::search::file_contents::stream_matching_lines(&repo_path, query, scope, listener)
}

pub(crate) fn write_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    content: &str,
) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    crate::api::with_session(state, session_id, |session| {
        crate::git::write_repo_file(&session.repo_path, file_path, content)
    })
}

/// Create a file of the repo nothing is at yet - see [`crate::git::create_repo_file`].
pub(crate) fn create_session_file(
    state: &AppState,
    session_id: &str,
    file_path: &str,
    content: &str,
) -> Result<()> {
    crate::api::ensure_session_is_writable(state, session_id)?;
    crate::api::with_session(state, session_id, |session| {
        crate::git::create_repo_file(&session.repo_path, file_path, content)
    })
}
