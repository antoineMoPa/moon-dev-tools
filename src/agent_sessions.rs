//! The sessions each agent already has for a repo, read from where that agent keeps them.
//!
//! A task resource remembers the session id its agent was started with, but that id stops
//! meaning anything when the user switches sessions inside the agent or the agent never
//! persisted it. This is the way back: list what the agents actually have on disk, so a
//! task can be attached to one of them.

// Reading the agents' own files is the server's: the window in a browser is only told what
// they hold.
#[cfg(not(target_arch = "wasm32"))]
mod on_disk;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use on_disk::list_for_session;

use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

/// One session an agent has on disk, as the attach modal lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AgentSessionView {
    pub(crate) agent: AgentKind,
    /// The id the agent itself resumes the session by.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) updated_at_unix: u64,
}
