//! Moontasks: the sprint board moonreview runs agents from.
//!
//! [`store`] is the `.moontasks` folder on disk and [`service`] is everything both frontends
//! do to it.

pub(crate) mod autopilot;
pub(crate) mod mcp;
pub(crate) mod service;
pub(crate) mod store;
pub(crate) mod worktrees;

use serde::{Deserialize, Serialize};

use crate::api::AgentKind;
pub(crate) use store::{BoardColumn, ColumnEnd, ColumnId, TaskResourceKind};

/// The tag that says a card may be worked without being handed over one at a time.
pub(crate) const AUTOPILOT_TAG: &str = "autopilot";

/// One task, as the board draws it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: ColumnId,
    pub(crate) created_at_unix: u64,
    /// The task folder itself, so the board can offer it to a shell or a file browser.
    pub(crate) dir_path: String,
    /// Where the task's agents and shells work: its own checkout once it has one, and the repo
    /// the board belongs to until then.
    pub(crate) repo_path: String,
    /// The checkout of its own this task was given, for the card to draw and the review step
    /// to work from.
    #[serde(default)]
    pub(crate) worktree: Option<TaskWorktreeView>,
    /// What the card is marked with. Tags are the person's; see [`AUTOPILOT_TAG`].
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    /// The whole of the task's `notes.md`, empty while nothing has been written in it. The
    /// card draws its first lines as the task's description, and typing there writes it back.
    pub(crate) notes: String,
    pub(crate) resources: Vec<TaskResourceView>,
}

/// A task's own checkout, as the board draws it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskWorktreeView {
    pub(crate) path: String,
    pub(crate) branch: String,
    /// Whether the checkout has nothing uncommitted in it. A card cannot be reviewed until it
    /// does, because reviewing puts the branch in the repo and only committed work is on it.
    pub(crate) is_clean: bool,
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
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StartResourceRequest {
    pub(crate) kind: TaskResourceKind,
    /// Which agent to run, for an agent resource.
    pub(crate) agent: AgentKind,
    /// The whole of the work, for a run that is to be given it up front and left to finish on
    /// its own — see [`AgentLaunch::one_shot`].
    #[serde(default)]
    pub(crate) prompt: Option<String>,
}

/// A session an agent already has, being put on a task as a new resource.
///
/// This is the way back when a task's recorded session id stopped pointing anywhere — the
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
    /// The agent to start on the task straight away. `None` leaves the task sitting in its
    /// column with nothing running.
    pub(crate) agent: AgentKind,
    /// The column the new card joins — the one whose `+` opened the composer.
    pub(crate) status: ColumnId,
    /// Which end of that column it joins, which is the `+` that was pressed: the one on the
    /// heading puts the card on top, the one under the last card puts it at the bottom.
    pub(crate) joins: ColumnEnd,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTitleRequest {
    pub(crate) title: String,
}

/// What a card is marked with, set whole rather than one tag at a time: the tag menu knows the
/// list it wants when it closes, and sending that is one write instead of a diff.
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTagsRequest {
    pub(crate) tags: Vec<String>,
}

/// The answer to reviewing a task: where the review opens, and against what.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskReviewPayload {
    pub(crate) repo_path: String,
    pub(crate) title: String,
    /// The base branch for a task reviewed from its own branch.
    pub(crate) base_branch: Option<String>,
}

/// The answer to opening a task's notes: where the file pane finds the file, relative to the
/// repo root, which is how every file pane path is addressed.
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskNotesPayload {
    pub(crate) file_path: String,
}

/// Where a dragged card was let go of: the column, and how many of that column's other cards
/// are above it.
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskPlacementRequest {
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
/// | `{prompt}` | the work, for a run that was given it up front |
///
/// Starting an agent the ordinary way does not hand it the work. It is opening a conversation,
/// not firing a job off: it comes up knowing which task it is on, and waits to be told what to
/// do about it. `brief.md` in the task folder is the same text, for an agent with no system
/// prompt to be given it in.
///
/// The card's title is typed into its box a moment after it starts — see
/// [`crate::terminal::TerminalSpec::type_ahead`] — so the conversation opens with something
/// written and nothing sent. That is a keystroke short of firing the job off, and the
/// keystroke is the person's.
///
/// [`AgentLaunch::one_shot`] is the other way, and the difference is only where the work comes
/// from: it is on the command line rather than typed, so nothing is ever typed into that run
/// at all. The keystroke was still the person's — it happened further up, when they asked for
/// the run.
///
/// An argument whose placeholder has nothing to fill it takes the flag in front of it with it,
/// so an agent that cannot be told its session id is simply run without one.
pub(crate) struct AgentLaunch {
    pub(crate) kind: AgentKind,
    /// Args for a fresh run.
    pub(crate) start: &'static [&'static str],
    /// Args for a run given the whole of its work up front, which finishes on its own.
    pub(crate) one_shot: &'static [&'static str],
    /// Args that resume a run whose session id was never recorded, by whatever the agent
    /// itself reckons the run was. No brief and no prompt: the session being resumed
    /// already has both.
    pub(crate) resume: &'static [&'static str],
    /// Args that open the exact session `{session}` names. Used whenever the id is known —
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
        // Claude accepts a chosen session id for one-shot runs.
        one_shot: &[
            "-p",
            "--session-id",
            "{session}",
            "--append-system-prompt",
            "{brief}",
            "--permission-mode",
            "bypassPermissions",
            "{prompt}",
        ],
        resume: &[],
        attach: &["--resume", "{session}"],
    },
    AgentLaunch {
        kind: AgentKind::Codex,
        start: &[],
        one_shot: &["exec", "--full-auto", "{prompt}"],
        resume: &["resume", "--last"],
        attach: &["resume", "{session}"],
    },
    AgentLaunch {
        kind: AgentKind::OpenCode,
        start: &[],
        one_shot: &["run", "--auto", "{prompt}"],
        resume: &["--continue"],
        attach: &["--session", "{session}"],
    },
];

/// Every placeholder [`AGENT_LAUNCHES`] may use.
///
/// A run fills in the ones it has a value for; an argument naming one it does not is dropped.
/// Which placeholders exist has to be written down, because a filled-in value can contain
/// braces of its own — the brief is free text — and so cannot be told apart from an unfilled
/// placeholder by looking at the result.
pub(crate) const LAUNCH_PLACEHOLDERS: &[&str] = &["{session}", "{brief}", "{prompt}"];

/// What an agent working in a task is told, beyond the work itself.
///
/// It names the task, says where to put anything that belongs to it, and says how the work
/// being finished is reported — which is by saying so, since the person reading this shell is
/// the one who moves the card.
pub(crate) fn brief_for(title: &str, task_dir: &str, style: RunStyle) -> String {
    format!(
        "You are working on a task from moonreview's moontasks board.\n\
         \n\
         Task: {title}\n\
         Task folder: {task_dir}\n\
         \n\
         Say plainly when the work is finished and ready to be looked at, and say just as \
         plainly if you have to stop before it is — you are blocked, or you need a decision. \
         The person who created this task is the one who moves its card, and what you say is \
         how they know to.\n\
         \n\
         Notes, plans and scratch files that belong to this task go in the task folder rather \
         than in the repo. Start with notes.md there: it is the task's description and shared \
         notes, shown on the board's card, read and written by you and the person running the \
         task alike.\n\
         \n\
         {}",
        closing_for(style)
    )
}

/// How a run is given its work, which is the one thing the brief's last paragraph turns on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RunStyle {
    /// Someone opened a session on the card and will type into it.
    Conversation,
    /// The work went in on the command line and nobody will ever type into this run.
    OneShot,
}

/// The last paragraph of the brief, per run style.
///
/// The rest of the brief is true of any run. This paragraph is the one place the two differ,
/// and they differ completely: telling a headless run to wait for a person is telling it to do
/// nothing, and telling a conversation to commit and stop is telling it to run off with a card
/// someone opened to talk about.
const RUN_STYLE_CLOSINGS: &[(RunStyle, &str)] = &[
    (
        RunStyle::Conversation,
        "The title above is the name on a card, not the brief. Wait for the person who opened \
         this session to explain what they actually want before starting on anything.",
    ),
    (
        RunStyle::OneShot,
        "Nobody is reading this run as it goes and there is no one to ask, so do not stop to \
         ask: the work you were given is the whole of the brief, and where it leaves something \
         open, make the call and write down in notes.md which call you made. Commit what you \
         finish before you stop — an uncommitted change is one nobody can review, and this run \
         is judged by what is on the branch when it ends. If you truly cannot get there, commit \
         what stands up on its own, write what stopped you in notes.md, and end the run rather \
         than waiting.",
    ),
];

fn closing_for(style: RunStyle) -> &'static str {
    RUN_STYLE_CLOSINGS
        .iter()
        .find(|(candidate, _)| *candidate == style)
        .map(|(_, closing)| *closing)
        .expect("every run style has a closing")
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
/// there — an agent, a shell the user opens, anything either of them starts — knows which
/// task it is in and which server owns it.
pub(crate) const TASK_ID_ENV_VAR: &str = "MOONREVIEW_TASK_ID";
pub(crate) const TASK_DIR_ENV_VAR: &str = "MOONREVIEW_TASK_DIR";
pub(crate) const SESSION_ID_ENV_VAR: &str = "MOONREVIEW_SESSION_ID";
pub(crate) const SERVER_URL_ENV_VAR: &str = "MOONREVIEW_SERVER_URL";
