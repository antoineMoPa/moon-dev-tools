//! Where the server listens, and what its routes are asked with: the halves of the API only the
//! server reads. Never compiled for the browser, whose window writes its requests out itself -
//! see `crate::backend::remote`.

use std::env;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{AgentKind, BlameOf, DEFAULT_PORT};

pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";
const HOST_ENV_VAR: &str = "MOONREVIEW_HOST";
const PORT_ENV_VAR: &str = "MOONREVIEW_PORT";

pub(crate) fn bind_host() -> String {
    env::var(HOST_ENV_VAR).unwrap_or_else(|_| DEFAULT_HOST.to_string())
}

pub(crate) fn client_host() -> String {
    match bind_host().as_str() {
        "0.0.0.0" | "::" => DEFAULT_HOST.to_string(),
        host => host.to_string(),
    }
}

pub(crate) fn port() -> Result<u16> {
    match env::var(PORT_ENV_VAR) {
        Ok(raw) => raw
            .parse::<u16>()
            .with_context(|| format!("{PORT_ENV_VAR} must be a valid TCP port")),
        Err(env::VarError::NotPresent) => Ok(DEFAULT_PORT),
        Err(error) => Err(error).with_context(|| format!("failed to read {PORT_ENV_VAR}")),
    }
}

fn port_or_default() -> u16 {
    port().unwrap_or(DEFAULT_PORT)
}

pub(crate) fn server_url() -> String {
    format!("http://{}:{}", client_host(), port_or_default())
}

pub(crate) fn export_server_url() -> String {
    format!("http://localhost:{}", port_or_default())
}

/// A blame asked about a file, and of which version of it.
#[derive(Serialize, Deserialize)]
pub(crate) struct BlameRequest {
    pub(crate) file_path: String,
    pub(crate) of: BlameOf,
}

/// A file as one commit has it, asked for by path and revision.
#[derive(Deserialize)]
pub(crate) struct FileAtQuery {
    pub(crate) file_path: String,
    pub(crate) revision: String,
}

#[derive(Deserialize)]
pub(crate) struct HunkRequest {
    pub(crate) hunk_id: String,
}

#[derive(Deserialize)]
pub(crate) struct HunkBatchRequest {
    pub(crate) hunk_ids: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileRequest {
    pub(crate) file_path: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WriteFileRequest {
    pub(crate) file_path: String,
    pub(crate) content: String,
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    pub(crate) file_path: String,
}

#[derive(Deserialize)]
pub(crate) struct FileSearchQuery {
    pub(crate) query: String,
    /// Whether the files the repo's `.gitignore` leaves out are searched too - see
    /// [`SearchScope`], which this is on the wire.
    pub(crate) include_ignored: bool,
}

#[derive(Deserialize)]
pub(crate) struct CommitHistoryQuery {
    pub(crate) offset: Option<usize>,
    pub(crate) limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct CancelCommentDispatchRequest {
    pub(crate) hunk_id: String,
    pub(crate) comment_index: usize,
}

#[derive(Deserialize)]
pub(crate) struct AgentLogQuery {
    pub(crate) dispatch_key: String,
}

#[derive(Deserialize)]
pub(crate) struct AgentSelectionRequest {
    pub(crate) agent: AgentKind,
}

#[derive(Deserialize)]
pub(crate) struct CommitSelectionRequest {
    pub(crate) commit: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SelectionRequest {
    pub(crate) hunk_id: String,
    pub(crate) selection: String,
}
