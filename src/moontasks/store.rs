//! The `.moontasks` folder: one directory per task, and the `metadata.json` inside it.
//!
//! The folder is the task. moonreview writes it, but nothing here assumes moonreview is the
//! only writer — a task can be created, renamed or moved between columns with a text editor,
//! and the board picks the change up on its next poll.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

/// The directory, in the repo being reviewed, that holds every task.
pub(crate) const TASKS_DIR_NAME: &str = ".moontasks";
const METADATA_FILE_NAME: &str = "metadata.json";
/// The MCP server definition written into a task folder, so an agent working in the task can
/// move it between columns itself.
pub(crate) const MCP_CONFIG_FILE_NAME: &str = "mcp.json";

/// Where a task sits on the board.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskStatus {
    Todo,
    InProgress,
    InLocalReview,
    InRemoteReview,
    Done,
}

/// The columns of the board, left to right. This is the whole order: a status that is not
/// here has no column to be drawn in.
pub(crate) const BOARD_COLUMNS: &[TaskStatus] = &[
    TaskStatus::Todo,
    TaskStatus::InProgress,
    TaskStatus::InLocalReview,
    TaskStatus::InRemoteReview,
    TaskStatus::Done,
];

/// What each status is called on the board and in the MCP tool an agent calls.
const STATUS_NAMES: &[(TaskStatus, &str, &str)] = &[
    (TaskStatus::Todo, "TODO", "todo"),
    (TaskStatus::InProgress, "IN PROGRESS", "in_progress"),
    (TaskStatus::InLocalReview, "IN LOCAL REVIEW", "in_local_review"),
    (
        TaskStatus::InRemoteReview,
        "IN REMOTE REVIEW",
        "in_remote_review",
    ),
    (TaskStatus::Done, "DONE", "done"),
];

impl TaskStatus {
    pub(crate) fn label(self) -> &'static str {
        STATUS_NAMES
            .iter()
            .find(|(status, _, _)| *status == self)
            .map(|(_, label, _)| *label)
            .expect("every status is named")
    }

    /// The name this status goes by on the wire and in `metadata.json`.
    pub(crate) fn wire_name(self) -> &'static str {
        STATUS_NAMES
            .iter()
            .find(|(status, _, _)| *status == self)
            .map(|(_, _, wire)| *wire)
            .expect("every status is named")
    }

    pub(crate) fn from_wire_name(name: &str) -> Option<Self> {
        STATUS_NAMES
            .iter()
            .find(|(_, _, wire)| *wire == name)
            .map(|(status, _, _)| *status)
    }

}

/// Whether a resource is a plain shell or an agent working on the task.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskResourceKind {
    Shell,
    Agent,
}

/// Something the task has running, or had running: a shell, or a run of an agent.
///
/// Whether it is running right now is not written down — the shells the server has are what
/// answers that, and they are gone once the server is.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskResource {
    pub(crate) id: String,
    pub(crate) kind: TaskResourceKind,
    /// Which agent this is a run of. `None` for a shell.
    #[serde(default)]
    pub(crate) agent: AgentKind,
    /// The shell it was last attached to. Kept after the shell ends so the board can tell
    /// which past run a resumed one continues.
    #[serde(default)]
    pub(crate) terminal_id: Option<String>,
    /// The id the agent was told to give its session, so that run can be resumed exactly
    /// rather than by whatever the agent thinks the most recent one was.
    #[serde(default)]
    pub(crate) agent_session_id: Option<String>,
    pub(crate) started_at_unix: u64,
}

/// The `metadata.json` of one task folder.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TaskMetadata {
    pub(crate) title: String,
    pub(crate) status: TaskStatus,
    pub(crate) created_at_unix: u64,
    /// Where the card sits in its column, lowest at the top. Renumbered from zero across the
    /// whole column whenever one is dragged into it, so the numbers stay small and readable
    /// in a file somebody may well edit by hand.
    ///
    /// A board written before cards could be reordered has none of these, which reads as a
    /// column of zeroes — and cards that tie fall back on the order they were created in,
    /// which is the order that board was already drawn in.
    #[serde(default)]
    pub(crate) position: u32,
    #[serde(default)]
    pub(crate) resources: Vec<TaskResource>,
}

pub(crate) fn tasks_root(repo_path: &Path) -> PathBuf {
    repo_path.join(TASKS_DIR_NAME)
}

/// What the board's own `.gitignore` says.
///
/// A board is working state — running agents, scratch files, an MCP config carrying this
/// run's port — and none of that belongs in someone's `git status` by default. The file
/// ignores the whole folder including itself, so a repo where moonreview has been opened
/// looks exactly like one where it has not.
///
/// Someone who wants the board shared can delete this file and commit the folder, which is
/// why it is written once and never rewritten.
const TASKS_GITIGNORE: &str = "\
# Written by moonreview when it created this board.
#
# A task folder holds running state — shells, agent sessions, scratch files — so by default
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
    for entry in fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
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

/// Create a task folder and return the id it was given.
pub(crate) fn create_task(repo_path: &Path, title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a task needs a title");
    }
    let task_id = format!("{}-{}", slug_of(title), new_uuid());
    let metadata = TaskMetadata {
        title: title.to_string(),
        status: TaskStatus::Todo,
        created_at_unix: now_unix(),
        // A new card joins the bottom of TODO, where the newest card has always been drawn.
        position: next_position(repo_path, TaskStatus::Todo),
        resources: Vec::new(),
    };
    write_task(repo_path, &task_id, &metadata)?;
    Ok(task_id)
}

/// The position a card takes to sit under everything already in a column.
///
/// A board that cannot be read is a board with nothing in that column as far as this is
/// concerned: the new card goes to the top of it, which is no worse than anywhere else.
fn next_position(repo_path: &Path, status: TaskStatus) -> u32 {
    list_task_ids(repo_path)
        .unwrap_or_default()
        .iter()
        .filter_map(|task_id| read_task(repo_path, task_id).ok())
        .filter(|metadata| metadata.status == status)
        .map(|metadata| metadata.position + 1)
        .max()
        .unwrap_or_default()
}

/// Drop the whole task folder, including anything an agent left in it.
pub(crate) fn delete_task(repo_path: &Path, task_id: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    if !dir.join(METADATA_FILE_NAME).is_file() {
        bail!("{task_id} is not a task");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("failed to remove {}", dir.display()))
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

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A version 4 UUID, from the same entropy the standard library seeds its hashers with.
pub(crate) fn new_uuid() -> String {
    let (high, low) = (random_u64(), random_u64());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        high >> 32,
        (high >> 16) & 0xffff,
        high & 0x0fff,
        // The variant bits: `10xx`, which puts the first character in `8..=b`.
        0x8000 | (low >> 48) & 0x3fff,
        low & 0xffff_ffff_ffff_u64
    )
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};

    // `RandomState` takes its keys from the operating system, and moves them on every call,
    // which is exactly the per-call randomness an id needs.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default(),
    );
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonreview-tasks-{}-{name}-{}",
            std::process::id(),
            new_uuid()
        ));
        fs::create_dir_all(&path).expect("failed to create the test repo");
        path
    }

    #[test]
    fn a_title_becomes_a_readable_slug() {
        assert_eq!(slug_of("Fix the login page"), "fix-the-login-page");
        assert_eq!(slug_of("  Ünïcode & symbols!! "), "ünïcode-symbols");
        assert_eq!(slug_of("***"), "task");
        assert!(slug_of(&"word ".repeat(40)).len() <= 44);
    }

    #[test]
    fn uuids_are_version_four_and_distinct() {
        let one = new_uuid();
        let two = new_uuid();

        assert_ne!(one, two);
        assert_eq!(one.len(), 36);
        assert_eq!(one.as_bytes()[14], b'4', "expected a version 4 uuid: {one}");
        assert!(matches!(one.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn a_created_task_reads_back_and_lists() {
        let repo = temp_repo("roundtrip");

        let task_id = create_task(&repo, "Fix the login page").expect("expected a task");
        assert!(task_id.starts_with("fix-the-login-page-"));

        let metadata = read_task(&repo, &task_id).expect("expected metadata");
        assert_eq!(metadata.title, "Fix the login page");
        assert_eq!(metadata.status, TaskStatus::Todo);
        assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    #[test]
    fn a_directory_without_metadata_is_not_a_task() {
        let repo = temp_repo("stray");
        fs::create_dir_all(tasks_root(&repo).join("notes")).expect("failed to create a directory");

        assert!(list_task_ids(&repo).expect("expected a listing").is_empty());

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A board is working state, so opening moonreview in a repo must not put anything in
    /// somebody's `git status`.
    #[test]
    fn a_new_board_ignores_itself() {
        let repo = temp_repo("gitignore");

        create_task(&repo, "Fix the login page").expect("expected a task");

        let ignore = tasks_root(&repo).join(".gitignore");
        let text = fs::read_to_string(&ignore).expect("expected a .gitignore");
        assert!(
            text.lines().any(|line| line.trim() == "*"),
            "the board should ignore everything in it, got: {text}"
        );
        assert!(
            text.contains("Delete this file"),
            "it should say how to share the board instead"
        );

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// Once it exists the file is the user's: a board they chose to share by deleting it must
    /// not start ignoring itself again the next time a task is made.
    #[test]
    fn a_board_that_has_been_shared_is_left_shared() {
        let repo = temp_repo("gitignore-removed");
        create_task(&repo, "First").expect("expected a task");
        let ignore = tasks_root(&repo).join(".gitignore");
        fs::remove_file(&ignore).expect("failed to remove the .gitignore");

        create_task(&repo, "Second").expect("expected another task");

        assert!(!ignore.exists(), "the .gitignore should not have come back");

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// The `.gitignore` sits beside the task folders, and is not one of them.
    #[test]
    fn the_boards_own_files_are_not_listed_as_tasks() {
        let repo = temp_repo("gitignore-listing");
        let task_id = create_task(&repo, "Fix the login page").expect("expected a task");

        assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    #[test]
    fn a_task_id_may_not_climb_out_of_the_tasks_folder() {
        assert!(task_dir(Path::new("/repo"), "../secrets").is_err());
        assert!(task_dir(Path::new("/repo"), "a/b").is_err());
        assert!(task_dir(Path::new("/repo"), "").is_err());
    }

    /// The order the board draws its columns in, which is also the order work moves through.
    #[test]
    fn the_columns_run_left_to_right() {
        let order: Vec<&str> = BOARD_COLUMNS
            .iter()
            .map(|status| status.wire_name())
            .collect();

        assert_eq!(
            order,
            [
                "todo",
                "in_progress",
                "in_local_review",
                "in_remote_review",
                "done"
            ]
        );
    }

    #[test]
    fn statuses_survive_a_round_trip_through_their_wire_name() {
        for status in BOARD_COLUMNS {
            assert_eq!(TaskStatus::from_wire_name(status.wire_name()), Some(*status));
            // The wire name is what `metadata.json` holds, so serde has to agree with it.
            let encoded = serde_json::to_string(status).expect("expected json");
            assert_eq!(encoded, format!("\"{}\"", status.wire_name()));
        }
        assert_eq!(TaskStatus::from_wire_name("blocked"), None);
    }
}
