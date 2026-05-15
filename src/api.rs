use std::{
    collections::{HashMap, HashSet},
    env,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::comments::CommentDispatchState;

pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";
pub(crate) const DEFAULT_PORT: u16 = 42000;
const HOST_ENV_VAR: &str = "MOONREVIEW_HOST";
const PORT_ENV_VAR: &str = "MOONREVIEW_PORT";

pub(crate) fn bind_host() -> String {
    env::var(HOST_ENV_VAR).unwrap_or_else(|_| DEFAULT_HOST.to_string())
}

pub(crate) fn client_host() -> String {
    match bind_host().as_str() {
        "0.0.0.0" | "::" => DEFAULT_HOST.to_string(),
        host => host.to_string(),
    }
}

pub(crate) fn port() -> Result<u16> {
    match env::var(PORT_ENV_VAR) {
        Ok(raw) => raw
            .parse::<u16>()
            .with_context(|| format!("{PORT_ENV_VAR} must be a valid TCP port")),
        Err(env::VarError::NotPresent) => Ok(DEFAULT_PORT),
        Err(error) => Err(error).with_context(|| format!("failed to read {PORT_ENV_VAR}")),
    }
}

fn port_or_default() -> u16 {
    port().unwrap_or(DEFAULT_PORT)
}

pub(crate) fn server_url() -> String {
    format!("http://{}:{}", client_host(), port_or_default())
}

pub(crate) fn export_server_url() -> String {
    format!("http://localhost:{}", port_or_default())
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) inner: Arc<Mutex<ServerState>>,
    pub(crate) agent_availability: AgentAvailability,
    pub(crate) last_activity: Arc<Mutex<Instant>>,
}

#[derive(Default)]
pub(crate) struct ServerState {
    pub(crate) sessions: HashMap<String, RepoSession>,
}

pub(crate) struct RepoSession {
    pub(crate) repo_path: PathBuf,
    pub(crate) diff_target: DiffTarget,
    pub(crate) active_commit: Option<String>,
    pub(crate) comments: HashMap<String, String>,
    pub(crate) comment_contexts: HashMap<String, HunkCommentContext>,
    pub(crate) reviewed: HashSet<String>,
    pub(crate) selected_agent: AgentKind,
    pub(crate) comment_dispatches: HashMap<String, CommentDispatchState>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct DiffTarget {
    pub(crate) base: Option<String>,
    pub(crate) pathspec: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionOpened {
    pub(crate) session_id: String,
}

#[derive(Serialize)]
pub(crate) struct SessionPayload {
    pub(crate) repo_name: String,
    pub(crate) branch_name: Option<String>,
    pub(crate) commit_base: Option<String>,
    pub(crate) commits: Vec<CommitView>,
    pub(crate) active_commit: Option<String>,
    pub(crate) repo_path: String,
    pub(crate) read_only: bool,
    pub(crate) patch_preview_line_limit: usize,
    pub(crate) available_agents: Vec<AgentOption>,
    pub(crate) selected_agent: AgentKind,
    pub(crate) hunks: Vec<HunkView>,
    pub(crate) sidebar_comments: Vec<SidebarCommentView>,
    pub(crate) export_text: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct CommitView {
    pub(crate) sha: String,
    pub(crate) short_sha: String,
    pub(crate) subject: String,
    pub(crate) author: String,
    pub(crate) review_status: CommitReviewStatus,
}

#[derive(Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CommitReviewStatus {
    Reviewed,
    Partial,
    #[default]
    Unreviewed,
}

#[derive(Serialize, Clone)]
pub(crate) struct HunkView {
    pub(crate) id: String,
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
    pub(crate) header: String,
    pub(crate) staged: bool,
    pub(crate) reviewed: bool,
    pub(crate) comment: String,
    pub(crate) comment_dispatches: Vec<CommentDispatchView>,
    pub(crate) patch_preview: String,
    pub(crate) patch_line_count: usize,
    pub(crate) added_line_count: usize,
    pub(crate) removed_line_count: usize,
    pub(crate) moved_from: Option<HunkMoveHint>,
    pub(crate) moved_to: Option<HunkMoveHint>,
}

#[derive(Serialize, Clone)]
pub(crate) struct HunkMoveHint {
    pub(crate) target_hunk_id: String,
    pub(crate) target_file_path: String,
    pub(crate) target_header: String,
    pub(crate) score: f64,
}

#[derive(Clone, Default)]
pub(crate) struct HunkCommentContext {
    pub(crate) file_path: String,
    pub(crate) header: String,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FileChangeKind {
    Added,
    Deleted,
    #[default]
    Modified,
}

#[derive(Serialize)]
pub(crate) struct SidebarCommentView {
    pub(crate) hunk_id: String,
    pub(crate) comment_index: usize,
    pub(crate) file_path: String,
    pub(crate) header: String,
    pub(crate) selection: String,
    pub(crate) comment: String,
    pub(crate) resolved: bool,
    pub(crate) dispatch_status: CommentDispatchStatus,
    pub(crate) jumpable: bool,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(crate) struct AgentAvailability {
    pub(crate) claude: bool,
    pub(crate) codex: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentKind {
    #[default]
    None,
    Claude,
    Codex,
}

impl AgentKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct AgentOption {
    pub(crate) kind: AgentKind,
    pub(crate) label: &'static str,
    pub(crate) available: bool,
}

#[derive(Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CommentDispatchStatus {
    #[default]
    Idle,
    Batched,
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct CommentDispatchView {
    pub(crate) status: CommentDispatchStatus,
    pub(crate) detail: String,
    pub(crate) can_cancel: bool,
}

#[derive(Serialize)]
pub(crate) struct PatchPayload {
    pub(crate) patch: String,
}

#[derive(Serialize)]
pub(crate) struct FileContentPayload {
    pub(crate) file_path: String,
    pub(crate) content: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenSessionRequest {
    pub(crate) repo_path: String,
    pub(crate) diff_target: Option<DiffTarget>,
}

#[derive(Deserialize)]
pub(crate) struct HunkRequest {
    pub(crate) hunk_id: String,
}

#[derive(Deserialize)]
pub(crate) struct HunkBatchRequest {
    pub(crate) hunk_ids: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileRequest {
    pub(crate) file_path: String,
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    pub(crate) file_path: String,
}

#[derive(Deserialize)]
pub(crate) struct CommentRequest {
    pub(crate) hunk_id: String,
    pub(crate) comment: String,
    #[serde(default)]
    pub(crate) batch: bool,
}

#[derive(Deserialize)]
pub(crate) struct AgentSelectionRequest {
    pub(crate) agent: AgentKind,
}

#[derive(Deserialize)]
pub(crate) struct CommitSelectionRequest {
    pub(crate) commit: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileReviewedRequest {
    pub(crate) file_path: String,
    pub(crate) reviewed: bool,
}

#[derive(Deserialize)]
pub(crate) struct SelectionRequest {
    pub(crate) hunk_id: String,
    pub(crate) selection: String,
}

#[derive(Deserialize)]
pub(crate) struct ReviewedRequest {
    pub(crate) hunk_id: String,
    pub(crate) reviewed: Option<bool>,
}

#[derive(Clone)]
pub(crate) struct DiffHunk {
    pub(crate) id: String,
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
    pub(crate) header: String,
    pub(crate) patch: String,
    pub(crate) staged: bool,
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

pub(crate) fn with_session<T, F>(
    state: &AppState,
    session_id: &str,
    mut f: F,
) -> Result<T, AppError>
where
    F: FnMut(&mut RepoSession) -> Result<T>,
{
    let mut guard = state
        .inner
        .lock()
        .map_err(|_| anyhow!("state lock poisoned"))?;
    let session = guard
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| anyhow!("unknown session"))?;
    f(session).map_err(AppError)
}

pub(crate) fn ensure_session_is_writable(
    state: &AppState,
    session_id: &str,
) -> Result<(), AppError> {
    with_session(state, session_id, |session| {
        if session.diff_target.base.is_some() {
            bail!("range diffs are read-only");
        }
        Ok(())
    })
}

pub(crate) fn lookup_hunk(
    state: &AppState,
    session_id: &str,
    hunk_id: &str,
) -> Result<(PathBuf, String, bool), AppError> {
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
) -> Result<(PathBuf, Vec<(String, bool)>), AppError> {
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

pub(crate) fn stable_id<T: Hash>(value: &T) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

pub(crate) fn session_id_for(path: &Path, diff_target: &DiffTarget) -> String {
    stable_id(&(
        path.display().to_string(),
        diff_target.base.clone(),
        diff_target.pathspec.clone(),
    ))
}
