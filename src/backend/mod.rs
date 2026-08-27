//! Where the native frontend gets its reviews from.
//!
//! The window is the same either way: [`local::LocalBackend`] reviews a repo in this
//! process, and [`remote::RemoteBackend`] reviews a repo on another machine over the same
//! HTTP API the web frontend uses. Only this trait knows which one is in play.

pub(crate) mod local;
pub(crate) mod remote;
#[cfg(test)]
mod remote_tests;

use anyhow::Result;

use crate::{
    agent_sessions::AgentSessionView,
    api::{
        AgentKind, AgentLogPayload, CommentRequest, CommitHistoryPayload, FileContentPayload,
        ContentMatchesPayload, FileMatchesPayload, OpenSessionRequest, PatchPayload, SessionOpened, SessionPayload,
        SubmoduleView,
    },
    commit_suggestion::CommitSuggestion,
    committing::{CommitAction, CommitState},
    moontasks::{
        AttachResourceRequest, BoardColumn, ColumnId, CreateTaskRequest, StartResourceRequest,
        TaskView,
    },
};

/// Every review operation the native frontend performs. Calls block, so the UI runs them
/// on worker threads - a remote backend is a network round-trip.
pub(crate) trait Backend: Send + Sync + 'static {
    /// How this connection reads in the UI, e.g. `local` or `dev-box:42000`.
    fn describe(&self) -> String;

    /// Whether the repos this reads are on the machine the window runs on, which is what
    /// decides if the window can offer a folder picker for them.
    fn reads_this_machine(&self) -> bool;

    /// Where a browser can open the same review, for the "open in browser" action.
    fn web_url(&self, session_id: &str) -> String;

    /// What another window would have to be given as `--remote` to reach the same repos.
    /// `None` for a backend that reads this machine, which needs no address at all.
    ///
    /// The full address rather than [`Backend::describe`]'s label: that one has its scheme
    /// trimmed off for reading, and a window opened on it would fall back to plain HTTP.
    fn connect_target(&self) -> Option<String>;

    fn open_session(&self, request: OpenSessionRequest) -> Result<SessionOpened>;
    fn session_state(&self, session_id: &str) -> Result<SessionPayload>;
    fn session_submodules(&self, session_id: &str) -> Result<Vec<SubmoduleView>>;
    fn commit_history(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<CommitHistoryPayload>;

    fn set_agent(&self, session_id: &str, agent: AgentKind) -> Result<()>;
    fn set_active_commit(&self, session_id: &str, commit: Option<String>) -> Result<()>;

    fn hunk_patch(&self, session_id: &str, hunk_id: &str) -> Result<PatchPayload>;
    fn file_content(&self, session_id: &str, file_path: &str) -> Result<FileContentPayload>;
    /// The files of the repo whose names match a search, for the palette's file finder.
    fn find_files(&self, session_id: &str, query: &str) -> Result<FileMatchesPayload>;
    /// The lines of the repo that hold what was typed, for the palette's content search.
    fn search_contents(&self, session_id: &str, query: &str) -> Result<ContentMatchesPayload>;
    fn write_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()>;

    fn set_comment(&self, session_id: &str, request: CommentRequest) -> Result<()>;
    fn resolve_comment(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()>;
    fn send_comment_batch(&self, session_id: &str) -> Result<()>;
    fn cancel_dispatch(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()>;
    fn dispatch_log(&self, session_id: &str, dispatch_key: &str) -> Result<AgentLogPayload>;

    fn set_reviewed(&self, session_id: &str, hunk_id: &str, reviewed: Option<bool>) -> Result<()>;
    fn set_file_reviewed(&self, session_id: &str, file_path: &str, reviewed: bool) -> Result<()>;

    fn stage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn unstage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn stage_file(&self, session_id: &str, file_path: &str) -> Result<()>;
    /// Stage the whole working tree at once, which is the commit pane's one staging action.
    fn stage_all(&self, session_id: &str) -> Result<()>;
    fn unstage_file(&self, session_id: &str, file_path: &str) -> Result<()>;
    fn discard_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()>;
    fn discard_hunks(&self, session_id: &str, hunk_ids: &[String]) -> Result<()>;

    /// The moontasks board of the repo this session reviews, and what running it.
    fn list_tasks(&self, session_id: &str) -> Result<Vec<TaskView>>;
    fn create_task(&self, session_id: &str, request: &CreateTaskRequest) -> Result<TaskView>;
    /// Put a task in a column, at a place among the cards already there, which is what a
    /// drag on the board does, and the only way a card moves.
    fn place_task(
        &self,
        session_id: &str,
        task_id: &str,
        status: ColumnId,
        position: usize,
    ) -> Result<()>;
    fn delete_task(&self, session_id: &str, task_id: &str) -> Result<()>;
    /// Start a shell or an agent in a task, and answer with the shell it runs in.
    fn start_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: StartResourceRequest,
    ) -> Result<String>;
    fn resume_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<String>;
    /// The sessions the installed agents already have for this repo, newest first - what the
    /// attach modal lists.
    fn list_agent_sessions(&self, session_id: &str) -> Result<Vec<AgentSessionView>>;
    /// Put one of those sessions on a task as a new resource, and answer with the shell it
    /// was opened in.
    fn attach_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: &AttachResourceRequest,
    ) -> Result<String>;
    fn stop_task_resource(&self, session_id: &str, task_id: &str, resource_id: &str)
    -> Result<()>;
    /// Take a run off the task for good, rather than leaving it to be resumed.
    fn delete_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()>;
    fn rename_task(&self, session_id: &str, task_id: &str, title: &str) -> Result<()>;
    /// Make sure the task's notes file exists, and answer with the repo-relative path a file
    /// pane opens it by. Editing then goes through [`Backend::write_file`] like any file.
    fn open_task_notes(&self, session_id: &str, task_id: &str) -> Result<String>;
    /// Put a file of the repo on the task's card, by its path relative to the repo root.
    fn link_task_file(&self, session_id: &str, task_id: &str, file_path: &str) -> Result<()>;

    /// The board's columns, left to right. A board that has never had them changed answers
    /// with the three defaults.
    fn list_columns(&self, session_id: &str) -> Result<Vec<BoardColumn>>;
    fn add_column(&self, session_id: &str, label: &str) -> Result<BoardColumn>;
    fn rename_column(&self, session_id: &str, column_id: &ColumnId, label: &str) -> Result<()>;
    /// Take an empty column off the board. One still holding cards is refused rather than
    /// taking them with it.
    fn delete_column(&self, session_id: &str, column_id: &ColumnId) -> Result<()>;
    /// Put a column at a place among the others, which is what dragging its heading does. Its
    /// cards go with it, because a card names its column rather than its place on screen.
    fn place_column(&self, session_id: &str, column_id: &ColumnId, position: usize) -> Result<()>;

    /// What the commit pane draws: what is staged, and where a push would send it.
    fn commit_state(&self, session_id: &str) -> Result<CommitState>;
    /// A commit message written from what is staged, by the agent that writes them. Blocks for
    /// as long as that takes, which is seconds rather than milliseconds.
    fn suggest_commit_message(&self, session_id: &str) -> Result<CommitSuggestion>;
    /// Start `git` on one action in a pty, and answer with the shell it runs in. Attached with
    /// [`Backend::attach_terminal`], like any other.
    fn start_commit_run(&self, session_id: &str, action: &CommitAction) -> Result<String>;
    /// The exit code of a run that has ended, `None` while it is still going. Answered once:
    /// the pane asks when the pty closes, and acts on what it gets.
    fn commit_run_outcome(&self, session_id: &str, terminal_id: &str) -> Result<Option<i32>>;

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String>;
    fn list_terminals(&self, session_id: &str) -> Result<Vec<String>>;
    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()>;
    /// Attach to a shell: everything it has printed, and a handle to type into it. This is
    /// what a terminal pane is built from - see [`egui_tty::TtyStream`].
    fn attach_terminal(&self, session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream>;
}
