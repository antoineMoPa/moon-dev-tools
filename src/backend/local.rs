//! Reviews the repo in this process. Calls [`crate::service`] straight through, so the
//! window and any browser tab pointed at the embedded server see one shared review.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::{
        AgentKind, AgentLogPayload, AppState, CommentRequest, CommitHistoryPayload,
        FileContentPayload, OpenSessionRequest, PatchPayload, SessionOpened, SessionPayload,
        SubmoduleView, server_url,
    },
    backend::Backend,
    moontasks::{self, CreateTaskRequest, StartResourceRequest, TaskStatus, TaskView},
    service,
    terminal::TerminalSession,
};

pub(crate) struct LocalBackend {
    state: AppState,
}

impl LocalBackend {
    pub(crate) fn new(state: AppState) -> Self {
        Self { state }
    }
}

/// A shell running in this process, as something a terminal pane can type into.
struct LocalShell {
    session: Arc<TerminalSession>,
}

impl egui_tty::Tty for LocalShell {
    fn write(&self, data: &[u8]) -> egui_tty::Result<()> {
        self.session.write_input(data).map_err(egui_tty::Error::msg)
    }

    fn resize(&self, cols: u16, rows: u16) -> egui_tty::Result<()> {
        self.session.resize(cols, rows).map_err(egui_tty::Error::msg)
    }

    /// The pane is what holds this shell's session alive, so its output channel stays open
    /// long after the shell itself is gone. Nothing but the session knows.
    fn has_exited(&self) -> bool {
        self.session.has_exited()
    }
}

impl Backend for LocalBackend {
    fn describe(&self) -> String {
        "local".to_string()
    }

    fn web_url(&self, session_id: &str) -> String {
        format!("{}/review/{session_id}", server_url())
    }

    fn open_session(&self, request: OpenSessionRequest) -> Result<SessionOpened> {
        service::open_session(&self.state, request)
    }

    fn session_state(&self, session_id: &str) -> Result<SessionPayload> {
        service::session_state(&self.state, session_id)
    }

    fn session_submodules(&self, session_id: &str) -> Result<Vec<SubmoduleView>> {
        service::session_submodules(&self.state, session_id)
    }

    fn commit_history(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<CommitHistoryPayload> {
        service::commit_history(&self.state, session_id, offset, limit)
    }

    fn set_agent(&self, session_id: &str, agent: AgentKind) -> Result<()> {
        service::update_agent(&self.state, session_id, agent)
    }

    fn set_active_commit(&self, session_id: &str, commit: Option<String>) -> Result<()> {
        service::update_commit_view(&self.state, session_id, commit)
    }

    fn hunk_patch(&self, session_id: &str, hunk_id: &str) -> Result<PatchPayload> {
        service::hunk_patch(&self.state, session_id, hunk_id)
    }

    fn write_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()> {
        service::write_session_file(&self.state, session_id, file_path, content)
    }

    fn file_content(&self, session_id: &str, file_path: &str) -> Result<FileContentPayload> {
        service::session_file(&self.state, session_id, file_path)
    }

    fn set_comment(&self, session_id: &str, request: CommentRequest) -> Result<()> {
        service::update_comment(&self.state, session_id, &request)
    }

    fn resolve_comment(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()> {
        service::resolve_comment(&self.state, session_id, hunk_id, comment_index)
    }

    fn send_comment_batch(&self, session_id: &str) -> Result<()> {
        service::send_comment_batch(&self.state, session_id)
    }

    fn cancel_dispatch(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()> {
        service::cancel_dispatch(&self.state, session_id, hunk_id, comment_index)
    }

    fn dispatch_log(&self, session_id: &str, dispatch_key: &str) -> Result<AgentLogPayload> {
        service::dispatch_log(&self.state, session_id, dispatch_key)
    }

    fn set_reviewed(&self, session_id: &str, hunk_id: &str, reviewed: Option<bool>) -> Result<()> {
        service::toggle_reviewed(&self.state, session_id, hunk_id, reviewed)
    }

    fn set_file_reviewed(&self, session_id: &str, file_path: &str, reviewed: bool) -> Result<()> {
        service::update_file_reviewed(&self.state, session_id, file_path, reviewed)
    }

    fn stage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        service::stage_hunk(&self.state, session_id, hunk_id)
    }

    fn unstage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        service::unstage_hunk(&self.state, session_id, hunk_id)
    }

    fn stage_file(&self, session_id: &str, file_path: &str) -> Result<()> {
        service::stage_file(&self.state, session_id, file_path)
    }

    fn unstage_file(&self, session_id: &str, file_path: &str) -> Result<()> {
        service::unstage_file(&self.state, session_id, file_path)
    }

    fn discard_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        service::discard_hunk(&self.state, session_id, hunk_id)
    }

    fn discard_hunks(&self, session_id: &str, hunk_ids: &[String]) -> Result<()> {
        service::discard_hunks(&self.state, session_id, hunk_ids)
    }

    fn list_tasks(&self, session_id: &str) -> Result<Vec<TaskView>> {
        moontasks::service::list_tasks(&self.state, session_id)
    }

    fn create_task(&self, session_id: &str, request: &CreateTaskRequest) -> Result<TaskView> {
        moontasks::service::create_task(&self.state, session_id, request)
    }

    fn set_task_status(&self, session_id: &str, task_id: &str, status: TaskStatus) -> Result<()> {
        moontasks::service::set_task_status(&self.state, session_id, task_id, status)
    }

    fn delete_task(&self, session_id: &str, task_id: &str) -> Result<()> {
        moontasks::service::delete_task(&self.state, session_id, task_id)
    }

    fn start_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: StartResourceRequest,
    ) -> Result<String> {
        moontasks::service::start_resource(&self.state, session_id, task_id, request)
    }

    fn resume_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<String> {
        moontasks::service::resume_resource(&self.state, session_id, task_id, resource_id)
    }

    fn stop_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()> {
        moontasks::service::stop_resource(&self.state, session_id, task_id, resource_id)
    }

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String> {
        let repo_path = crate::api::with_session(&self.state, session_id, |session| {
            Ok(session.repo_path.clone())
        })?;
        self.state
            .terminals
            .spawn(crate::terminal::TerminalSpec::shell(repo_path, command))
    }

    fn list_terminals(&self, _session_id: &str) -> Result<Vec<String>> {
        Ok(self.state.terminals.terminal_ids())
    }

    fn close_terminal(&self, _session_id: &str, terminal_id: &str) -> Result<()> {
        self.state.terminals.remove(terminal_id);
        Ok(())
    }

    fn attach_terminal(&self, _session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream> {
        let (output, session) = self.state.terminals.attach(terminal_id)?;
        Ok(egui_tty::TtyStream {
            output,
            tty: Arc::new(LocalShell { session }),
        })
    }
}
