//! The part of [`super::RemoteBackend`]'s [`crate::backend::Backend`] that is about what the
//! far server runs: the board's agents and shells, commit runs, terminals, language servers
//! and desktop applications.
//!
//! A trait is implemented in one block, so these methods are spelled here as a macro that
//! block expands. Names in them resolve where it expands, in `remote.rs`, whose imports they
//! use.

macro_rules! methods {
    () => {
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

        /// The shells are the far server's, and a line reaches a shell through the socket of the
        /// window whose own moon holds it: there is no asking another machine's.
        #[cfg(not(target_arch = "wasm32"))]
        fn tell_terminal(&self, terminal_id: &str, _line: &str) -> Result<()> {
            anyhow::bail!(
                "{terminal_id} is a shell of {}, which this window cannot type a line into",
                self.base_url
            )
        }

        /// The far server's too, and read the same way a line is typed: by the window whose own
        /// moon holds the shell.
        #[cfg(not(target_arch = "wasm32"))]
        fn terminal_shown(&self, terminal_id: &str, _wanted: crate::terminal::Shown) -> Result<String> {
            anyhow::bail!(
                "{terminal_id} is a shell of {}, which this window cannot read the screen of",
                self.base_url
            )
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

        fn create_terminal_in_folder(&self, session_id: &str, folder: &Path) -> Result<String> {
            #[derive(serde::Deserialize)]
            struct Created {
                terminal_id: String,
            }

            let created: Created = self.post_json(
                &format!("/api/session/{session_id}/terminals"),
                &json!({ "command": null, "folder": folder.display().to_string() }),
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
        ) -> Result<crate::api::TerminalAttentionList> {
            self.get(&format!("/api/session/{session_id}/terminals/attention"))
        }

        fn terminal_visualizations(
            &self,
            session_id: &str,
        ) -> Result<Vec<crate::visualizations::VisualizationView>> {
            let list: crate::visualizations::VisualizationList = self.get(&format!(
                "/api/session/{session_id}/terminals/visualizations"
            ))?;
            Ok(list.visualizations)
        }

        fn visualization_page(&self, session_id: &str, fragment_path: &str) -> Result<String> {
            let encoded = urlencode(fragment_path);
            let page: crate::visualizations::VisualizationPage = self.get(&format!(
                "/api/session/{session_id}/visualizations/page?fragment_path={encoded}"
            ))?;
            Ok(page.html)
        }

        fn terminal_conversation(
            &self,
            session_id: &str,
            terminal_id: &str,
        ) -> Result<crate::agent_conversation::AgentConversation> {
            self.get(&format!(
                "/api/session/{session_id}/terminals/{terminal_id}/conversation"
            ))
        }

        fn submit_to_agent(&self, session_id: &str, terminal_id: &str, text: &str) -> Result<()> {
            self.post(
                &format!("/api/session/{session_id}/terminals/{terminal_id}/conversation/prompt"),
                &crate::agent_conversation::AgentPrompt {
                    text: text.to_string(),
                },
            )
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
            self.attach_shell(&websocket_url(
                &self.base_url,
                &format!("/api/session/{session_id}/terminals/{terminal_id}/socket"),
            ))
        }

        fn start_application(
            &self,
            session_id: &str,
            request: &crate::api::display::StartApplicationRequest,
        ) -> Result<crate::api::display::DisplayView> {
            self.post_json(
                &format!("/api/session/{session_id}/display/applications"),
                request,
            )
        }

        fn installed_applications(&self) -> Result<crate::api::applications::InstalledApplications> {
            self.get("/api/applications")
        }

        fn display(&self) -> Result<Option<crate::api::display::DisplayView>> {
            self.get("/api/display")
        }

        fn end_display(&self) -> Result<()> {
            self.delete("/api/display")
        }

        fn attach_display(&self) -> Result<crate::backend::Socket> {
            self.open_socket(&websocket_url(&self.base_url, "/api/display/socket"))
        }
    };
}

pub(super) use methods;
