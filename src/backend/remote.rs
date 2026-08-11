//! Reviews a repo on another machine, over the same HTTP API the web frontend speaks.
//!
//! `moonreview serve` on the far side is the whole server contract, so no extra daemon is
//! involved: this is the web client's transport with a native window in front of it.

use std::{
    net::TcpStream,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    api::{
        AgentKind, AgentLogPayload, CommentRequest, CommitHistoryPayload, FileContentPayload,
        OpenSessionRequest, PatchPayload, SessionOpened, SessionPayload, SubmoduleView,
    },
    backend::Backend,
    moontasks::{
        AttachResourceRequest, BoardColumn, ColumnId, ColumnLabelRequest, ColumnPlacementRequest,
        CreateTaskRequest, StartResourceRequest, TaskNotesPayload, TaskPlacementRequest, TaskView,
        TerminalOpened,
    },
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct RemoteBackend {
    /// Base URL of the remote server, without a trailing slash, e.g. `http://dev-box:42000`.
    base_url: String,
    label: String,
    client: Client,
}

impl RemoteBackend {
    /// `target` is a URL, or a `host` / `host:port` shorthand that means plain HTTP.
    pub(crate) fn connect(target: &str) -> Result<Self> {
        let base_url = base_url_for(target)?;
        let label = label_for(&base_url);
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;

        let health = client
            .get(format!("{base_url}/healthz"))
            .send()
            .with_context(|| format!("failed to reach a moonreview server at {base_url}"))?;
        if !health.status().is_success() {
            bail!("{base_url} answered {} for /healthz", health.status());
        }

        Ok(Self {
            base_url,
            label,
            client,
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("GET {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?
            .json()
            .with_context(|| format!("could not decode the response to GET {path}"))
    }

    fn post_json<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        self.client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .with_context(|| format!("POST {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?
            .json()
            .with_context(|| format!("could not decode the response to POST {path}"))
    }

    fn post(&self, path: &str, body: &impl Serialize) -> Result<()> {
        self.client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .with_context(|| format!("POST {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    fn get_ok(&self, path: &str) -> Result<()> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("GET {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.client
            .delete(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("DELETE {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }
}

/// The server puts the real reason in the body, so a bare status line is not enough.
fn remote_refusal(error: reqwest::Error) -> anyhow::Error {
    anyhow!("{error}")
}

fn base_url_for(target: &str) -> Result<String> {
    let target = target.trim().trim_end_matches('/');
    if target.is_empty() {
        bail!("remote server address must not be empty");
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok(target.to_string());
    }
    if target.contains("://") {
        bail!("remote server address must be http:// or https://, got {target}");
    }
    // A bare `host` or `host:port` is the common case over an SSH tunnel.
    if target.contains(':') {
        Ok(format!("http://{target}"))
    } else {
        Ok(format!("http://{target}:{}", crate::api::DEFAULT_PORT))
    }
}

fn label_for(base_url: &str) -> String {
    base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

fn websocket_url(base_url: &str, path: &str) -> String {
    let socket_base = if let Some(rest) = base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base_url.to_string()
    };
    format!("{socket_base}{path}")
}

/// Both halves of an attached remote shell run on the socket, so writes go through the
/// same lock the reader thread holds between frames.
struct RemoteShell {
    socket: Arc<Mutex<SharedSocket>>,
}

type SharedSocket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

impl egui_tty::Tty for RemoteShell {
    fn write(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "input", "data": text }))
            .map_err(egui_tty::Error::msg)
    }

    /// Sent as its own kind of message, so the far end knows this is the terminal answering
    /// the program rather than a person at the keyboard.
    fn reply(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "reply", "data": text }))
            .map_err(egui_tty::Error::msg)
    }

    fn resize(&self, cols: u16, rows: u16) -> egui_tty::Result<()> {
        self.send(&json!({ "type": "resize", "cols": cols, "rows": rows }))
            .map_err(egui_tty::Error::msg)
    }
}

impl RemoteShell {
    fn send(&self, message: &serde_json::Value) -> Result<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| anyhow!("terminal socket lock poisoned"))?;
        socket
            .send(tungstenite::Message::Text(message.to_string().into()))
            .context("failed to send to the remote shell")?;
        Ok(())
    }
}

impl Backend for RemoteBackend {
    fn describe(&self) -> String {
        self.label.clone()
    }

    fn reads_this_machine(&self) -> bool {
        false
    }

    fn web_url(&self, session_id: &str) -> String {
        format!("{}/review/{session_id}", self.base_url)
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

    fn session_submodules(&self, session_id: &str) -> Result<Vec<SubmoduleView>> {
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

    fn file_content(&self, session_id: &str, file_path: &str) -> Result<FileContentPayload> {
        let encoded = urlencode(file_path);
        self.get(&format!(
            "/api/session/{session_id}/file?file_path={encoded}"
        ))
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
        self.post(&format!("/api/session/{session_id}/comment-batch"), &json!({}))
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

    fn set_reviewed(&self, session_id: &str, hunk_id: &str, reviewed: Option<bool>) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/reviewed"),
            &json!({ "hunk_id": hunk_id, "reviewed": reviewed }),
        )
    }

    fn set_file_reviewed(&self, session_id: &str, file_path: &str, reviewed: bool) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/reviewed-file"),
            &json!({ "file_path": file_path, "reviewed": reviewed }),
        )
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

    fn place_task(
        &self,
        session_id: &str,
        task_id: &str,
        status: ColumnId,
        position: usize,
    ) -> Result<()> {
        self.post(
            &format!("/api/session/{session_id}/tasks/{task_id}/placement"),
            &TaskPlacementRequest { status, position },
        )
    }

    fn delete_task(&self, session_id: &str, task_id: &str) -> Result<()> {
        self.delete(&format!("/api/session/{session_id}/tasks/{task_id}"))
    }

    fn list_columns(&self, session_id: &str) -> Result<Vec<BoardColumn>> {
        self.get(&format!("/api/session/{session_id}/columns"))
    }

    fn add_column(&self, session_id: &str, label: &str) -> Result<BoardColumn> {
        self.post_json(
            &format!("/api/session/{session_id}/columns"),
            &ColumnLabelRequest {
                label: label.to_string(),
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

    fn resume_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<String> {
        let opened: TerminalOpened = self.post_json(
            &format!(
                "/api/session/{session_id}/tasks/{task_id}/resources/{resource_id}/resume"
            ),
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

    fn stop_task_resource(
        &self,
        session_id: &str,
        task_id: &str,
        resource_id: &str,
    ) -> Result<()> {
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

    fn open_task_notes(&self, session_id: &str, task_id: &str) -> Result<String> {
        let notes: TaskNotesPayload = self.post_json(
            &format!("/api/session/{session_id}/tasks/{task_id}/notes/open"),
            &json!({}),
        )?;
        Ok(notes.file_path)
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

    fn close_terminal(&self, session_id: &str, terminal_id: &str) -> Result<()> {
        self.delete(&format!(
            "/api/session/{session_id}/terminals/{terminal_id}"
        ))
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

/// Reads have to give the lock up so keystrokes can go out, so the socket polls instead of
/// blocking forever.
fn set_read_timeout(socket: &SharedSocket) -> Result<()> {
    let timeout = Some(Duration::from_millis(20));
    match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout)?,
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => {
            stream.get_ref().set_read_timeout(timeout)?
        }
        _ => bail!("unsupported terminal socket transport"),
    }
    Ok(())
}

fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_becomes_an_http_url_on_the_default_port() {
        assert_eq!(
            base_url_for("dev-box").expect("expected a URL"),
            format!("http://dev-box:{}", crate::api::DEFAULT_PORT)
        );
    }

    #[test]
    fn host_and_port_shorthand_keeps_the_port() {
        assert_eq!(
            base_url_for("127.0.0.1:9000").expect("expected a URL"),
            "http://127.0.0.1:9000"
        );
    }

    #[test]
    fn explicit_urls_are_kept_and_trailing_slashes_dropped() {
        assert_eq!(
            base_url_for("https://review.example.com/").expect("expected a URL"),
            "https://review.example.com"
        );
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        let error = base_url_for("ssh://dev-box").expect_err("expected ssh:// to be rejected");
        assert!(error.to_string().contains("must be http:// or https://"));
    }

    #[test]
    fn websocket_urls_follow_the_http_scheme() {
        assert_eq!(
            websocket_url("http://dev-box:42000", "/socket"),
            "ws://dev-box:42000/socket"
        );
        assert_eq!(
            websocket_url("https://dev-box", "/socket"),
            "wss://dev-box/socket"
        );
    }

    #[test]
    fn file_paths_with_spaces_and_unicode_are_encoded() {
        assert_eq!(urlencode("src/a b/é.rs"), "src%2Fa%20b%2F%C3%A9.rs");
    }
}
