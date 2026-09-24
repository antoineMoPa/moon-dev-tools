// The server's own state is kept apart from the wire types, which are all the window in a
// browser compiles of this module.
#[cfg(not(target_arch = "wasm32"))]
mod server_state;
#[cfg(not(target_arch = "wasm32"))]
mod serving;

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use server_state::*;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use serving::*;

/// The port a server listens on unless told otherwise, and the one an address without a port
/// is taken to name.
pub(crate) const DEFAULT_PORT: u16 = 42000;

/// A shell that asked for a person and has not had one since: it rang its bell, or sent the
/// notification a terminal would put on the desktop - see [`crate::attention`]. Answered by
/// typing into the shell, which takes it off.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct TerminalAttentionView {
    pub(crate) terminal_id: String,
    /// What the shell is called, if it has been named - `write the parser claude - 1`.
    pub(crate) name: Option<String>,
    /// What it asked for: a bare bell, or a notification with what it said. The two are
    /// told apart because a window posts a notification and only marks a bell - see
    /// [`crate::native::model::Model::take_attention`].
    pub(crate) asked: crate::attention::Asked,
    /// When it asked, in seconds since the epoch: a window posts each ask once, and this is
    /// how it tells one it has posted from a new one.
    pub(crate) at_unix: u64,
}

/// Every shell asking for a person, as the window asks for them.
#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalAttentionList {
    pub(crate) terminals: Vec<TerminalAttentionView>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
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
    /// The commits of the repo no remote branch has yet - see
    /// [`crate::git::unpushed_commit_count`].
    pub(crate) unpushed_commits: usize,
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
    /// Whether this is a file outside the repo - a dependency's source or the standard
    /// library, landed on by a jump to a definition. Those are read-only: the pane offers no
    /// save on one, and a write to it is refused repo-side whatever the pane does.
    pub(crate) outside_the_repo: bool,
    /// The file as HEAD has it, which the editor marks the new lines of the text against:
    /// empty for a file HEAD does not have, every line of which is new, and `None` for a file
    /// outside the repo, which has no history here to be new against.
    pub(crate) committed: Option<String>,
}

/// Who last touched each stretch of a file, as `git blame` has it - see
/// [`crate::git::blame_file`]. The stretches are in order, cover every line of the text they
/// were asked about, and are cut where the commit changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct BlamePayload {
    pub(crate) file_path: String,
    pub(crate) chunks: Vec<BlameChunk>,
}

/// One stretch of lines that a single commit - or nothing yet - last touched.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct BlameChunk {
    /// The lines of the text the stretch covers, as indexes from zero.
    pub(crate) lines: std::ops::Range<usize>,
    pub(crate) blamed: Blamed,
    /// The file as it was just before the change the stretch is put down to: the blamed
    /// commit's parent and the path there, or HEAD for lines not committed yet. `None` where
    /// there is no before - the commit that brought the file in.
    pub(crate) before: Option<FileVersion>,
    /// The line the stretch started on in the version the change was made in, counted from
    /// one - about where to look for it in the version before.
    pub(crate) line_in_commit: usize,
}

/// A file as one commit has it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct FileVersion {
    pub(crate) sha: String,
    /// The path the file had in that commit, which a rename since has changed.
    pub(crate) file_path: String,
}

/// What a stretch of lines is put down to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) enum Blamed {
    Committed(BlamedCommit),
    /// Lines no commit has: typed into the working tree, saved or not, since the last one.
    NotYetCommitted,
}

/// The commit a stretch of lines was last changed in, as much of it as the column beside the
/// lines and the note hung off it read out.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct BlamedCommit {
    pub(crate) sha: String,
    pub(crate) author: String,
    /// The day it was authored, as `YYYY-MM-DD` in the author's own time zone - the day the
    /// author would say they wrote it.
    pub(crate) authored_on: String,
    /// The first line of the commit message.
    pub(crate) summary: String,
}

/// Which version of a file a blame is of.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) enum BlameOf {
    /// A text that may not be what is on disk: the tab's buffer, edits and all, so a line
    /// typed a moment ago reads as not committed rather than as whatever used to be on that
    /// line.
    Text(String),
    /// The file as one commit has it - a blame with no uncommitted lines in it.
    Revision(String),
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct OpenSessionRequest {
    pub(crate) repo_path: String,
    pub(crate) diff_target: Option<DiffTarget>,
    pub(crate) active_commit: Option<String>,
}

/// A pass key the server made for a window already let in - see `POST /api/pass-key`. Only a
/// native window asks: a browser's has no Tools menu to hand one on from.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct PassKeyMinted {
    pub(crate) pass_key: String,
}

/// What `POST /api/login-ticket` is asked: how long the ticket is to be good for.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct LoginTicketRequest {
    pub(crate) lifetime_seconds: u64,
}

/// A login ticket the server made for a window already let in, to open a browser with - see
/// `POST /api/login-ticket`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct LoginTicketMinted {
    pub(crate) ticket: String,
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

/// The types a language question and its answer are made of, from the client crate.
///
/// Re-exported rather than redefined so that the wire format and the client's own types are
/// one thing: a `--remote` window serialises exactly what [`moon_lsp`] hands back.
pub(crate) use moon_lsp::{
    LspCompletion, LspFileEdit, LspLocation, LspPlaces, LspPosition, LspStatus, LspWork,
};

/// A question about the places of one kind for the name at one place in one file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspPlacesRequest {
    pub(crate) file_path: String,
    pub(crate) at: LspPosition,
    pub(crate) which: LspPlaces,
}

/// A file to format, and how the repo indents.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspFormatRequest {
    pub(crate) file_path: String,
    pub(crate) options: moon_lsp::LspFormatting,
}

/// The edits that format one file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspTextEditsPayload {
    pub(crate) edits: Vec<moon_lsp::LspTextEdit>,
}

/// What a server says about a name, as markdown.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspHoverPayload {
    pub(crate) markdown: Option<String>,
}

/// What a server last said is wrong with a file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspDiagnosticsPayload {
    pub(crate) diagnostics: Vec<moon_lsp::LspDiagnostic>,
}

/// What a server offers to do at a place.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspCodeActionsPayload {
    pub(crate) actions: Vec<moon_lsp::LspCodeAction>,
}

/// The signature of the call around a place.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspSignaturePayload {
    pub(crate) signature: Option<moon_lsp::LspSignature>,
}

/// A new name for the name at one place in one file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspRenameRequest {
    pub(crate) file_path: String,
    pub(crate) at: LspPosition,
    pub(crate) new_name: String,
}

/// What the name at a place is called, as the server would rename it. `None` is nothing there
/// that can be renamed.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspRenamablePayload {
    pub(crate) name: Option<String>,
}

/// Everything a rename changes, one entry per file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspFileEditsPayload {
    pub(crate) files: Vec<LspFileEdit>,
}

/// A file the editor has opened or changed, and what is in it now.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspDocumentRequest {
    pub(crate) file_path: String,
    pub(crate) text: String,
}

/// A question about one place in one file.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspPositionRequest {
    pub(crate) file_path: String,
    pub(crate) at: LspPosition,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LspStatusPayload {
    pub(crate) status: LspStatus,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LspWorkPayload {
    pub(crate) working: Vec<LspWork>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LspLocationsPayload {
    pub(crate) locations: Vec<LspLocation>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct LspCompletionsPayload {
    pub(crate) completions: Vec<LspCompletion>,
}

/// The characters a language server said should open a completion list on their own - the
/// `.` of `thing.`, the `:` of a path. One answer per file, and it does not change for as
/// long as that server runs.
#[derive(Serialize, Deserialize)]
pub(crate) struct LspTriggersPayload {
    pub(crate) triggers: Vec<char>,
}

/// Which files a search reads: the files of the repo, or those and the ones its `.gitignore`
/// leaves out - a submodule some repos ignore, a vendored directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum SearchScope {
    #[default]
    RepoFiles,
    IncludingIgnored,
}

impl SearchScope {
    /// The scope a checkbox reading "include gitignored" stands for, ticked or not.
    pub(crate) fn including_ignored(included: bool) -> Self {
        if included {
            Self::IncludingIgnored
        } else {
            Self::RepoFiles
        }
    }

    pub(crate) fn includes_ignored(self) -> bool {
        self == Self::IncludingIgnored
    }
}

/// What a search has found so far. A search reports one of these every time what it has
/// found changes, and once more, marked done, when it is over; each stands on its own - the
/// matches are the whole of what is worth showing, not the ones since the report before.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SearchProgress<T> {
    pub(crate) matches: Vec<T>,
    /// Set when there were more matches than the search hands back, so the palette can say
    /// that narrowing the query would show different rows rather than only fewer.
    pub(crate) truncated: bool,
    pub(crate) done: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> SearchProgress<T> {
    /// A search that is over without having had anything to look for.
    pub(crate) fn nothing() -> Self {
        Self {
            matches: Vec::new(),
            truncated: false,
            done: true,
        }
    }
}

/// One line of a search streamed over the wire: a report, or the reason the search failed,
/// which comes last when it comes at all - the response is under way by then, so a status
/// line cannot carry it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SearchLine<T> {
    Found(SearchProgress<T>),
    Failed(String),
}

/// One line of the repo that a content search found.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ContentMatch {
    pub(crate) file_path: String,
    /// Counted from one, the way the number in an editor's fringe is.
    pub(crate) line_number: usize,
    /// The line itself, trimmed of its indentation and cut short if it was a long one.
    pub(crate) line: String,
}

/// How many commits of the history one page of it holds: the first page comes with the
/// session, and the window asks for each next one by this size too.
pub(crate) const HISTORY_COMMIT_PAGE_SIZE: usize = 30;

#[derive(Deserialize)]
pub(crate) struct CommentRequest {
    pub(crate) hunk_id: String,
    pub(crate) comment: String,
    #[serde(default)]
    pub(crate) batch: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct AgentLogPayload {
    pub(crate) dispatch_key: String,
    pub(crate) text: String,
}
