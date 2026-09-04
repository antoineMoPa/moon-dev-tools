//! Moontasks: the sprint board moonreview runs agents from.
//!
//! [`store`] is the `.moontasks` folder on disk and [`service`] is everything the board
//! do to it.

pub(crate) mod review_request;
pub(crate) mod service;
pub(crate) mod store;

use serde::{Deserialize, Serialize};

use crate::{api::AgentKind, commit_suggestion::CommitSuggestion};
pub(crate) use review_request::REVIEW_REQUEST_BRIEF_FILE_NAME;
pub(crate) use store::{BoardColumn, ColumnEnd, ColumnId, TaskResourceKind};

/// One task, as the board draws it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: ColumnId,
    pub(crate) created_at_unix: u64,
    /// The task folder itself, so the board can offer it to a shell or a file browser.
    pub(crate) dir_path: String,
    /// The repo the task's agents work in, which is the repo the board belongs to.
    pub(crate) repo_path: String,
    /// The whole of the task's `notes.md`, empty while nothing has been written in it. The
    /// card draws its first lines as the task's description, and typing there writes it back.
    pub(crate) notes: String,
    pub(crate) resources: Vec<TaskResourceView>,
}

/// A shell, an agent run or a linked file belonging to a task.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskResourceView {
    pub(crate) id: String,
    pub(crate) kind: TaskResourceKind,
    pub(crate) agent: AgentKind,
    pub(crate) label: String,
    /// The file a linked file opens, relative to the repo root. `Some` for a file and nothing
    /// else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file_path: Option<String>,
    /// The shell it is attached to, while it is still running.
    pub(crate) terminal_id: Option<String>,
    pub(crate) running: bool,
    /// Whether the run can be started again where it left off, which needs the agent to have
    /// been told its session id when it started.
    pub(crate) resumable: bool,
    pub(crate) started_at_unix: u64,
}

/// One repo a task's `request_for_review.txt` asks to have looked at, as the board draws it.
///
/// These are read off the file rather than out of `metadata.json`: the list is the agent's, and
/// what it says is what the row says. They arrive in deploy order - task by task, and within a
/// task in the order the lines are written.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ReviewRequestView {
    pub(crate) task_id: String,
    /// Where the line sits in its task's file, counting entries from the top. What a dismiss
    /// names, so it takes out the line it was asked about and no other like it.
    pub(crate) index: usize,
    /// The repo as the line named it - `repos/turbocharger`. Empty for the board's own repo.
    pub(crate) path_under_repo: String,
    /// That path against the board's repo, which is the review the row opens. The same string
    /// the submodule hub carries for the same repo, so the two lists agree about it.
    pub(crate) repo_path: String,
    /// The repo's directory name, which is what the row says.
    pub(crate) name: String,
    /// The branch the line asked the commit to be made on, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) branch: Option<String>,
    /// The commit the agent wrote for that repo, which its commit pane offers instead of
    /// starting an agent to write one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) suggestion: Option<CommitSuggestion>,
    /// How many files are changed where this is reviewed, which is what says whether it is still
    /// pending. Counted here rather than taken off the submodule hub: the hub knows repos, and
    /// this may be a worktree beside one - and a row that could not be told about would read as
    /// pending for ever, however long ago it was committed.
    pub(crate) changed_files: usize,
    /// Whether the line has been crossed off by hand. The board can tell that a repo has nothing
    /// left to commit; it cannot tell that work already committed and pushed is finished with, so
    /// that is said from the row's menu and written on the line.
    pub(crate) done: bool,
    /// Whether the task this line is on has been moved to the column that finishes a task - see
    /// [`crate::moontasks::store::CLOSES_REVIEWS_IN`]. Finishing the card finishes its reviews
    /// without crossing a single line off, so this is read off where the card sits rather than
    /// off the file, and a card dragged back out asks again.
    pub(crate) task_finished: bool,
}

/// What starting a task's resource asked for.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StartResourceRequest {
    pub(crate) kind: TaskResourceKind,
    /// Which agent to run, for an agent resource.
    pub(crate) agent: AgentKind,
}

/// A session an agent already has, being put on a task as a new resource.
///
/// This is the way back when a task's recorded session id stopped pointing anywhere - the
/// user switched sessions inside the agent, or the agent never persisted the one it was
/// started on. The id here is one read off the agent's own records, so it is known to exist.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AttachResourceRequest {
    pub(crate) agent: AgentKind,
    pub(crate) agent_session_id: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct CreateTaskRequest {
    pub(crate) title: String,
    /// The column the new card joins - the one whose `+` opened the pane the title was
    /// written on.
    pub(crate) status: ColumnId,
    /// Which end of that column it joins, which is the `+` that was pressed: the one on the
    /// heading puts the card on top, the one under the last card puts it at the bottom.
    pub(crate) joins: ColumnEnd,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTitleRequest {
    pub(crate) title: String,
}

/// A file of the repo being put on a task's card, by the path the file pane opens it with:
/// relative to the repo root.
#[derive(Serialize, Deserialize)]
pub(crate) struct LinkFileRequest {
    pub(crate) file_path: String,
}

/// The answer to opening a task's notes: where the file pane finds the file, relative to the
/// repo root, which is how every file pane path is addressed.
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskNotesPayload {
    pub(crate) file_path: String,
}

/// Where dragged cards were let go of: which cards, the column, and how many of that
/// column's other cards are above them. More than one card is a drag made with a selection.
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskPlacementRequest {
    pub(crate) task_ids: Vec<String>,
    pub(crate) status: ColumnId,
    pub(crate) position: usize,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TerminalOpened {
    pub(crate) terminal_id: String,
}

/// A column being added, or one being renamed: in both cases what it is to be called.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnLabelRequest {
    pub(crate) label: String,
}

/// Which end of a column cards moved into it go to, or nothing for wherever they were dropped.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnArrivalsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arrivals: Option<ColumnEnd>,
}

/// Where a dragged column was let go of: how many of the other columns are to its left.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnPlacementRequest {
    pub(crate) position: usize,
}

/// How each agent is run for a task: told which task it is on, and given the work.
///
/// Every string here is filled in before it is passed on. The placeholders are:
///
/// | | |
/// | --- | --- |
/// | `{session}` | the session id moontasks generated for this run |
/// | `{brief}` | the standing instructions: which task, and where its notes go |
///
/// No agent is handed the work as a prompt. Starting one is opening a conversation, not
/// firing a job off: it comes up knowing which task it is on, and waits to be told what to do
/// about it. `brief.md` in the task folder is the same text, for an agent with no system
/// prompt to be given it in.
///
/// The card's title is typed into its box a moment after it starts - see
/// [`crate::terminal::TerminalSpec::type_ahead`] - so the conversation opens with something
/// written and nothing sent. That is a keystroke short of firing the job off, and the
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
        attach: &[
            "--resume",
            "{session}",
            "--append-system-prompt",
            "{brief}",
        ],
    },
    AgentLaunch {
        kind: AgentKind::Codex,
        start: &[],
        resume: &["resume", "--last"],
        attach: &["resume", "{session}"],
    },
    AgentLaunch {
        kind: AgentKind::OpenCode,
        start: &[],
        resume: &["--continue"],
        attach: &["--session", "{session}"],
    },
];

/// Every placeholder [`AGENT_LAUNCHES`] may use.
///
/// A run fills in the ones it has a value for; an argument naming one it does not is dropped.
/// Which placeholders exist has to be written down, because a filled-in value can contain
/// braces of its own - the brief is free text - and so cannot be told apart from an unfilled
/// placeholder by looking at the result.
pub(crate) const LAUNCH_PLACEHOLDERS: &[&str] = &["{session}", "{brief}"];

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
         To request code deploy/review, check {REVIEW_REQUEST_BRIEF_FILE_NAME}"
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
