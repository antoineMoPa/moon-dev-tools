//! What the server holds between requests: its sessions, and the agents and shells working
//! on them. Never compiled for the browser, whose window holds none of it - it asks the
//! server, through the types beside this module.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};

use anyhow::{Result, anyhow, bail};
use axum::response::IntoResponse;

use serde::{Deserialize, Serialize};

use super::{AgentKind, DiffTarget, FileChangeKind, ImageDiffView};
use crate::comments::CommentDispatchState;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) inner: Arc<Mutex<ServerState>>,
    pub(crate) agent_availability: AgentAvailability,
    pub(crate) last_activity: Arc<Mutex<Instant>>,
    pub(crate) terminals: Arc<crate::terminal::TerminalRegistry>,
    /// The language servers running for these reviews. Repo-side like the shells beside it,
    /// because a server has to read the files it answers about - see [`crate::lsp`].
    pub(crate) lsp: Arc<moon_lsp::LspRegistry>,
}

#[derive(Default)]
pub(crate) struct ServerState {
    pub(crate) sessions: HashMap<String, RepoSession>,
    /// The repo this process is for: the one it was started in, or else the first one it
    /// opened - which, for a window started from a launcher, in `/`, is the repo the window is
    /// on. What a browser opening `/moon` without saying which repo is shown - see
    /// `crate::server::web_page`.
    pub(crate) home_repo: Option<PathBuf>,
}

pub(crate) struct RepoSession {
    pub(crate) repo_path: PathBuf,
    pub(crate) diff_target: DiffTarget,
    pub(crate) active_commit: Option<String>,
    pub(crate) comments: HashMap<String, String>,
    pub(crate) comment_contexts: HashMap<String, HunkCommentContext>,
    pub(crate) selected_agent: AgentKind,
    pub(crate) comment_dispatches: HashMap<String, CommentDispatchState>,
    /// The files outside the repo a language server has named as answers to this session's
    /// go-to-definition questions, which are the only files outside it that may be read - see
    /// [`crate::lsp::FilesNamedOutsideTheRepo`].
    pub(crate) files_named_outside_the_repo: crate::lsp::FilesNamedOutsideTheRepo,
}

impl RepoSession {
    /// What this session's review is of - see [`ReviewTarget`].
    pub(crate) fn review_target(&self) -> ReviewTarget {
        ReviewTarget {
            repo_path: self.repo_path.clone(),
            diff_target: self.diff_target.clone(),
            active_commit: self.active_commit.clone(),
        }
    }
}

/// What a session's review is of: the repo, and which of its changes - the part of a
/// [`RepoSession`] that git is asked about. Taken off the session as a value of its own so
/// git can be asked with the server's lock released: the diff of a repo with a lot changed
/// takes a while, and everything else the server does - starting a shell, staging a hunk -
/// waits on that one lock. Compared after the asking, so an answer about a target the
/// session has since moved off is asked again rather than shown.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ReviewTarget {
    pub(crate) repo_path: PathBuf,
    pub(crate) diff_target: DiffTarget,
    pub(crate) active_commit: Option<String>,
}

pub(crate) type CancelToken = Arc<AtomicBool>;

/// Live stdout/stderr of one agent run, shared by every comment that run addresses.
pub(crate) type AgentLog = Arc<Mutex<String>>;

/// Older output is dropped once a run exceeds this, so a chatty agent cannot grow the session forever.
const AGENT_LOG_MAX_BYTES: usize = 200_000;

pub(crate) fn append_to_agent_log(log: &AgentLog, chunk: &str) {
    let Ok(mut text) = log.lock() else {
        return;
    };

    text.push_str(chunk);
    if text.len() <= AGENT_LOG_MAX_BYTES {
        return;
    }

    let drop_until = text
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| text.len() - index <= AGENT_LOG_MAX_BYTES)
        .unwrap_or(text.len());
    text.replace_range(..drop_until, "");
}

pub(crate) fn read_agent_log(log: &AgentLog) -> Result<String> {
    log.lock()
        .map(|text| text.clone())
        .map_err(|_| anyhow!("agent log lock poisoned"))
}

#[derive(Clone)]
pub(crate) struct DiffHunk {
    pub(crate) id: String,
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
    pub(crate) header: String,
    pub(crate) patch: String,
    pub(crate) staged: bool,
    pub(crate) image_diff: Option<ImageDiffView>,
}

#[derive(Debug)]
pub(crate) struct AppError(pub(crate) anyhow::Error);

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self(value.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (axum::http::StatusCode::BAD_REQUEST, self.0.to_string()).into_response()
    }
}

/// Push back the idle shutdown: the server only stops once nothing has touched it for a while.
pub(crate) fn mark_activity(last_activity: &Mutex<Instant>) {
    if let Ok(mut last_activity) = last_activity.lock() {
        *last_activity = Instant::now();
    }
}

pub(crate) fn with_session<T, F>(state: &AppState, session_id: &str, f: F) -> Result<T>
where
    F: FnOnce(&mut RepoSession) -> Result<T>,
{
    let mut guard = state
        .inner
        .lock()
        .map_err(|_| anyhow!("state lock poisoned"))?;
    let session = guard
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| anyhow!("unknown session"))?;
    f(session)
}

pub(crate) fn ensure_session_is_writable(state: &AppState, session_id: &str) -> Result<()> {
    with_session(state, session_id, |session| {
        if session.diff_target.base.is_some() || session.diff_target.comparison.is_some() {
            bail!("this review is read-only");
        }
        Ok(())
    })
}

pub(crate) fn lookup_hunk(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
) -> Result<(PathBuf, String, bool)> {
    with_session(state, session_id, |session| {
        let hunk = crate::git::collect_session_hunks(session)?
            .into_iter()
            .find(|hunk| hunk.id == hunk_id)
            .ok_or_else(|| anyhow!("hunk no longer exists"))?;
        Ok((session.repo_path.clone(), hunk.patch, hunk.staged))
    })
}

pub(crate) fn lookup_hunks(
    state: &AppState,
    session_id: &str,
    hunk_ids: &[String],
) -> Result<(PathBuf, Vec<(String, bool)>)> {
    with_session(state, session_id, |session| {
        let hunks = crate::git::collect_session_hunks(session)?;
        let mut patches = Vec::with_capacity(hunk_ids.len());

        for hunk_id in hunk_ids {
            let hunk = hunks
                .iter()
                .find(|hunk| hunk.id == *hunk_id)
                .ok_or_else(|| anyhow!("hunk no longer exists"))?;
            patches.push((hunk.patch.clone(), hunk.staged));
        }

        Ok((session.repo_path.clone(), patches))
    })
}

#[derive(Clone, Default)]
pub(crate) struct HunkCommentContext {
    pub(crate) file_path: String,
    pub(crate) header: String,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct AgentAvailability {
    pub(crate) claude: bool,
    pub(crate) codex: bool,
    pub(crate) opencode: bool,
}

pub(crate) fn stable_id<T: Hash>(value: &T) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub(crate) fn session_id_for_view(
    path: &Path,
    diff_target: &DiffTarget,
    active_commit: Option<&str>,
) -> String {
    stable_id(&(
        path.display().to_string(),
        diff_target.base.clone(),
        diff_target.pathspec.clone(),
        diff_target.comparison.clone(),
        active_commit.map(ToOwned::to_owned),
    ))
}
