//! Moontasks: the sprint board moonreview runs agents from.
//!
//! [`store`] is the `.moontasks` folder on disk and [`service`] is everything the board
//! do to it.

pub(crate) mod column_sort;
pub(crate) mod review_request;
// The board's folder is read and written by the server - see `store`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod service;
pub(crate) mod store;
// How the server starts a task's agents and what it tells them, which a window in a browser
// only ever asks it to do.
#[cfg(not(target_arch = "wasm32"))]
mod task_processes;
// What the agents of a board tell each other, from their shells: posted by a command, and
// kept and delivered by the server and its window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod wire;

use serde::{Deserialize, Serialize};

use crate::{api::AgentKind, commit_suggestion::CommitSuggestion};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use review_request::REVIEW_REQUEST_BRIEF_FILE_NAME;
pub(crate) use store::{BoardColumn, ColumnEnd, ColumnId, ColumnSort, TaskResourceKind};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use task_processes::*;

/// One task, as the board draws it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskView {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: ColumnId,
    pub(crate) created_at_unix: u64,
    /// When the card arrived in its column - see [`store::TaskMetadata::entered_column_at_unix`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) entered_column_at_unix: Option<u64>,
    /// The task folder itself, so the board can offer it to a shell or a file browser.
    pub(crate) dir_path: String,
    /// The repo the task's agents work in, which is the repo the board belongs to.
    pub(crate) repo_path: String,
    /// What the card is marked with, in the spelling the store keeps - see
    /// [`store::tag_of`].
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    /// The whole of the task's `notes.md`, empty while nothing has been written in it. The
    /// card draws its first lines as the task's description, and typing there writes it back.
    pub(crate) notes: String,
    /// The documents the task's `file_attachments.txt` lists, one per line, as written there:
    /// a path from the task's folder, or an absolute one. The task's pane lists them so a
    /// document is found from the task.
    #[serde(default)]
    pub(crate) attachments: Vec<String>,
    pub(crate) resources: Vec<TaskResourceView>,
}

/// The board task, as the board draws it above its columns: the one task that is on no
/// column, and so has no card - see `create_board_task` in the store. It is what the agents
/// and shells started from the board itself are runs of.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BoardTaskView {
    pub(crate) id: String,
    pub(crate) title: String,
    /// The repo the board task's agents work in, which is the repo the board belongs to.
    pub(crate) repo_path: String,
    pub(crate) resources: Vec<TaskResourceView>,
}

/// What a list of runs and a `[start]` menu are drawn for: a task's card, or the board task.
/// The two are started in, listed and stopped the same way, so they are drawn by the same
/// code, from this.
#[derive(Clone, Copy)]
pub(crate) struct RunsOf<'holder> {
    /// The id of the task the runs are recorded on.
    pub(crate) id: &'holder str,
    pub(crate) title: &'holder str,
    pub(crate) repo_path: &'holder str,
    pub(crate) resources: &'holder [TaskResourceView],
}

impl TaskView {
    pub(crate) fn runs(&self) -> RunsOf<'_> {
        RunsOf {
            id: &self.id,
            title: &self.title,
            repo_path: &self.repo_path,
            resources: &self.resources,
        }
    }
}

impl BoardTaskView {
    pub(crate) fn runs(&self) -> RunsOf<'_> {
        RunsOf {
            id: &self.id,
            title: &self.title,
            repo_path: &self.repo_path,
            resources: &self.resources,
        }
    }
}

/// A shell, an agent run or a linked file belonging to a task.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskResourceView {
    pub(crate) id: String,
    pub(crate) kind: TaskResourceKind,
    pub(crate) agent: AgentKind,
    pub(crate) label: String,
    /// The file a linked file opens, relative to the repo root. `Some` for a file, and for a
    /// visualization: its copy in the task's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file_path: Option<String>,
    /// The process a run is going in when that is not a shell of this moon: another moon's -
    /// the window beside a `moon serve` - or the agent's own, started outside any moon and put
    /// on the task afterwards. It is going, and it is not this moon's to open, stop or
    /// resume. `None` for a run that is going here, or not going.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) going_elsewhere_in: Option<u32>,
    /// The shell it is attached to, while it is still running.
    pub(crate) terminal_id: Option<String>,
    pub(crate) running: bool,
    /// How long the shell behind a running agent run has printed nothing, in whole seconds.
    /// An agent draws a spinner while it works and stops once it is waiting - on the person,
    /// or on a question it asked - so a run quiet for a while is one to look at, and the card
    /// marks it. Nothing for a shell, a file, a run that has ended, or one that has not
    /// printed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quiet_for_secs: Option<u64>,
    /// What the shell behind it is asking a person for, if anything: a bell, or the
    /// notification an agent sends when it is waiting on a question or a permission - see
    /// [`crate::attention`]. The card marks it, and typing into the shell takes it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) attention: Option<crate::api::TerminalAttentionView>,
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
    /// When the task's file was last written, in seconds since the epoch. Several tasks can
    /// name the same repo, and the one that wrote its file last is the one whose work is in
    /// the repo now - the older lines are for work already committed, whose messages must not
    /// land in a box that is for something else.
    #[serde(default)]
    pub(crate) written_at_unix: u64,
}

/// What starting a task's resource asked for.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StartResourceRequest {
    pub(crate) kind: TaskResourceKind,
    /// Which agent to run, for an agent resource.
    pub(crate) agent: AgentKind,
    /// The folder it comes up in.
    pub(crate) opens_in: StartFolder,
    /// Whether nobody is at the window for the agent as it comes up, which is what
    /// `moon agent start` says. Two things a person would have done are then done or left out
    /// for them. The card's title is not written in the agent's box for them to send: the
    /// line the agent is told next would be typed on the end of it. And an agent asking
    /// whether it trusts a folder it has not been run in is told that it does - see `trust`
    /// in [`crate::terminal`] - since starting it there is what said so, and the Enter after
    /// that line would otherwise be the answer.
    #[serde(default)]
    pub(crate) unattended: bool,
}

/// Which folder a task's shell or agent comes up in.
///
/// An agent is always started in the repo: the work is there, and the task folder is what the
/// brief points it at. A shell is offered both, because a task folder is a place you read -
/// the notes, the brief, whatever the agent left in it.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartFolder {
    /// The repo the board is over.
    Repo,
    /// The task's own folder under `.moontasks`.
    TaskFolder,
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

// Read by the server's routes only: a remote window writes this and the next as `json!`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTitleRequest {
    pub(crate) title: String,
}

/// What a card is marked with, set whole rather than one tag at a time: the tag menu knows the
/// list it wants when it closes, and sending that is one write instead of a diff.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct TaskTagsRequest {
    pub(crate) tags: Vec<String>,
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

/// Where the work log is, as the file pane addresses it - the answer to opening it, which
/// makes the file when the project has none yet.
#[derive(Serialize, Deserialize)]
pub(crate) struct WorkLogPayload {
    pub(crate) file_path: String,
}

/// Where the wire's broadcasts are kept, as the file pane addresses the file - the answer to
/// opening it, which makes the file on a board nobody has posted to yet.
#[derive(Serialize, Deserialize)]
pub(crate) struct WirePayload {
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

/// A column being renamed: what it is to be called.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnLabelRequest {
    pub(crate) label: String,
}

/// A column being added: what it is called, and where among the others it goes.
#[derive(Serialize, Deserialize)]
pub(crate) struct NewColumnRequest {
    pub(crate) label: String,
    /// How many columns are to its left. Nothing puts it at the right-hand end, which is
    /// where the board's own `+` adds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) at: Option<usize>,
}

/// Which end of a column cards moved into it go to, or nothing for wherever they were dropped.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnArrivalsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arrivals: Option<ColumnEnd>,
}

/// The order a column keeps its cards in by itself, or nothing for the order they are dragged
/// into.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnSortRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sort: Option<ColumnSort>,
}

/// Whether a column draws a line under a day's work of its cards.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnDaysWorkRequest {
    pub(crate) marks_a_days_work: bool,
}

/// Where a dragged column was let go of: how many of the other columns are to its left.
#[derive(Serialize, Deserialize)]
pub(crate) struct ColumnPlacementRequest {
    pub(crate) position: usize,
}
