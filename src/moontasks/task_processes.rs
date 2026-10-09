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
        kind: AgentKind::Pi,
        start: &[
            "--session-id",
            "{session}",
            "--append-system-prompt",
            "{brief}",
        ],
        resume: &["--continue", "--append-system-prompt", "{brief}"],
        attach: &[
            "--session",
            "{session}",
            "--append-system-prompt",
            "{brief}",
        ],
        env: &[],
    },
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
         To attach a document to this task (a PDF, a spreadsheet, etc.), check {ATTACHMENTS_BRIEF_FILE_NAME}\n\
         \n\
         Before working, check {COORDINATION_BRIEF_FILE_NAME}\n\
         \n\
         New card on this board: `{program} tasks new \"<title>\"`, which prints its folder.",
        program = crate::cli::PROGRAM
    )
}

/// What an agent started from the board itself is told - see `create_board_task` in the store.
///
/// It has no card to be about, so it is told where the board is and how it is laid out on
/// disk instead: the board is what it is most likely to be asked to work on, and a board is
/// changed by changing those files. The person who opened the shell says the rest.
pub(crate) fn board_task_brief_for(board_dir: &str, home_dir: &str) -> String {
    format!(
        "You were started from moonreview's moontasks board itself, not from one of its cards: \
         for work on the board, or for work no card is about.\n\
         \n\
         Board folder: {board_dir}\n\
         board.json lists the columns, left to right. A card is a folder beside it: its \
         metadata.json holds the title, the column it is in (`status`, a column's `id`) and \
         its place there (`position`, lowest at the top), and its {NOTES_FILE_NAME} is the \
         card's description. The board reads those files again every second or two.\n\
         \n\
         New card on this board: `{program} tasks new \"<title>\"`, which prints its folder.\n\
         \n\
         Your own folder, for whatever you want kept: {home_dir}\n\
         \n\
         To request code deploy/review, check {REVIEW_REQUEST_BRIEF_FILE_NAME} in it\n\
         \n\
         Before working, check {COORDINATION_BRIEF_FILE_NAME} in it",
        program = crate::cli::PROGRAM
    )
}

/// The file the brief is also written to, so it can be read by a person or by an agent that
/// had no way to be handed it.
pub(crate) const BRIEF_FILE_NAME: &str = "brief.md";

/// The task's description and shared notes, in its folder. The card draws its first lines
/// under the title, and agents are told to write theirs there.
pub(crate) const NOTES_FILE_NAME: &str = "notes.md";

/// The file in a task's folder that lists the documents the task is about, one per line.
pub(crate) const ATTACHMENTS_FILE_NAME: &str = "file_attachments.txt";

/// What the format is written down in, in the task's folder beside the file it describes. The
/// brief only points here, as it does for the review request, so the agents that never attach
/// anything are not made to read it.
pub(crate) const ATTACHMENTS_BRIEF_FILE_NAME: &str = "attach_files.md";

/// The whole of the attachments format, for an agent that has a document to attach.
pub(crate) const ATTACHMENTS_BRIEF: &str = "\
# Attaching a document

List the document in `file_attachments.txt` in this task folder, one path per line, for
example a PDF you made or a spreadsheet you were given:

```
report.pdf
/Users/someone/Documents/figures.xlsx
```

A relative path is from this task folder, so a file left in the folder is just its name.
Anything else is an absolute path. Add a line, and leave the other lines alone: the file is
also written by hand.

The task's pane lists the files under `Files`. A click opens a text file - notes, a log, JSON,
source - in a tab of the window, and any other document with the machine's own opener. The
mark at the end of a file's row takes its line out of `file_attachments.txt`, and leaves the
document where it is.
";

/// What an agent is told about the other agents of the board, in the task's folder. The brief
/// only points here, as it does for the review request, so the brief stays a few lines.
pub(crate) const COORDINATION_BRIEF_FILE_NAME: &str = "Coordination.md";

/// The whole of that file. What a line may be, how a handle is read and what is refused are
/// kept out of it: an agent reads this on every task, and a post that goes wrong says why.
pub(crate) fn coordination_brief() -> String {
    format!(
        "Before working, read {wire_file}, then post the areas you will touch: \
         `{post} \"<one line>\"`. Start it with @handle to message one agent instead.\n",
        wire_file = wire_repo_path(),
        post = super::wire::post_command()
    )
}

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

/// The file the wire's broadcasts are kept in, in the board's folder beside the tasks - see
/// [`crate::moontasks::wire`]. Only the file's name says `messageboard`: everywhere else it is
/// the wire.
pub(crate) const WIRE_FILE_NAME: &str = "messageboard.txt";

/// That file as the file pane addresses it, and as an agent started in the repo reads it:
/// relative to the repo root.
pub(crate) fn wire_repo_path() -> String {
    format!("{}/{WIRE_FILE_NAME}", store::TASKS_DIR_NAME)
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
