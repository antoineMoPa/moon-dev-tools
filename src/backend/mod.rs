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
        OpenSessionRequest, PatchPayload, SessionOpened, SessionPayload, SubmoduleView,
    },
    moontasks::{
        AttachResourceRequest, BoardColumn, ColumnName, CreateTaskRequest, StartResourceRequest,
        TaskReviewPayload, TaskView, TaskWorktreeView,
    },
};

/// Every review operation the native frontend performs. Calls block, so the UI runs them
/// on worker threads — a remote backend is a network round-trip.
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
        status: ColumnName,
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
    /// The sessions the installed agents already have for this repo, newest first — what the
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
    fn stop_task_resource(&self, session_id: &str, task_id: &str, resource_id: &str) -> Result<()>;
    /// Take a run off the task for good, rather than leaving it to be resumed.
    fn delete_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()>;
    fn rename_task(&self, session_id: &str, task_id: &str, title: &str) -> Result<()>;
    /// What a card is marked with, set whole. Tags are how a person says which cards an
    /// autopilot window may work.
    fn set_task_tags(&self, session_id: &str, task_id: &str, tags: &[String]) -> Result<()>;
    /// Give a task a checkout of its own, on a branch named after it. Everything the task
    /// starts from then on runs there instead of in the repo.
    fn create_task_worktree(&self, session_id: &str, task_id: &str) -> Result<TaskWorktreeView>;
    /// Give that checkout back. `force` takes uncommitted work in it with it.
    fn discard_task_worktree(&self, session_id: &str, task_id: &str, force: bool) -> Result<()>;
    /// Prepare a task's work for review.
    fn review_task(&self, session_id: &str, task_id: &str) -> Result<TaskReviewPayload>;
    /// Whether the board acts on itself: hooks fire, work is picked up, cards move on their
    /// own. This is what the play/pause control on the board reads and writes.
    fn hooks_running(&self, session_id: &str) -> Result<bool>;
    fn set_hooks_running(&self, session_id: &str, running: bool) -> Result<()>;
    /// Make sure the task's notes file exists, and answer with the repo-relative path a file
    /// pane opens it by. Editing then goes through [`Backend::write_file`] like any file.
    fn open_task_notes(&self, session_id: &str, task_id: &str) -> Result<String>;
    /// Make sure the board has its hooks, and answer with the repo-relative path of the one
    /// autopilot is, so it can be opened in a pane and edited like any other file.
    fn open_autopilot_script(&self, session_id: &str) -> Result<String>;
    /// Where the board wrote down what it decided, for a pane to read.
    fn open_hooks_log(&self, session_id: &str) -> Result<String>;

    /// The board's columns, left to right. A board that has never had them changed answers
    /// with the three defaults.
    fn list_columns(&self, session_id: &str) -> Result<Vec<BoardColumn>>;
    fn add_column(&self, session_id: &str, name: &str) -> Result<BoardColumn>;
    fn rename_column(&self, session_id: &str, column: &ColumnName, name: &str) -> Result<()>;
    /// Take an empty column off the board. One still holding cards is refused rather than
    /// taking them with it.
    fn delete_column(&self, session_id: &str, column: &ColumnName) -> Result<()>;
    /// Put a column at a place among the others, which is what dragging its heading does. Its
    /// cards go with it, because a card names its column rather than its place on screen.
    fn place_column(&self, session_id: &str, column: &ColumnName, position: usize) -> Result<()>;

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String>;
    fn list_terminals(&self, session_id: &str) -> Result<Vec<String>>;
    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()>;
    /// Attach to a shell: everything it has printed, and a handle to type into it. This is
    /// what a terminal pane is built from — see [`egui_tty::TtyStream`].
    fn attach_terminal(&self, session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream>;
}
