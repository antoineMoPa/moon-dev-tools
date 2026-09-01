use std::{
    collections::HashMap,
    env,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
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
    pub(crate) terminals: Arc<crate::terminal::TerminalRegistry>,
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
    pub(crate) selected_agent: AgentKind,
    pub(crate) comment_dispatches: HashMap<String, CommentDispatchState>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct DiffTarget {
    pub(crate) base: Option<String>,
    pub(crate) pathspec: Option<String>,
    pub(crate) comparison: Option<[String; 2]>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionOpened {
    pub(crate) session_id: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionPayload {
    pub(crate) repo_name: String,
    pub(crate) branch_name: Option<String>,
    pub(crate) commit_base: Option<String>,
    pub(crate) commits: Vec<CommitView>,
    pub(crate) history_commits: Vec<CommitView>,
    pub(crate) history_has_more: bool,
    pub(crate) local_change_summary: LocalChangeSummary,
    pub(crate) active_commit: Option<String>,
    pub(crate) repo_path: String,
    pub(crate) read_only: bool,
    pub(crate) patch_preview_line_limit: usize,
    pub(crate) available_agents: Vec<AgentOption>,
    pub(crate) selected_agent: AgentKind,
    pub(crate) full_file_path: Option<String>,
    pub(crate) hunks: Vec<HunkView>,
    pub(crate) review_comments: Vec<ReviewCommentView>,
    pub(crate) export_text: String,
}

/// One repo and how many of its files have changed, as the submodule hub lists it: the
/// reviewed repo itself, or one of its submodules. A submodule with changed files is another
/// review the user can open beside this one.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct RepoStatusView {
    pub(crate) repo_path: String,
    /// The repo's directory name.
    pub(crate) name: String,
    pub(crate) changed_files: usize,
}

/// What the submodule hub shows: the reviewed repo first, then every submodule of it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SubmoduleHubPayload {
    pub(crate) root: RepoStatusView,
    pub(crate) submodules: Vec<RepoStatusView>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CommitHistoryPayload {
    pub(crate) commits: Vec<CommitView>,
    pub(crate) has_more: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct LocalChangeSummary {
    pub(crate) modified: usize,
    pub(crate) added: usize,
    pub(crate) deleted: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CommitView {
    pub(crate) sha: String,
    pub(crate) short_sha: String,
    pub(crate) subject: String,
    pub(crate) author: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct HunkView {
    pub(crate) id: String,
    pub(crate) file_path: String,
    pub(crate) change_kind: FileChangeKind,
    pub(crate) header: String,
    pub(crate) staged: bool,
    pub(crate) comment: String,
    pub(crate) comment_dispatches: Vec<CommentDispatchView>,
    pub(crate) patch_preview: String,
    pub(crate) patch_line_count: usize,
    pub(crate) added_line_count: usize,
    pub(crate) removed_line_count: usize,
    pub(crate) moved_from: Option<HunkMoveHint>,
    pub(crate) moved_to: Option<HunkMoveHint>,
    pub(crate) image_diff: Option<ImageDiffView>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ImageDiffView {
    pub(crate) before_src: Option<String>,
    pub(crate) after_src: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
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

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FileChangeKind {
    Added,
    Deleted,
    #[default]
    Modified,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct ReviewCommentView {
    pub(crate) hunk_id: String,
    pub(crate) comment_index: usize,
    pub(crate) file_path: String,
    pub(crate) header: String,
    pub(crate) selection: String,
    pub(crate) comment: String,
    pub(crate) resolved: bool,
    pub(crate) dispatch: CommentDispatchView,
    pub(crate) jumpable: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct AgentAvailability {
    pub(crate) claude: bool,
    pub(crate) codex: bool,
    pub(crate) opencode: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentKind {
    #[default]
    None,
    Claude,
    Codex,
    OpenCode,
}

impl AgentKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AgentOption {
    pub(crate) kind: AgentKind,
    pub(crate) label: String,
    pub(crate) available: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CommentDispatchStatus {
    #[default]
    Idle,
    Batched,
    Queued,
    Running,
    Canceled,
    Completed,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CommentDispatchView {
    pub(crate) key: String,
    pub(crate) status: CommentDispatchStatus,
    pub(crate) detail: String,
    pub(crate) agent: AgentKind,
    pub(crate) can_cancel: bool,
    pub(crate) has_log: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PatchPayload {
    pub(crate) patch: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct FileContentPayload {
    pub(crate) file_path: String,
    pub(crate) content: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct OpenSessionRequest {
    pub(crate) repo_path: String,
    pub(crate) diff_target: Option<DiffTarget>,
    pub(crate) active_commit: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CommitRunStarted {
    pub(crate) terminal_id: String,
}

/// One shell as the server has it: which one, and what it is called.
#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalView {
    pub(crate) terminal_id: String,
    /// What its tab reads, if it has been named - see `TerminalRegistry::name`.
    pub(crate) name: Option<String>,
}

/// A shell being renamed: what it is to be called.
#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalNameRequest {
    pub(crate) name: String,
}

/// How a commit run ended. `None` while it is still going.
#[derive(Serialize, Deserialize)]
pub(crate) struct CommitRunOutcome {
    pub(crate) exit_code: Option<i32>,
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

#[derive(Serialize, Deserialize)]
pub(crate) struct WriteFileRequest {
    pub(crate) file_path: String,
    pub(crate) content: String,
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    pub(crate) file_path: String,
}

#[derive(Deserialize)]
pub(crate) struct FileSearchQuery {
    pub(crate) query: String,
}

/// The paths of the repo whose names match a search, and whether there were more of them
/// than the search hands back.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct FileMatchesPayload {
    pub(crate) files: Vec<String>,
    pub(crate) truncated: bool,
}

/// One line of the repo that a content search found.
#[derive(Serialize, Deserialize)]
pub(crate) struct ContentMatch {
    pub(crate) file_path: String,
    /// Counted from one, the way the number in an editor's fringe is.
    pub(crate) line_number: usize,
    /// The line itself, trimmed of its indentation and cut short if it was a long one.
    pub(crate) line: String,
}

/// The lines of the repo that hold what was searched for, and whether there were more of
/// them than the search hands back.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct ContentMatchesPayload {
    pub(crate) matches: Vec<ContentMatch>,
    pub(crate) truncated: bool,
}

#[derive(Deserialize)]
pub(crate) struct CommitHistoryQuery {
    pub(crate) offset: Option<usize>,
    pub(crate) limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct CommentRequest {
    pub(crate) hunk_id: String,
    pub(crate) comment: String,
    #[serde(default)]
    pub(crate) batch: bool,
}

#[derive(Deserialize)]
pub(crate) struct CancelCommentDispatchRequest {
    pub(crate) hunk_id: String,
    pub(crate) comment_index: usize,
}

#[derive(Deserialize)]
pub(crate) struct AgentLogQuery {
    pub(crate) dispatch_key: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AgentLogPayload {
    pub(crate) dispatch_key: String,
    pub(crate) text: String,
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
pub(crate) struct SelectionRequest {
    pub(crate) hunk_id: String,
    pub(crate) selection: String,
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

pub(crate) fn with_session<T, F>(state: &AppState, session_id: &str, mut f: F) -> Result<T>
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
