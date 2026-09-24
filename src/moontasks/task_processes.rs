//! What the processes the server starts for a task are run with: each agent's command line,
//! the brief it is told, and the files and environment that say which task it is in.

use super::{review_request::REVIEW_REQUEST_BRIEF_FILE_NAME, store};
use crate::api::AgentKind;

/// How each agent is run for a task: told which task it is on, and given the work.
///
/// Every string here is filled in before it is passed on. The placeholders are:
///
/// | | |
/// | --- | --- |
/// | `{session}` | the session id moontasks generated for this run |
/// | `{brief}` | the standing instructions: which task, and where its notes go |
/// | `{brief_file}` | the path of `brief.md`, for an agent given a config that names it |
///
/// No agent is handed the work as a prompt. Starting one is opening a conversation, not
/// firing a job off: it comes up knowing which task it is on, and waits to be told what to do
/// about it. `brief.md` in the task folder is the same text, for a person to read.
///
/// The three take the brief three different ways, which is why [`AgentLaunch`] carries both
/// args and environment: Claude has a system-prompt flag, Codex takes developer instructions
/// as a config override, and OpenCode has neither but reads a config out of the environment
/// that can name files to load as instructions. Typing it at them is not an option - an
/// agent's box does not exist yet when the run begins, and what is typed then is dropped.
///
/// The card's title is typed into its box a moment after it starts - see
/// [`crate::terminal::TerminalSpec::type_ahead`] - so the conversation opens with something
/// written and nothing sent. That is a convenience and nothing rests on it: OpenCode's TUI
/// takes the terminal over a second or so in and drops whatever was typed before then, which
/// is exactly why the brief goes through the environment instead. That is a keystroke short of firing the job off, and the
/// keystroke is the person's.
///
/// An argument whose placeholder has nothing to fill it takes the flag in front of it with it,
/// so an agent that cannot be told its session id is simply run without one.
pub(crate) struct AgentLaunch {
    pub(crate) kind: AgentKind,
    /// Args for a fresh run.
    pub(crate) start: &'static [&'static str],
    /// Args that resume a run whose session id was never recorded, by whatever the agent
    /// itself reckons the run was. No brief and no prompt: the session being resumed
    /// already has both.
    pub(crate) resume: &'static [&'static str],
    /// Args that open the exact session `{session}` names. Used whenever the id is known -
    /// resuming a run that recorded one, and attaching a session picked off the agent's own
    /// records.
    pub(crate) attach: &'static [&'static str],
    /// Environment every run of this agent is given, for one that reads its instructions out
    /// of the environment rather than off its command line. Filled in the same way the args
    /// are, and a variable whose value has nothing to fill it is left unset.
    pub(crate) env: &'static [(&'static str, &'static str)],
}

pub(crate) const AGENT_LAUNCHES: &[AgentLaunch] = &[
    AgentLaunch {
        kind: AgentKind::Claude,
        // The brief and no prompt: it knows the task from the moment it starts, and waits at
        // its prompt for the person who created the task to explain the work.
        start: &[
            "--session-id",
            "{session}",
            "--append-system-prompt",
            "{brief}",
        ],
        // The brief again on both, because a resumed session comes back with the system prompt
        // it was opened on and never hears anything new - so a run resumed today would be
        // working from whatever the brief said the day it started, and one attached from the
        // agent's own records would never have had one at all.
        resume: &["--append-system-prompt", "{brief}"],
        attach: &["--resume", "{session}", "--append-system-prompt", "{brief}"],
        env: &[],
    },
    AgentLaunch {
        kind: AgentKind::Codex,
        // Codex has no system-prompt flag, but developer instructions are a config value and
        // every config value can be overridden on the command line. `-c` is a global option,
        // so it leads - `resume` is a subcommand and everything of the program's own comes
        // before it.
        start: &["-c", "developer_instructions={brief}"],
        resume: &["-c", "developer_instructions={brief}", "resume", "--last"],
        attach: &[
            "-c",
            "developer_instructions={brief}",
            "resume",
            "{session}",
        ],
        env: &[],
    },
    AgentLaunch {
        kind: AgentKind::OpenCode,
        start: &[],
        resume: &["--continue"],
        attach: &["--session", "{session}"],
        // OpenCode has no flag of either kind, and an inline config in the environment is
        // what it does have. It is merged over the user's own config rather than replacing
        // it, and `instructions` names files to load as instructions - which is what
        // `brief.md` is written for.
        env: &[(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"$schema":"https://opencode.ai/config.json","instructions":["{brief_file}"]}"#,
        )],
    },
];

/// Every placeholder [`AGENT_LAUNCHES`] may use.
///
/// A run fills in the ones it has a value for; an argument naming one it does not is dropped.
/// Which placeholders exist has to be written down, because a filled-in value can contain
/// braces of its own - the brief is free text - and so cannot be told apart from an unfilled
/// placeholder by looking at the result.
pub(crate) const LAUNCH_PLACEHOLDERS: &[&str] = &["{session}", "{brief}", "{brief_file}"];

/// What an agent working in a task is told, beyond the work itself.
///
/// It names the task and says where anything belonging to it goes; the person who opened the
/// shell says the rest.
pub(crate) fn brief_for(title: &str, task_dir: &str) -> String {
    format!(
        "You are working on a task from moonreview's moontasks board.\n\
         \n\
         Task: {title}\n\
         Task folder: {task_dir}\n\
         \n\
         To request code deploy/review, check {REVIEW_REQUEST_BRIEF_FILE_NAME}\n\
         \n\
         New card on this board: `{program} tasks new \"<title>\"`, which prints its folder.",
        program = crate::cli::PROGRAM
    )
}

/// The file the brief is also written to, so it can be read by a person or by an agent that
/// had no way to be handed it.
pub(crate) const BRIEF_FILE_NAME: &str = "brief.md";

/// The task's description and shared notes, in its folder. The card draws its first lines
/// under the title, and agents are told to write theirs there.
pub(crate) const NOTES_FILE_NAME: &str = "notes.md";

/// The notes file as the file pane addresses it: relative to the repo root.
pub(crate) fn notes_repo_path(task_id: &str) -> String {
    format!("{}/{task_id}/{NOTES_FILE_NAME}", store::TASKS_DIR_NAME)
}

/// The project's work log, in the board's folder beside the tasks - see
/// [`crate::native::work_log`]. One per project: what is going on in this repo is written
/// here, whichever task it is about.
pub(crate) const WORK_LOG_FILE_NAME: &str = "work-log.org";

/// The work log as the file pane addresses it: relative to the repo root.
pub(crate) fn work_log_repo_path() -> String {
    format!("{}/{WORK_LOG_FILE_NAME}", store::TASKS_DIR_NAME)
}

pub(crate) fn agent_launch(agent: AgentKind) -> Option<&'static AgentLaunch> {
    AGENT_LAUNCHES.iter().find(|launch| launch.kind == agent)
}

/// The environment every process moontasks starts for a task is given, so anything running
/// there - an agent, a shell the user opens, anything either of them starts - knows which
/// task it is in and which server owns it.
pub(crate) const TASK_ID_ENV_VAR: &str = "MOONREVIEW_TASK_ID";
pub(crate) const TASK_DIR_ENV_VAR: &str = "MOONREVIEW_TASK_DIR";
pub(crate) const SESSION_ID_ENV_VAR: &str = "MOONREVIEW_SESSION_ID";
pub(crate) const SERVER_URL_ENV_VAR: &str = "MOONREVIEW_SERVER_URL";
