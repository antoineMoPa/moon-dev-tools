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
    backend::{Backend, TerminalAttachment, TerminalInput},
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

struct LocalTerminalInput {
    session: Arc<TerminalSession>,
}

impl TerminalInput for LocalTerminalInput {
    fn write(&self, data: &[u8]) -> Result<()> {
        self.session.write_input(data)
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.session.resize(cols, rows)
    }

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

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String> {
        let repo_path = crate::api::with_session(&self.state, session_id, |session| {
            Ok(session.repo_path.clone())
        })?;
        self.state.terminals.spawn(&repo_path, command)
    }

    fn list_terminals(&self, _session_id: &str) -> Result<Vec<String>> {
        Ok(self.state.terminals.terminal_ids())
    }

    fn close_terminal(&self, _session_id: &str, terminal_id: &str) -> Result<()> {
        self.state.terminals.remove(terminal_id);
        Ok(())
    }

    fn attach_terminal(&self, _session_id: &str, terminal_id: &str) -> Result<TerminalAttachment> {
        let (output, session) = self.state.terminals.attach(terminal_id)?;
        Ok(TerminalAttachment {
            output,
            input: Arc::new(LocalTerminalInput { session }),
        })
    }
}
