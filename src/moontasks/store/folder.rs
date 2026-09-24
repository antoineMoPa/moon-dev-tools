//! Reading and writing the `.moontasks` folder.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::*;
use crate::api::AgentKind;

/// The directory, in the repo being reviewed, that holds every task.
pub(crate) const TASKS_DIR_NAME: &str = ".moontasks";

/// The folder, inside `.moontasks`, that a deleted task's folder is moved to. Hidden, so it can
/// never be read as a task id - see [`task_dir`].
pub(crate) const DELETED_TASKS_DIR_NAME: &str = ".deleted";

/// The columns a board starts with, left to right, and which end of each one a card moved in
/// from another column goes to.
pub(crate) const DEFAULT_COLUMNS: &[(&str, &str, Option<ColumnEnd>)] = &[
    ("todo", "TODO", None),
    ("in_progress", "IN PROGRESS", None),
    // What was finished last is what one wants to see, so DONE reads newest first however far
    // down the column a card was dropped.
    ("done", "DONE", Some(ColumnEnd::Top)),
];

/// The board's columns, left to right. This is the whole order: a card naming a column that is
/// not here has nowhere to be drawn.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct BoardConfig {
    pub(crate) columns: Vec<BoardColumn>,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            columns: DEFAULT_COLUMNS
                .iter()
                .map(|(id, label, arrivals)| BoardColumn {
                    id: ColumnId::new(*id),
                    label: (*label).to_string(),
                    arrivals: *arrivals,
                    sort: None,
                })
                .collect(),
        }
    }
}

impl BoardConfig {
    pub(crate) fn position_of(&self, id: &ColumnId) -> Option<usize> {
        self.columns.iter().position(|column| column.id == *id)
    }

    pub(crate) fn has(&self, id: &ColumnId) -> bool {
        self.position_of(id).is_some()
    }

    /// Which end of a column a card arriving from another column goes to, if that column says.
    pub(crate) fn arrivals_end(&self, id: &ColumnId) -> Option<ColumnEnd> {
        self.columns
            .iter()
            .find(|column| column.id == *id)?
            .arrivals
    }

    /// The column one of the board's own rules points at, if it is still on the board.
    pub(crate) fn role(&self, role: &str) -> Option<ColumnId> {
        let id = ColumnId::new(role);
        self.has(&id).then_some(id)
    }
}

/// Something on the task's card: a shell, a run of an agent, a file linked to the task, or a
/// visualization one of its runs showed.
///
/// Whether a shell or a run is going right now is not written down - the shells the server
/// has are what answers that, and they are gone once the server is. A file has nothing
/// running; it is a way back to the file from the card.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskResource {
    pub(crate) id: String,
    pub(crate) kind: TaskResourceKind,
    /// Which agent this is a run of, or which agent showed a visualization. `None` for a shell
    /// or a file.
    #[serde(default)]
    pub(crate) agent: AgentKind,
    /// The file this links to, relative to the repo root - the way every file pane path is
    /// addressed. `Some` for a file, and for a visualization: its copy in the task's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file_path: Option<String>,
    /// The shell it was last attached to. Kept after the shell ends so the board can tell
    /// which past run a resumed one continues.
    #[serde(default)]
    pub(crate) terminal_id: Option<String>,
    /// The process of the moon that holds that shell. The record is in the repo, where every
    /// moon on the machine reads it, and only one of them has the shell: another reading the
    /// board has to tell a shell someone else holds from one whose moon is gone, or it takes a
    /// live run for an ended one - see `reconcile` in `crate::moontasks::service`. `None` on a
    /// run written down before this was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_owner: Option<u32>,
    /// The id the agent was told to give its session, so that run can be resumed exactly
    /// rather than by whatever the agent thinks the most recent one was.
    #[serde(default)]
    pub(crate) agent_session_id: Option<String>,
    /// What the run's shell is called: the name it was given as it started - `claude - 2` -
    /// or was renamed to since. Kept after the shell is gone, so the card still reads it and
    /// a resumed run's shell takes it back. `None` on a run written down before runs had names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    pub(crate) started_at_unix: u64,
}

/// The `metadata.json` of one task folder.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskMetadata {
    pub(crate) title: String,
    /// The column the card is in, by the id the board's file gives it.
    pub(crate) status: ColumnId,
    pub(crate) created_at_unix: u64,
    /// When the card arrived in the column it is in: made there, or moved in from another.
    /// Shuffling a card about inside its column leaves it alone. For a card in DONE this is
    /// when the task was finished, which is what the column's date lines read.
    ///
    /// `None` on a card written before the board kept this, which nothing back-fills: the
    /// file's own timestamp says when it was last touched, not when it was moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) entered_column_at_unix: Option<u64>,
    /// Where the card sits in its column, lowest at the top. Renumbered from zero across the
    /// whole column whenever one is dragged into it, so the numbers stay small and readable
    /// in a file somebody may well edit by hand.
    ///
    /// A board written before cards could be reordered has none of these, which reads as a
    /// column of zeroes - and cards that tie fall back on the order they were created in,
    /// which is the order that board was already drawn in.
    #[serde(default)]
    pub(crate) position: u32,
    /// What the card is marked with, each in the spelling [`super::tag_of`] settles on. A tag is the
    /// person's word for a card - `bug`, `needs-tests`, a client's name - drawn as a pill under
    /// the title and looked through by the board's filter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) resources: Vec<TaskResource>,
}

/// The part of a task id that reads as its title: lower case words joined by dashes.
pub(crate) fn slug_of(title: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in title.chars() {
        if character.is_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.extend(character.to_lowercase());
            // Long titles make unwieldy directory names, and the uuid is what identifies the
            // task anyway.
            if slug.len() >= 40 {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        slug.push_str("task");
    }
    slug
}

const METADATA_FILE_NAME: &str = "metadata.json";
/// The board's own file, beside the task folders: what its columns are and what they are
/// called. A board without one has the columns in [`super::DEFAULT_COLUMNS`], and only grows
/// the file once someone changes them.
const BOARD_FILE_NAME: &str = "board.json";

pub(crate) fn tasks_root(repo_path: &Path) -> PathBuf {
    repo_path.join(TASKS_DIR_NAME)
}

/// What the board's own `.gitignore` says.
///
/// A board is working state - running agents, scratch files, whatever an agent leaves in a
/// task folder - and none of that belongs in someone's `git status` by default. The file
/// ignores the whole folder including itself, so a repo where moonreview has been opened
/// looks exactly like one where it has not.
///
/// Someone who wants the board shared can delete this file and commit the folder, which is
/// why it is written once and never rewritten.
const TASKS_GITIGNORE: &str = "\
# Written by moonreview when it created this board.
#
# A task folder holds running state - shells, agent sessions, scratch files - so by default
# none of it is committed and none of it shows up in `git status`.
#
# Delete this file to share the board with the rest of the team.
*
";

/// Make the `.moontasks` folder if it is not there yet, ignored by git from the start.
fn ensure_tasks_root(repo_path: &Path) -> Result<PathBuf> {
    let root = tasks_root(repo_path);
    // The `.gitignore` goes in when the board is made and never again: it is the user's file
    // from that moment, and a board they chose to share by deleting it must not start
    // ignoring itself again the next time a task is created.
    if root.is_dir() {
        return Ok(root);
    }

    fs::create_dir_all(&root).with_context(|| format!("failed to create {}", root.display()))?;
    let ignore = root.join(".gitignore");
    fs::write(&ignore, TASKS_GITIGNORE)
        .with_context(|| format!("failed to write {}", ignore.display()))?;
    Ok(root)
}

/// The board's columns, as its file has them - or the defaults, for a board that has never
/// had them changed.
///
/// A file that cannot be read or makes no sense is the defaults too, with the same reasoning
/// the task list uses for a broken `metadata.json`: a board that draws is worth more than an
/// error, and nothing here is the only writer.
pub(crate) fn read_board(repo_path: &Path) -> BoardConfig {
    let path = tasks_root(repo_path).join(BOARD_FILE_NAME);
    let Ok(text) = fs::read_to_string(&path) else {
        return BoardConfig::default();
    };
    let Ok(config) = serde_json::from_str::<BoardConfig>(&text) else {
        return BoardConfig::default();
    };
    // A board with no columns has nowhere to put a card, which is worse than not having been
    // customised at all.
    if config.columns.is_empty() {
        return BoardConfig::default();
    }
    config
}

pub(crate) fn write_board(repo_path: &Path, config: &BoardConfig) -> Result<()> {
    let root = ensure_tasks_root(repo_path)?;
    let path = root.join(BOARD_FILE_NAME);
    let text = serde_json::to_string_pretty(config).context("failed to encode the board")?;
    fs::write(&path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn task_dir(repo_path: &Path, task_id: &str) -> Result<PathBuf> {
    // A task id becomes a path, so it may only ever be one directory name.
    if task_id.is_empty()
        || task_id.contains('/')
        || task_id.contains('\\')
        || task_id.starts_with('.')
    {
        bail!("{task_id} is not a task id");
    }
    Ok(tasks_root(repo_path).join(task_id))
}

/// Every task folder in the repo, in the order the board should read them.
pub(crate) fn list_task_ids(repo_path: &Path) -> Result<Vec<String>> {
    let root = tasks_root(repo_path);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut ids = Vec::new();
    for entry in
        fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // A folder with no metadata is not a task, so an unrelated directory dropped in
        // `.moontasks` is left alone rather than shown as a broken card.
        //
        // Neither is anything hidden, which is where the board's own `.gitignore` lives.
        if entry.path().join(METADATA_FILE_NAME).is_file() {
            ids.push(name);
        }
    }
    ids.sort();
    Ok(ids)
}

/// The name of every run written down on the repo's tasks, whichever task and whether or
/// not its shell is still running. This is what a new run's number is counted past.
pub(crate) fn recorded_run_names(repo_path: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for task_id in list_task_ids(repo_path)? {
        names.extend(
            read_task(repo_path, &task_id)?
                .resources
                .into_iter()
                .filter_map(|resource| resource.name),
        );
    }
    Ok(names)
}

pub(crate) fn read_task(repo_path: &Path, task_id: &str) -> Result<TaskMetadata> {
    let path = task_dir(repo_path, task_id)?.join(METADATA_FILE_NAME);
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not a task", path.display()))
}

pub(crate) fn write_task(repo_path: &Path, task_id: &str, metadata: &TaskMetadata) -> Result<()> {
    ensure_tasks_root(repo_path)?;
    let dir = task_dir(repo_path, task_id)?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let text = serde_json::to_string_pretty(metadata)?;
    let path = dir.join(METADATA_FILE_NAME);
    fs::write(&path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Create a task folder at one end of the given column and return the id it was given.
pub(crate) fn create_task(
    repo_path: &Path,
    title: &str,
    status: &ColumnId,
    joins: ColumnEnd,
) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a task needs a title");
    }
    // A card in a column the board does not have has nowhere to be drawn, so the mistake is
    // refused here rather than written down.
    if !read_board(repo_path).has(status) {
        bail!("the board has no {status} column");
    }
    let task_id = format!("{}-{}", slug_of(title), new_uuid());
    let position = match joins {
        ColumnEnd::Top => {
            make_room_at_the_top(repo_path, status)?;
            0
        }
        ColumnEnd::Bottom => position_under_the_column(repo_path, status),
    };
    let now = now_unix();
    let metadata = TaskMetadata {
        title: title.to_string(),
        created_at_unix: now,
        entered_column_at_unix: Some(now),
        position,
        status: status.clone(),
        tags: Vec::new(),
        resources: Vec::new(),
    };
    write_task(repo_path, &task_id, &metadata)?;
    ensure_notes_file(repo_path, &task_id)?;
    Ok(task_id)
}

/// The whole of a task's notes file. A task without one has nothing written yet, which reads
/// as the empty string it is - the card draws its box either way.
pub(crate) fn read_notes(repo_path: &Path, task_id: &str) -> String {
    task_dir(repo_path, task_id)
        .and_then(|dir| {
            fs::read_to_string(dir.join(crate::moontasks::NOTES_FILE_NAME))
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_default()
}

/// Write the whole of a task's notes file, creating it if it is not there.
pub(crate) fn write_notes(repo_path: &Path, task_id: &str, content: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(crate::moontasks::NOTES_FILE_NAME);
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

/// Make sure the task's notes file exists, without touching what anyone wrote in it.
///
/// Empty rather than seeded: the card's title already sits right above the notes box, and the
/// file pane can only open a file that is really there.
pub(crate) fn ensure_notes_file(repo_path: &Path, task_id: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    if dir.join(crate::moontasks::NOTES_FILE_NAME).is_file() {
        return Ok(());
    }
    write_notes(repo_path, task_id, "")
}

/// Make the project's work log if it is not there yet: a file holding only the line a new
/// entry goes above - see [`crate::native::work_log`]. Made once and never rewritten: from
/// then on it is the person's, and only ever written through the file pane.
pub(crate) fn ensure_work_log_file(repo_path: &Path) -> Result<()> {
    let root = ensure_tasks_root(repo_path)?;
    let path = root.join(crate::moontasks::WORK_LOG_FILE_NAME);
    if path.is_file() {
        return Ok(());
    }
    fs::write(&path, format!("{}\n", crate::native::work_log::NOW_MARKER))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// The position a card takes to sit under everything already in a column.
///
/// A board that cannot be read is a board with nothing in that column as far as this is
/// concerned: the new card goes to the top of it, which is no worse than anywhere else.
fn position_under_the_column(repo_path: &Path, status: &ColumnId) -> u32 {
    list_task_ids(repo_path)
        .unwrap_or_default()
        .iter()
        .filter_map(|task_id| read_task(repo_path, task_id).ok())
        .filter(|metadata| metadata.status == *status)
        .map(|metadata| metadata.position + 1)
        .max()
        .unwrap_or_default()
}

/// Push every card already in a column down one place, so a new one can have the top.
///
/// Moving each card down by one keeps the order they were in among themselves.
fn make_room_at_the_top(repo_path: &Path, status: &ColumnId) -> Result<()> {
    for task_id in list_task_ids(repo_path)? {
        let Ok(mut metadata) = read_task(repo_path, &task_id) else {
            // A task whose metadata cannot be read is skipped everywhere else too; it has no
            // place in the column to give up.
            continue;
        };
        if metadata.status != *status {
            continue;
        }
        metadata.position += 1;
        write_task(repo_path, &task_id, &metadata)?;
    }
    Ok(())
}

/// Take a task off the board without losing it: its folder, with anything an agent left in
/// it, moves under [`DELETED_TASKS_DIR_NAME`] and keeps the name it had.
///
/// Nothing the board reads looks in there - [`list_task_ids`] only takes folders that hold a
/// `metadata.json` themselves - so the card is gone from every column, and moving the folder
/// back up by hand is the whole of bringing it back.
pub(crate) fn delete_task(repo_path: &Path, task_id: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    if !dir.join(METADATA_FILE_NAME).is_file() {
        bail!("{task_id} is not a task");
    }
    let deleted_root = tasks_root(repo_path).join(DELETED_TASKS_DIR_NAME);
    fs::create_dir_all(&deleted_root)
        .with_context(|| format!("failed to create {}", deleted_root.display()))?;
    let kept_at = deleted_root.join(task_id);
    // A task id ends in a uuid, so a folder already kept under this name is the same task,
    // put back by hand and deleted again. Renaming over it would lose the first one.
    if kept_at.exists() {
        bail!("{} is already there", kept_at.display());
    }
    fs::rename(&dir, &kept_at)
        .with_context(|| format!("failed to move {} to {}", dir.display(), kept_at.display()))
}

#[cfg(test)]
#[path = "../store_tests.rs"]
mod tests;
