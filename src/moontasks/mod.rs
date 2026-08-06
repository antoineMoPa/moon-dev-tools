//! Moontasks: the sprint board moonreview runs agents from.
//!
//! [`store`] is the `.moontasks` folder on disk, [`service`] is everything both frontends do
//! to it, and [`mcp`] is the small MCP server an agent working in a task talks to when it
//! wants to move itself to another column.

pub(crate) mod mcp;
pub(crate) mod service;
pub(crate) mod store;

use serde::{Deserialize, Serialize};

use crate::api::AgentKind;
pub(crate) use store::{TaskResourceKind, TaskStatus};

/// One task, as the board draws it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: TaskStatus,
    pub(crate) created_at_unix: u64,
    /// The task folder itself, so the board can offer it to a shell or a file browser.
    pub(crate) dir_path: String,
    /// The repo the task's agents work in, which is the repo the board belongs to.
    pub(crate) repo_path: String,
    pub(crate) resources: Vec<TaskResourceView>,
}

/// A shell or an agent run belonging to a task.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskResourceView {
    pub(crate) id: String,
    pub(crate) kind: TaskResourceKind,
    pub(crate) agent: AgentKind,
    pub(crate) label: String,
    /// The shell it is attached to, while it is still running.
    pub(crate) terminal_id: Option<String>,
    pub(crate) running: bool,
    /// Whether the run can be started again where it left off, which needs the agent to have
    /// been told its session id when it started.
    pub(crate) resumable: bool,
    pub(crate) started_at_unix: u64,
}

/// What starting a task's resource asked for.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StartResourceRequest {
    pub(crate) kind: TaskResourceKind,
    /// Which agent to run, for an agent resource.
    pub(crate) agent: AgentKind,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CreateTaskRequest {
    pub(crate) title: String,
    /// The agent to start on the task straight away. `None` leaves the task in TODO with
    /// nothing running.
    pub(crate) agent: AgentKind,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTitleRequest {
    pub(crate) title: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TaskStatusRequest {
    pub(crate) status: TaskStatus,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalOpened {
    pub(crate) terminal_id: String,
}

/// How each agent is run for a task: told which task it is on, given the work, and pointed at
/// the MCP server that lets it move its own card.
///
/// Every string here is filled in before it is passed on. The placeholders are:
///
/// | | |
/// | --- | --- |
/// | `{exe}` | moonreview itself, as a TOML string, for a config value that names it |
/// | `{mcp_json}` | the task's MCP config in Claude's shape |
/// | `{mcp_opencode}` | the same in OpenCode's shape |
/// | `{mcp_env_toml}` | the task's environment as a TOML inline table, for Codex's `-c` |
/// | `{session}` | the session id moontasks generated for this run |
/// | `{brief}` | the standing instructions: which task, and to report back when done |
///
/// No agent is handed the work as a prompt. Starting one is opening a conversation, not
/// firing a job off: it comes up knowing which task it is on, and waits to be told what to do
/// about it. `brief.md` in the task folder is the same text, for an agent with no system
/// prompt to be given it in.
///
/// An argument whose placeholder has nothing to fill it takes the flag in front of it with it,
/// so an agent that cannot be told its session id is simply run without one.
pub(crate) struct AgentLaunch {
    pub(crate) kind: AgentKind,
    /// Args that wire the task's MCP server in.
    pub(crate) mcp: &'static [&'static str],
    /// The same, for an agent that reads its MCP config from a file the environment names.
    pub(crate) env: &'static [(&'static str, &'static str)],
    /// Args for a fresh run.
    pub(crate) start: &'static [&'static str],
    /// Args that resume the run recorded on the task. No brief and no prompt: the session
    /// being resumed already has both.
    pub(crate) resume: &'static [&'static str],
}

pub(crate) const AGENT_LAUNCHES: &[AgentLaunch] = &[
    AgentLaunch {
        kind: AgentKind::Claude,
        mcp: &["--mcp-config", "{mcp_json}"],
        env: &[],
        // The brief and no prompt: it knows the task and the tools from the moment it starts,
        // and waits at its prompt for the person who created the task to explain the work.
        start: &[
            "--session-id",
            "{session}",
            "--append-system-prompt",
            "{brief}",
        ],
        resume: &["--resume", "{session}"],
    },
    AgentLaunch {
        kind: AgentKind::Codex,
        // Codex has no per-run MCP config file, but `-c` overrides one key of its config, and
        // an MCP server is three keys.
        mcp: &[
            "-c",
            "mcp_servers.moontasks.command={exe}",
            "-c",
            "mcp_servers.moontasks.args=[\"mcp\"]",
            "-c",
            "mcp_servers.moontasks.env={mcp_env_toml}",
        ],
        env: &[],
        start: &[],
        resume: &["resume", "--last"],
    },
    AgentLaunch {
        kind: AgentKind::OpenCode,
        mcp: &[],
        // OpenCode takes its whole config from the file this names, so the task's copy is
        // what it reads rather than the one in the repo.
        env: &[("OPENCODE_CONFIG", "{mcp_opencode}")],
        start: &[],
        resume: &["--continue"],
    },
];

/// Every placeholder [`AGENT_LAUNCHES`] may use.
///
/// A run fills in the ones it has a value for; an argument naming one it does not is dropped.
/// Which placeholders exist has to be written down, because a filled-in value can contain
/// braces of its own — Codex's MCP override is a TOML table — and so cannot be told apart from
/// an unfilled placeholder by looking at the result.
pub(crate) const LAUNCH_PLACEHOLDERS: &[&str] = &[
    "{exe}",
    "{mcp_json}",
    "{mcp_opencode}",
    "{mcp_env_toml}",
    "{session}",
    "{brief}",
];

/// What an agent working in a task is told, beyond the work itself.
///
/// It names the task, says where to put anything that belongs to it, and — the point of all
/// of this — says which tool to call when the work is ready to be looked at. Without it an
/// agent has the tools and no reason to reach for them.
pub(crate) fn brief_for(title: &str, task_dir: &str) -> String {
    format!(
        "You are working on a task from moonreview's moontasks board.\n\
         \n\
         Task: {title}\n\
         Task folder: {task_dir}\n\
         \n\
         An MCP server called \"moontasks\" is attached to this session. It has two tools:\n\
         \n\
         - moontasks_set_status — move this task to another column of the board\n\
         - moontasks_get_task — read the task you are on\n\
         \n\
         Call moontasks_set_status with in_local_review as soon as the work is finished and \
         ready to be reviewed. That is how the person who created this task finds out. If you \
         have to stop before it is finished — you are blocked, or you need a decision — call \
         it with todo instead, and say plainly what you need.\n\
         \n\
         Notes, plans and scratch files that belong to this task go in the task folder rather \
         than in the repo.\n\
         \n\
         The title above is the name on a card, not the brief. Wait for the person who opened \
         this session to explain what they actually want before starting on anything."
    )
}

/// The file the brief is also written to, so it can be read by a person or by an agent that
/// had no way to be handed it.
pub(crate) const BRIEF_FILE_NAME: &str = "brief.md";
/// The task's MCP config in OpenCode's shape, beside the one in Claude's.
pub(crate) const OPENCODE_CONFIG_FILE_NAME: &str = "opencode.json";

pub(crate) fn agent_launch(agent: AgentKind) -> Option<&'static AgentLaunch> {
    AGENT_LAUNCHES.iter().find(|launch| launch.kind == agent)
}

/// The environment every process moontasks starts for a task is given, so anything running
/// there — an agent, the MCP server it spawns, a shell the user opens — knows which task it
/// is in and which server owns it.
pub(crate) const TASK_ID_ENV_VAR: &str = "MOONREVIEW_TASK_ID";
pub(crate) const TASK_DIR_ENV_VAR: &str = "MOONREVIEW_TASK_DIR";
pub(crate) const SESSION_ID_ENV_VAR: &str = "MOONREVIEW_SESSION_ID";
pub(crate) const SERVER_URL_ENV_VAR: &str = "MOONREVIEW_SERVER_URL";
