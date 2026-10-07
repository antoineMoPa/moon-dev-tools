//! Agent commands - handles `moon agent start`, `tell` and `view` when they arrive in the
//! window, on the thread that reads its socket - see [`AgentAsks`] for why not on a frame.
//!
//! All three are the window's moon's to do, which is the backend: it starts a task's runs and
//! holds their shells. The window draws nothing for them. A run started here turns up on the
//! board the next time the board reads its tasks, as one started from its own `[start]` does,
//! and its tab is opened from the card.

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::{
    api::{AgentKind, OpenSessionRequest},
    backend::Backend,
    instances::window::AgentAsks,
    moontasks::{StartFolder, StartResourceRequest, TaskResourceKind},
    terminal::Shown,
};

/// The agents of this window's moon, as a shell asks after them.
pub(crate) struct WindowAgents {
    pub(crate) backend: Arc<dyn Backend>,
}

impl AgentAsks for WindowAgents {
    /// The agent comes up in the repo, as one started from the card does, but unattended:
    /// nobody is at the window to send the card's title or to say the folder is trusted, and
    /// the next thing a shell that started an agent does is tell it something.
    ///
    /// The session is the one on the board's repo and the whole of its working tree - the
    /// same one however many times it is opened, and the window's own when it was opened on
    /// the repo with nothing narrowing its review.
    fn start(&self, repo_path: &str, task_id: &str, agent: AgentKind) -> Result<String> {
        let session_id = self
            .backend
            .open_session(OpenSessionRequest {
                repo_path: repo_path.to_string(),
                diff_target: None,
                active_commit: None,
            })?
            .session_id;
        let terminal_id = self.backend.start_task_resource(
            &session_id,
            task_id,
            StartResourceRequest {
                kind: TaskResourceKind::Agent,
                agent,
                opens_in: StartFolder::Repo,
                unattended: true,
            },
        )?;
        self.backend
            .terminal_name(&session_id, &terminal_id)?
            .context("the run was started with no name")
    }

    fn tell(&self, terminal_id: &str, line: &str) -> Result<()> {
        self.backend.tell_terminal(terminal_id, line)
    }

    fn shown(&self, terminal_id: &str, wanted: Shown) -> Result<String> {
        self.backend.terminal_shown(terminal_id, wanted)
    }
}
