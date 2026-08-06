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
    api::{
        AgentKind, AgentLogPayload, CommentRequest, CommitHistoryPayload, FileContentPayload,
        OpenSessionRequest, PatchPayload, SessionOpened, SessionPayload, SubmoduleView,
    },
    moontasks::{CreateTaskRequest, StartResourceRequest, TaskStatus, TaskView},
};

/// Every review operation the native frontend performs. Calls block, so the UI runs them
/// on worker threads — a remote backend is a network round-trip.
pub(crate) trait Backend: Send + Sync + 'static {
    /// How this connection reads in the UI, e.g. `local` or `dev-box:42000`.
    fn describe(&self) -> String;

    /// Where a browser can open the same review, for the "open in browser" action.
    fn web_url(&self, session_id: &str) -> String;

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
    fn set_task_status(&self, session_id: &str, task_id: &str, status: TaskStatus) -> Result<()>;
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
    fn stop_task_resource(&self, session_id: &str, task_id: &str, resource_id: &str)
    -> Result<()>;

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String>;
    fn list_terminals(&self, session_id: &str) -> Result<Vec<String>>;
    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()>;
    /// Attach to a shell: everything it has printed, and a handle to type into it. This is
    /// what a terminal pane is built from — see [`egui_tty::TtyStream`].
    fn attach_terminal(&self, session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream>;
}
