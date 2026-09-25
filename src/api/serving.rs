//! Where the server listens, and what its routes are asked with: the halves of the API only the
//! server reads. Never compiled for the browser, whose window writes its requests out itself -
//! see `crate::backend::remote`.

use std::{env, sync::OnceLock};

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

/// The port the server is asked to listen on: `MOONREVIEW_PORT`, or [`DEFAULT_PORT`]. Where
/// it ends up listening may be a port or two up - see [`record_bound_port`].
pub(crate) fn port() -> Result<u16> {
    match env::var(PORT_ENV_VAR) {
        Ok(raw) => raw
            .parse::<u16>()
            .with_context(|| format!("{PORT_ENV_VAR} must be a valid TCP port")),
        Err(env::VarError::NotPresent) => Ok(DEFAULT_PORT),
        Err(error) => Err(error).with_context(|| format!("failed to read {PORT_ENV_VAR}")),
    }
}

/// The port this process's server is listening on, once it is: the asked port when that was
/// free, and the next free one after it when another server - a window, or a `moon serve` -
/// already had it. Every address handed out afterwards is on this port.
static BOUND_PORT: OnceLock<u16> = OnceLock::new();

/// Say which port the server bound - once, as a process serves once.
pub(crate) fn record_bound_port(port: u16) {
    BOUND_PORT
        .set(port)
        .expect("a process binds its server once");
}

/// The port addresses are written with: the one bound when the server is up, the one asked for
/// before that.
fn port_or_default() -> u16 {
    match BOUND_PORT.get() {
        Some(bound) => *bound,
        None => port().unwrap_or(DEFAULT_PORT),
    }
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
