//! Remote backend - reviews a repo on another machine, over its `moonreview serve` HTTP API.
//!
//! `moonreview serve` on the far side is the whole server contract, so no extra daemon is
//! involved.

mod addresses;
mod running;
// The same methods either way: blocking reqwest and tungstenite in a program of its own, and
// the browser's own requests and sockets in the build for the browser.
#[cfg(not(target_arch = "wasm32"))]
mod transport;
#[cfg(target_arch = "wasm32")]
#[path = "remote/transport_web.rs"]
mod transport;
#[cfg(target_arch = "wasm32")]
pub(crate) mod rounds;

// The server spells repo paths into the addresses it redirects to the same way.
pub(crate) use addresses::urlencode;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use transport::PASS_KEY_ENV_VAR;
use addresses::websocket_url;

use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::{
    api::{
        AgentKind, AgentLogPayload, BlameOf, BlamePayload, CommentRequest, RunningAgent, CommitHistoryPayload,
        ContentMatch, FileContentPayload, LspCompletion, LspCompletionsPayload, LspDocumentRequest,
        LspLocation, LspLocationsPayload, LspPosition, LspPositionRequest, LspStatus,
        LspStatusPayload, LspTriggersPayload, LspWork, LspWorkPayload, OpenSessionRequest,
        PatchPayload, SearchScope, SessionOpened, SessionPayload, SubmoduleHubPayload,
        TerminalNameRequest, TerminalView,
    },
    backend::{Backend, SearchListener},
    moontasks::{
        AttachResourceRequest, BoardColumn, BoardTaskView, ColumnId, ColumnLabelRequest,
        ColumnPlacementRequest, CreateTaskRequest, LinkFileRequest, NewColumnRequest,
        ReviewRequestView, StartResourceRequest, TaskNotesPayload, TaskPlacementRequest, TaskView,
        TerminalOpened, WirePayload, WorkLogPayload, review_request::Amend,
    },
    palette_history::{HistoryEntry, HistoryRecord},
    project::{ProjectConfig, ProjectRunRequest},
    settings::{Settings, SettingsChange},
};

pub(crate) struct RemoteBackend {
    /// Base URL of the remote server, without a trailing slash, e.g. `http://dev-box:42000`.
    base_url: String,
    label: String,
    connection: transport::Connection,
}

impl Backend for RemoteBackend {
    fn describe(&self) -> String {
        self.label.clone()
    }

    fn reads_this_machine(&self) -> bool {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn connect_target(&self) -> Option<crate::backend::ConnectTarget> {
        Some(crate::backend::ConnectTarget {
            address: self.base_url.clone(),
            pass_key: self.connection.pass_key.clone(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn mint_pass_key(&self) -> Result<String> {
        let minted: crate::api::PassKeyMinted = self.post_json("/api/pass-key", &json!({}))?;
        Ok(minted.pass_key)
    }

    fn mint_login_ticket(&self, lifetime: std::time::Duration) -> Result<String> {
        let minted: crate::api::LoginTicketMinted = self.post_json(
            "/api/login-ticket",
            &crate::api::LoginTicketRequest {
                lifetime_seconds: lifetime.as_secs(),
            },
        )?;
        Ok(minted.ticket)
    }

    fn open_session(&self, request: OpenSessionRequest) -> Result<SessionOpened> {
        self.post_json("/api/session/open", &request)
    }

    fn session_state(&self, session_id: &str) -> Result<SessionPayload> {
        self.get(&format!("/api/session/{session_id}/state"))
    }

    fn session_submodules(&self, session_id: &str) -> Result<SubmoduleHubPayload> {
        self.get(&format!("/api/session/{session_id}/submodules"))
    }

    fn commit_history(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<CommitHistoryPayload> {
        self.get(&format!(
            "/api/session/{session_id}/history?offset={offset}&limit={limit}"
        ))
    }

    fn set_agent(&self, session_id: &str, agent: AgentKind) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/agent"),
            &json!({ "agent": agent }),
        )
    }

    fn set_active_commit(&self, session_id: &str, commit: Option<String>) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/commit"),
            &json!({ "commit": commit }),
        )
    }

    fn hunk_patch(&self, session_id: &str, hunk_id: &str) -> Result<PatchPayload> {
        self.get(&format!("/api/session/{session_id}/hunk/{hunk_id}"))
    }

    fn write_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/file"),
            &json!({ "file_path": file_path, "content": content }),
        )
    }

    fn create_file(&self, session_id: &str, file_path: &str, content: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/file/new"),
            &json!({ "file_path": file_path, "content": content }),
        )
    }

    fn decrypted_file_content(
        &self,
        session_id: &str,
        request: &crate::api::encrypted_files::DecryptFileRequest,
    ) -> Result<FileContentPayload> {
        self.post_json(&format!("/api/session/{session_id}/file/decrypt"), request)
    }

    fn write_encrypted_file(
        &self,
        session_id: &str,
        request: &crate::api::encrypted_files::WriteEncryptedFileRequest,
    ) -> Result<()> {
        self.post(&format!("/api/session/{session_id}/file/encrypted"), request)
    }

    fn list_folder(&self, folder: &str) -> Result<crate::api::folders::FolderListing> {
        let encoded = urlencode(folder);
        self.get(&format!("/api/folder?path={encoded}"))
    }

    fn create_project_folder(&self, path: &str) -> Result<()> {
        self.post(
            "/api/folder/new-project",
            &crate::api::folders::NewProjectFolderRequest {
                path: path.to_string(),
            },
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn place_file(&self, path: &str) -> Result<crate::api::folders::FilePlaced> {
        self.post_json(
            "/api/session/place-file",
            &crate::api::folders::PlaceFileRequest {
                path: path.to_string(),
            },
        )
    }

    fn file_content(&self, session_id: &str, file_path: &str) -> Result<FileContentPayload> {
        let encoded = urlencode(file_path);
        self.get(&format!(
            "/api/session/{session_id}/file?file_path={encoded}"
        ))
    }

    fn file_content_at(
        &self,
        session_id: &str,
        file_path: &str,
        revision: &str,
    ) -> Result<FileContentPayload> {
        let encoded = urlencode(file_path);
        let revision = urlencode(revision);
        self.get(&format!(
            "/api/session/{session_id}/file-at?file_path={encoded}&revision={revision}"
        ))
    }

    fn blame_file(&self, session_id: &str, file_path: &str, of: &BlameOf) -> Result<BlamePayload> {
        // Posted rather than fetched: a text asked about goes with the question, and a
        // file's worth of it has no place in a URL.
        self.post_json(
            &format!("/api/session/{session_id}/blame"),
            &json!({ "file_path": file_path, "of": of }),
        )
    }

    fn find_files(
        &self,
        session_id: &str,
        query: &str,
        scope: SearchScope,
        listener: &mut dyn SearchListener<String>,
    ) -> Result<()> {
        let encoded = urlencode(query);
        let include_ignored = scope.includes_ignored();
        self.stream_search(
            &format!(
                "/api/session/{session_id}/files?query={encoded}&include_ignored={include_ignored}"
            ),
            listener,
        )
    }

    fn search_contents(
        &self,
        session_id: &str,
        query: &str,
        scope: SearchScope,
        listener: &mut dyn SearchListener<ContentMatch>,
    ) -> Result<()> {
        let encoded = urlencode(query);
        let include_ignored = scope.includes_ignored();
        self.stream_search(
            &format!(
                "/api/session/{session_id}/content?query={encoded}&include_ignored={include_ignored}"
            ),
            listener,
        )
    }

    fn set_comment(&self, session_id: &str, request: CommentRequest) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/comment"),
            &json!({
                "hunk_id": request.hunk_id,
                "comment": request.comment,
                "batch": request.batch,
                "patch_excerpt": request.patch_excerpt,
            }),
        )
    }

    fn resolve_comment(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()> {
        self.get_ok(&format!(
            "/api/session/{session_id}/resolve/{hunk_id}/{comment_index}"
        ))
    }

    fn send_comment_batch(&self, session_id: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/comment-batch"),
            &json!({}),
        )
    }

    fn tells_running_agents(&self) -> bool {
        false
    }

    fn tell_comment(
        &self,
        _session_id: &str,
        _request: CommentRequest,
        agent: &RunningAgent,
    ) -> Result<()> {
        anyhow::bail!(
            "{} is on {}, which this window cannot type into",
            agent.name,
            self.base_url
        )
    }

    fn tell_held_comments(&self, _session_id: &str, agent: &RunningAgent) -> Result<()> {
        anyhow::bail!(
            "{} is on {}, which this window cannot type into",
            agent.name,
            self.base_url
        )
    }

    fn cancel_dispatch(&self, session_id: &str, hunk_id: &str, comment_index: usize) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/comment-dispatch/cancel"),
            &json!({ "hunk_id": hunk_id, "comment_index": comment_index }),
        )
    }

    fn dispatch_log(&self, session_id: &str, dispatch_key: &str) -> Result<AgentLogPayload> {
        let encoded = urlencode(dispatch_key);
        self.get(&format!(
            "/api/session/{session_id}/agent-dispatch/log?dispatch_key={encoded}"
        ))
    }

    fn stage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/stage"),
            &json!({ "hunk_id": hunk_id }),
        )
    }

    fn unstage_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/unstage"),
            &json!({ "hunk_id": hunk_id }),
        )
    }

    fn stage_file(&self, session_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/stage-file"),
            &json!({ "file_path": file_path }),
        )
    }

    fn unstage_file(&self, session_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/unstage-file"),
            &json!({ "file_path": file_path }),
        )
    }

    fn discard_hunk(&self, session_id: &str, hunk_id: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/discard"),
            &json!({ "hunk_id": hunk_id }),
        )
    }

    fn discard_hunks(&self, session_id: &str, hunk_ids: &[String]) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/discard-batch"),
            &json!({ "hunk_ids": hunk_ids }),
        )
    }

    fn list_tasks(&self, session_id: &str) -> Result<Vec<TaskView>> {
        self.get(&format!("/api/session/{session_id}/tasks"))
    }

    fn board_task(&self, session_id: &str) -> Result<BoardTaskView> {
        self.get(&format!("/api/session/{session_id}/board-task"))
    }

    fn create_task(&self, session_id: &str, request: &CreateTaskRequest) -> Result<TaskView> {
        self.post_json(&format!("/api/session/{session_id}/tasks"), request)
    }

    fn place_tasks(
        &self,
        session_id: &str,
        task_ids: &[String],
        status: ColumnId,
        position: usize,
        parent: Option<String>,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/placement"),
            &TaskPlacementRequest {
                task_ids: task_ids.to_vec(),
                status,
                position,
                parent,
            },
        )
    }

    fn delete_task(&self, session_id: &str, task_id: &str) -> Result<()> {
        self.delete(&format!("/api/session/{session_id}/tasks/{task_id}"))
    }

    fn list_columns(&self, session_id: &str) -> Result<Vec<BoardColumn>> {
        self.get(&format!("/api/session/{session_id}/columns"))
    }

    fn add_column(&self, session_id: &str, label: &str, at: Option<usize>) -> Result<BoardColumn> {
        self.post_json(
            &format!("/api/session/{session_id}/columns"),
            &NewColumnRequest {
                label: label.to_string(),
                at,
            },
        )
    }

    fn rename_column(&self, session_id: &str, column_id: &ColumnId, label: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/columns/{column_id}/title"),
            &ColumnLabelRequest {
                label: label.to_string(),
            },
        )
    }

    fn set_column_arrivals(
        &self,
        session_id: &str,
        column_id: &ColumnId,
        arrivals: Option<crate::moontasks::ColumnEnd>,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/columns/{column_id}/arrivals"),
            &crate::moontasks::ColumnArrivalsRequest { arrivals },
        )
    }

    fn set_column_sort(
        &self,
        session_id: &str,
        column_id: &ColumnId,
        sort: Option<crate::moontasks::ColumnSort>,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/columns/{column_id}/sort"),
            &crate::moontasks::ColumnSortRequest { sort },
        )
    }

    fn set_column_marks_a_days_work(
        &self,
        session_id: &str,
        column_id: &ColumnId,
        marks_a_days_work: bool,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/columns/{column_id}/days-work"),
            &crate::moontasks::ColumnDaysWorkRequest { marks_a_days_work },
        )
    }

    fn list_review_requests(&self, session_id: &str) -> Result<Vec<ReviewRequestView>> {
        self.get(&format!("/api/session/{session_id}/review-requests"))
    }

    fn amend_review_request(
        &self,
        session_id: &str,
        task_id: &str,
        index: usize,
        amend: Amend,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/review-requests/{index}"),
            &amend,
        )
    }

    fn settings(&self) -> Result<Settings> {
        self.get("/api/settings")
    }

    fn change_settings(&self, change: SettingsChange) -> Result<()> {
        self.post("/api/settings", &change)
    }

    fn palette_history(&self, session_id: &str) -> Result<Vec<HistoryEntry>> {
        self.get(&format!("/api/session/{session_id}/palette-history"))
    }

    fn record_in_palette_history(&self, session_id: &str, record: &HistoryRecord) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/palette-history"),
            record,
        )
    }

    fn opencode_models(&self) -> Result<Vec<String>> {
        self.get("/api/opencode-models")
    }

    fn project_commands(&self, session_id: &str) -> Result<ProjectConfig> {
        self.get(&format!("/api/session/{session_id}/project"))
    }

    fn set_project_config(&self, session_id: &str, commands: &ProjectConfig) -> Result<()> {
        self.post(&format!("/api/session/{session_id}/project"), commands)
    }

    fn run_project_command(&self, session_id: &str, request: &ProjectRunRequest) -> Result<String> {
        let opened: TerminalOpened =
            self.post_json(&format!("/api/session/{session_id}/project/run"), request)?;
        Ok(opened.terminal_id)
    }

    fn delete_column(&self, session_id: &str, column_id: &ColumnId) -> Result<()> {
        self.delete(&format!("/api/session/{session_id}/columns/{column_id}"))
    }

    fn place_column(&self, session_id: &str, column_id: &ColumnId, position: usize) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/columns/{column_id}/placement"),
            &ColumnPlacementRequest { position },
        )
    }

    fn rename_task(&self, session_id: &str, task_id: &str, title: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/title"),
            &json!({ "title": title }),
        )
    }

    fn set_task_tags(&self, session_id: &str, task_id: &str, tags: &[String]) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/tags"),
            &json!({ "tags": tags }),
        )
    }

    fn set_task_remote_tracker_url(
        &self,
        session_id: &str,
        task_id: &str,
        url: &str,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/remote-tracker"),
            &json!({ "url": url }),
        )
    }

    fn open_task_notes(&self, session_id: &str, task_id: &str) -> Result<String> {
        let notes: TaskNotesPayload = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/notes/open"),
            &json!({}),
        )?;
        Ok(notes.file_path)
    }

    fn open_work_log(&self, session_id: &str) -> Result<String> {
        let work_log: WorkLogPayload =
            self.post_json(&format!("/api/session/{session_id}/work-log/open"), &json!({}))?;
        Ok(work_log.file_path)
    }

    fn open_wire(&self, session_id: &str) -> Result<String> {
        let wire: WirePayload =
            self.post_json(&format!("/api/session/{session_id}/wire/open"), &json!({}))?;
        Ok(wire.file_path)
    }

    fn link_task_file(&self, session_id: &str, task_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/files"),
            &LinkFileRequest {
                file_path: file_path.to_string(),
            },
        )
    }

    fn remove_task_attachment(&self, session_id: &str, task_id: &str, listed: &str) -> Result<()> {
        // In the body rather than the path: a listed document is a path itself, slashes and all.
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/attachments/remove"),
            &crate::moontasks::RemoveAttachmentRequest {
                listed: listed.to_string(),
            },
        )
    }

    fn stage_all(&self, session_id: &str) -> Result<()> {
        self.post(&format!("/api/session/{session_id}/stage-all"), &json!({}))
    }

    running::methods!();
}
