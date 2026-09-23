//! Reviews a repo on another machine, over the HTTP API its `moonreview serve` answers.
//!
//! `moonreview serve` on the far side is the whole server contract, so no extra daemon is
//! involved.

mod transport;

use transport::{RemoteShell, set_read_timeout, urlencode, websocket_url};

use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::json;

use crate::{
    api::{
        AgentKind, AgentLogPayload, BlameOf, BlamePayload, CommentRequest, CommitHistoryPayload,
        ContentMatch, FileContentPayload, LspCompletion, LspCompletionsPayload, LspDocumentRequest,
        LspLocation, LspLocationsPayload, LspPosition, LspPositionRequest, LspStatus,
        LspStatusPayload, LspTriggersPayload, LspWork, LspWorkPayload, OpenSessionRequest,
        PatchPayload, SearchScope, SessionOpened, SessionPayload, SubmoduleHubPayload,
        TerminalNameRequest, TerminalView,
    },
    backend::Backend,
    moontasks::{
        AttachResourceRequest, BoardColumn, ColumnId, ColumnLabelRequest, ColumnPlacementRequest,
        CreateTaskRequest, LinkFileRequest, NewColumnRequest, StartResourceRequest,
        TaskNotesPayload, TaskPlacementRequest, TaskView, TerminalOpened, WorkLogPayload,
        explainer::ExplainRequest,
    },
    project::{ProjectCommand, ProjectConfig},
    search::SearchListener,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a streamed search may take: as long as `ag` needs on a large tree, which is not
/// the half minute a plain request gets. A search nobody wants is stopped long before.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(crate) struct RemoteBackend {
    /// Base URL of the remote server, without a trailing slash, e.g. `http://dev-box:42000`.
    base_url: String,
    label: String,
    client: Client,
}

impl Backend for RemoteBackend {
    fn describe(&self) -> String {
        self.label.clone()
    }

    fn reads_this_machine(&self) -> bool {
        false
    }

    fn connect_target(&self) -> Option<String> {
        Some(self.base_url.clone())
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

    fn create_task(&self, session_id: &str, request: &CreateTaskRequest) -> Result<TaskView> {
        self.post_json(&format!("/api/session/{session_id}/tasks"), request)
    }

    fn place_tasks(
        &self,
        session_id: &str,
        task_ids: &[String],
        status: ColumnId,
        position: usize,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/placement"),
            &TaskPlacementRequest {
                task_ids: task_ids.to_vec(),
                status,
                position,
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

    fn project_commands(&self, session_id: &str) -> Result<ProjectConfig> {
        self.get(&format!("/api/session/{session_id}/project"))
    }

    fn set_project_config(&self, session_id: &str, commands: &ProjectConfig) -> Result<()> {
        self.post(&format!("/api/session/{session_id}/project"), commands)
    }

    fn run_project_command(&self, session_id: &str, which: ProjectCommand) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/project/run/{}", which.token()),
            &json!({}),
        )?;
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

    fn start_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: StartResourceRequest,
    ) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/resources"),
            &request,
        )?;
        Ok(opened.terminal_id)
    }

    fn explain_task_changes(
        &self,
        session_id: &str,
        task_id: &str,
        request: ExplainRequest,
    ) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/explanation"),
            &request,
        )?;
        Ok(opened.terminal_id)
    }

    fn resume_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/resume"),
            &json!({}),
        )?;
        Ok(opened.terminal_id)
    }

    fn list_agent_sessions(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::agent_sessions::AgentSessionView>> {
        self.get(&format!("/api/session/{session_id}/agent-sessions"))
    }

    fn attach_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        request: &AttachResourceRequest,
    ) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/resources/attach"),
            request,
        )?;
        Ok(opened.terminal_id)
    }

    fn stop_task_resource(&self, session_id: &str, task_id: &str, resource_id: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/stop"),
            &json!({}),
        )
    }

    fn delete_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()> {
        self.delete(&format!(
            "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}"
        ))
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

    fn link_task_file(&self, session_id: &str, task_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/files"),
            &LinkFileRequest {
                file_path: file_path.to_string(),
            },
        )
    }

    fn stage_all(&self, session_id: &str) -> Result<()> {
        self.post(&format!("/api/session/{session_id}/stage-all"), &json!({}))
    }

    fn commit_state(&self, session_id: &str) -> Result<crate::committing::CommitState> {
        self.get(&format!("/api/session/{session_id}/commit-state"))
    }

    fn suggest_commit_message(
        &self,
        session_id: &str,
    ) -> Result<crate::commit_suggestion::CommitSuggestion> {
        self.post_json(
            &format!("/api/session/{session_id}/commit-message"),
            &json!({}),
        )
    }

    fn start_commit_run(
        &self,
        session_id: &str,
        action: &crate::committing::CommitAction,
    ) -> Result<String> {
        let started: crate::api::CommitRunStarted =
            self.post_json(&format!("/api/session/{session_id}/commit-run"), action)?;
        Ok(started.terminal_id)
    }

    fn commit_run_outcome(&self, session_id: &str, terminal_id: &str) -> Result<Option<i32>> {
        let outcome: crate::api::CommitRunOutcome = self.get(&format!(
            "/api/session/{session_id}/commit-run/{terminal_id}/outcome"
        ))?;
        Ok(outcome.exit_code)
    }

    fn run_in_shell(&self, session_id: &str, command: &str) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!("/api/session/{session_id}/run-in-shell"),
            &json!({ "command": command }),
        )?;
        Ok(opened.terminal_id)
    }

    fn create_terminal(&self, session_id: &str, command: Option<AgentKind>) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct Created {
            terminal_id: String,
        }

        let created: Created = self.post_json(
            &format!("/api/session/{session_id}/terminals"),
            &json!({ "command": command }),
        )?;
        Ok(created.terminal_id)
    }

    fn list_terminals(&self, session_id: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct List {
            terminal_ids: Vec<String>,
        }

        let list: List = self.get(&format!("/api/session/{session_id}/terminals"))?;
        Ok(list.terminal_ids)
    }

    fn terminals_running_a_command(&self, session_id: &str) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct List {
            terminal_ids: Vec<String>,
        }

        let list: List = self.get(&format!("/api/session/{session_id}/terminals/running"))?;
        Ok(list.terminal_ids)
    }

    fn terminals_wanting_attention(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::api::TerminalAttentionView>> {
        let list: crate::terminal::TerminalAttentionList =
            self.get(&format!("/api/session/{session_id}/terminals/attention"))?;
        Ok(list.terminals)
    }

    fn terminal_visualizations(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::visualizations::VisualizationView>> {
        let list: crate::visualizations::routes::VisualizationList = self.get(&format!(
            "/api/session/{session_id}/terminals/visualizations"
        ))?;
        Ok(list.visualizations)
    }

    fn visualization_page(&self, session_id: &str, fragment_path: &str) -> Result<String> {
        let encoded = urlencode(fragment_path);
        let page: crate::visualizations::routes::VisualizationPage = self.get(&format!(
            "/api/session/{session_id}/visualizations/page?fragment_path={encoded}"
        ))?;
        Ok(page.html)
    }

    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()> {
        self.delete(&format!(
            "/api/session/{session_id}/terminals/{terminal_id}"
        ))
    }

    fn terminal_name(&self, session_id: &str, terminal_id: &str) -> Result<Option<String>> {
        let view: TerminalView = self.get(&format!(
            "/api/session/{session_id}/terminals/{terminal_id}"
        ))?;
        Ok(view.name)
    }

    fn rename_terminal(&self, session_id: &str, terminal_id: &str, name: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/terminals/{terminal_id}/name"),
            &TerminalNameRequest {
                name: name.to_string(),
            },
        )
    }

    fn lsp_status(&self, session_id: &str, file_path: &str) -> Result<LspStatus> {
        let encoded = urlencode(file_path);
        let payload: LspStatusPayload = self.get(&format!(
            "/api/session/{session_id}/lsp/status?file_path={encoded}"
        ))?;
        Ok(payload.status)
    }

    fn lsp_trigger_characters(&self, session_id: &str, file_path: &str) -> Result<Vec<char>> {
        let encoded = urlencode(file_path);
        let payload: LspTriggersPayload = self.get(&format!(
            "/api/session/{session_id}/lsp/triggers?file_path={encoded}"
        ))?;
        Ok(payload.triggers)
    }

    fn lsp_working(&self, session_id: &str) -> Result<Vec<LspWork>> {
        let payload: LspWorkPayload =
            self.get(&format!("/api/session/{session_id}/lsp/working"))?;
        Ok(payload.working)
    }

    fn lsp_did_open(&self, session_id: &str, file_path: &str, text: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/lsp/open"),
            &LspDocumentRequest {
                file_path: file_path.to_string(),
                text: text.to_string(),
            },
        )
    }

    fn lsp_did_change(&self, session_id: &str, file_path: &str, text: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/lsp/change"),
            &LspDocumentRequest {
                file_path: file_path.to_string(),
                text: text.to_string(),
            },
        )
    }

    fn lsp_did_close(&self, session_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/lsp/close"),
            &json!({ "file_path": file_path }),
        )
    }

    fn lsp_places(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
        which: crate::api::LspPlaces,
    ) -> Result<Vec<LspLocation>> {
        let payload: LspLocationsPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/places"),
            &crate::api::LspPlacesRequest {
                file_path: file_path.to_string(),
                at,
                which,
            },
        )?;
        Ok(payload.locations)
    }

    fn lsp_completion(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Vec<LspCompletion>> {
        let payload: LspCompletionsPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/completion"),
            &LspPositionRequest {
                file_path: file_path.to_string(),
                at,
            },
        )?;
        Ok(payload.completions)
    }

    fn lsp_prepare_rename(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<String>> {
        let payload: crate::api::LspRenamablePayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/prepare-rename"),
            &LspPositionRequest {
                file_path: file_path.to_string(),
                at,
            },
        )?;
        Ok(payload.name)
    }

    fn lsp_rename(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
        new_name: &str,
    ) -> Result<Vec<crate::api::LspFileEdit>> {
        let payload: crate::api::LspFileEditsPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/rename"),
            &crate::api::LspRenameRequest {
                file_path: file_path.to_string(),
                at,
                new_name: new_name.to_string(),
            },
        )?;
        Ok(payload.files)
    }

    fn lsp_format(
        &self,
        session_id: &str,
        file_path: &str,
        options: moon_lsp::LspFormatting,
    ) -> Result<Vec<moon_lsp::LspTextEdit>> {
        let payload: crate::api::LspTextEditsPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/format"),
            &crate::api::LspFormatRequest {
                file_path: file_path.to_string(),
                options,
            },
        )?;
        Ok(payload.edits)
    }

    fn lsp_hover(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<String>> {
        let payload: crate::api::LspHoverPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/hover"),
            &LspPositionRequest {
                file_path: file_path.to_string(),
                at,
            },
        )?;
        Ok(payload.markdown)
    }

    fn lsp_diagnostics(
        &self,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<moon_lsp::LspDiagnostic>> {
        let encoded = urlencode(file_path);
        let payload: crate::api::LspDiagnosticsPayload = self.get(&format!(
            "/api/session/{session_id}/lsp/diagnostics?file_path={encoded}"
        ))?;
        Ok(payload.diagnostics)
    }

    fn lsp_did_save(&self, session_id: &str, file_path: &str) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/lsp/save"),
            &json!({ "file_path": file_path }),
        )
    }

    fn lsp_code_actions(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Vec<moon_lsp::LspCodeAction>> {
        let payload: crate::api::LspCodeActionsPayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/code-actions"),
            &LspPositionRequest {
                file_path: file_path.to_string(),
                at,
            },
        )?;
        Ok(payload.actions)
    }

    fn lsp_signature_help(
        &self,
        session_id: &str,
        file_path: &str,
        at: LspPosition,
    ) -> Result<Option<moon_lsp::LspSignature>> {
        let payload: crate::api::LspSignaturePayload = self.post_json(
            &format!("/api/session/{session_id}/lsp/signature"),
            &LspPositionRequest {
                file_path: file_path.to_string(),
                at,
            },
        )?;
        Ok(payload.signature)
    }

    fn attach_terminal(&self, session_id: &str, terminal_id: &str) -> Result<egui_tty::TtyStream> {
        let url = websocket_url(
            &self.base_url,
            &format!("/api/session/{session_id}/terminals/{terminal_id}/socket"),
        );
        let (socket, _) = tungstenite::connect(&url)
            .with_context(|| format!("failed to attach to the remote shell at {url}"))?;
        set_read_timeout(&socket)?;

        let socket = Arc::new(Mutex::new(socket));
        let (sender, output) = mpsc::channel();
        let reader_socket = Arc::clone(&socket);

        thread::spawn(move || {
            loop {
                let message = {
                    let Ok(mut socket) = reader_socket.lock() else {
                        return;
                    };
                    match socket.read() {
                        Ok(message) => Some(message),
                        // A read timeout is how the writer gets the lock between frames.
                        Err(tungstenite::Error::Io(error))
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            None
                        }
                        Err(_) => return,
                    }
                };

                let Some(message) = message else {
                    thread::sleep(Duration::from_millis(4));
                    continue;
                };

                let chunk = match message {
                    tungstenite::Message::Binary(bytes) => bytes.to_vec(),
                    tungstenite::Message::Text(text) => text.as_bytes().to_vec(),
                    tungstenite::Message::Close(_) => return,
                    _ => continue,
                };
                if sender.send(chunk).is_err() {
                    return;
                }
            }
        });

        Ok(egui_tty::TtyStream {
            output,
            tty: Arc::new(RemoteShell { socket }),
        })
    }
}

