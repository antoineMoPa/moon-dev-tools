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
/// The board's own file, beside the task folders: what its columns are and what they are
/// called. A board without one has the columns in [`DEFAULT_COLUMNS`], and only grows the file
/// once someone changes them.
const BOARD_FILE_NAME: &str = "board.json";

/// Which column a task sits in, by the name that column goes by in the board's file.
///
/// A name rather than one of a fixed set: the columns belong to the board, and a card says
/// which of them it is in. This is what `metadata.json` has always held in its `status`, so a
/// board written before columns could be changed reads back unchanged.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ColumnId(String);

impl ColumnId {
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ColumnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One column of the board: the name cards are in it under, and what it is called on screen.
///
/// Renaming changes the label and leaves the id alone, so every card already in the column
/// stays in it and a board someone renamed a column on is still readable by hand.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct BoardColumn {
    pub(crate) id: ColumnId,
    pub(crate) label: String,
    /// The agent the last task created in this column was started with. The new-task box
    /// offers it first, so a column that always goes to the same agent only has to be told
    /// once. Absent until a task has been created in the column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_agent: Option<AgentKind>,
}

/// The columns a board starts with, left to right.
pub(crate) const DEFAULT_COLUMNS: &[(&str, &str)] = &[
    ("todo", "TODO"),
    ("in_progress", "IN PROGRESS"),
    ("done", "DONE"),
];

/// The two columns the board itself acts on, by the id they have on a board that started
/// from the defaults.
///
/// These are the only card movements moonreview makes on its own, so they need a column to
/// point at. They are pinned by id: renaming a column or dragging it somewhere else keeps its
/// part in these rules, and deleting it turns that rule off rather than sending cards to a
/// column nobody chose. A board built from scratch out of columns of its own has no automatic
/// movement at all, which is the honest answer to not having said where it should go.
pub(crate) const STARTS_IN: &str = "in_progress";
pub(crate) const RELEASES_SHELLS_IN: &str = "done";

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
                .map(|(id, label)| BoardColumn {
                    id: ColumnId::new(*id),
                    label: (*label).to_string(),
                    default_agent: None,
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

    /// The column one of the board's own rules points at, if it is still on the board.
    pub(crate) fn role(&self, role: &str) -> Option<ColumnId> {
        let id = ColumnId::new(role);
        self.has(&id).then_some(id)
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
    /// The column the card is in, by the id the board's file gives it.
    pub(crate) status: ColumnId,
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
/// A board is working state — running agents, scratch files, whatever an agent leaves in a
/// task folder — and none of that belongs in someone's `git status` by default. The file
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

/// The board's columns, as its file has them — or the defaults, for a board that has never
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

/// Create a task folder in the given column and return the id it was given.
pub(crate) fn create_task(repo_path: &Path, title: &str, status: &ColumnId) -> Result<String> {
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
    let metadata = TaskMetadata {
        title: title.to_string(),
        created_at_unix: now_unix(),
        position: next_position(repo_path, status),
        status: status.clone(),
        resources: Vec::new(),
    };
    write_task(repo_path, &task_id, &metadata)?;
    ensure_notes_file(repo_path, &task_id)?;
    Ok(task_id)
}

/// The whole of a task's notes file. A task without one has nothing written yet, which reads
/// as the empty string it is — the card draws its box either way.
pub(crate) fn read_notes(repo_path: &Path, task_id: &str) -> String {
    task_dir(repo_path, task_id)
        .and_then(|dir| {
            fs::read_to_string(dir.join(super::NOTES_FILE_NAME)).map_err(anyhow::Error::from)
        })
        .unwrap_or_default()
}

/// Write the whole of a task's notes file, creating it if it is not there.
pub(crate) fn write_notes(repo_path: &Path, task_id: &str, content: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(super::NOTES_FILE_NAME);
    fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

/// Make sure the task's notes file exists, without touching what anyone wrote in it.
///
/// Empty rather than seeded: the card's title already sits right above the notes box, and the
/// file pane can only open a file that is really there.
pub(crate) fn ensure_notes_file(repo_path: &Path, task_id: &str) -> Result<()> {
    let dir = task_dir(repo_path, task_id)?;
    if dir.join(super::NOTES_FILE_NAME).is_file() {
        return Ok(());
    }
    write_notes(repo_path, task_id, "")
}

/// The position a card takes to sit under everything already in a column.
///
/// A board that cannot be read is a board with nothing in that column as far as this is
/// concerned: the new card goes to the top of it, which is no worse than anywhere else.
fn next_position(repo_path: &Path, status: &ColumnId) -> u32 {
    list_task_ids(repo_path)
        .unwrap_or_default()
        .iter()
        .filter_map(|task_id| read_task(repo_path, task_id).ok())
        .filter(|metadata| metadata.status == *status)
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

        let task_id = create_task(&repo, "Fix the login page", &ColumnId::new("todo"))
            .expect("expected a task");
        assert!(task_id.starts_with("fix-the-login-page-"));

        let metadata = read_task(&repo, &task_id).expect("expected metadata");
        assert_eq!(metadata.title, "Fix the login page");
        assert_eq!(metadata.status, ColumnId::new("todo"));
        assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// The notes file is there from the moment the task is, because the file pane can only
    /// open a file that really exists.
    #[test]
    fn a_created_task_has_an_empty_notes_file() {
        let repo = temp_repo("notes-created");
        let task_id = create_task(&repo, "Fix the login page", &ColumnId::new("todo"))
            .expect("expected a task");

        let path = tasks_root(&repo).join(&task_id).join("notes.md");
        assert!(path.is_file(), "expected {} to exist", path.display());
        assert_eq!(read_notes(&repo, &task_id), "");

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    #[test]
    fn notes_round_trip_and_are_never_clobbered_by_ensure() {
        let repo = temp_repo("notes-roundtrip");
        let task_id = create_task(&repo, "Fix the login page", &ColumnId::new("todo"))
            .expect("expected a task");

        write_notes(&repo, &task_id, "what the login fix is about\n")
            .expect("expected the notes to be written");
        ensure_notes_file(&repo, &task_id).expect("expected ensure to succeed");

        assert_eq!(read_notes(&repo, &task_id), "what the login fix is about\n");

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

        create_task(&repo, "Fix the login page", &ColumnId::new("todo")).expect("expected a task");

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
        create_task(&repo, "First", &ColumnId::new("todo")).expect("expected a task");
        let ignore = tasks_root(&repo).join(".gitignore");
        fs::remove_file(&ignore).expect("failed to remove the .gitignore");

        create_task(&repo, "Second", &ColumnId::new("todo")).expect("expected another task");

        assert!(!ignore.exists(), "the .gitignore should not have come back");

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// The `.gitignore` sits beside the task folders, and is not one of them.
    #[test]
    fn the_boards_own_files_are_not_listed_as_tasks() {
        let repo = temp_repo("gitignore-listing");
        let task_id = create_task(&repo, "Fix the login page", &ColumnId::new("todo"))
            .expect("expected a task");

        assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    #[test]
    fn a_task_id_may_not_climb_out_of_the_tasks_folder() {
        assert!(task_dir(Path::new("/repo"), "../secrets").is_err());
        assert!(task_dir(Path::new("/repo"), "a/b").is_err());
        assert!(task_dir(Path::new("/repo"), "").is_err());
    }

    /// The columns a board starts with, which is also the order work moves through.
    #[test]
    fn a_board_with_no_file_has_the_default_columns_left_to_right() {
        let repo = temp_repo("default-columns");

        let board = read_board(&repo);
        let order: Vec<&str> = board
            .columns
            .iter()
            .map(|column| column.id.as_str())
            .collect();

        assert_eq!(
            order,
            ["todo", "in_progress", "done"]
        );

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// Every rule the board applies on its own points at one of the default columns, so a
    /// board that has not been changed behaves exactly as it always did.
    #[test]
    fn the_board_s_own_rules_point_at_columns_it_starts_with() {
        let config = BoardConfig::default();

        for role in [STARTS_IN, RELEASES_SHELLS_IN] {
            assert_eq!(
                config.role(role),
                Some(ColumnId::new(role)),
                "{role} should be a column of a new board"
            );
        }
    }

    /// A rule whose column has been deleted is off rather than pointing somewhere else: a card
    /// must not be sent to a column nobody chose for it.
    #[test]
    fn a_rule_whose_column_is_gone_points_nowhere() {
        let mut config = BoardConfig::default();
        config
            .columns
            .retain(|column| column.id.as_str() != STARTS_IN);

        assert_eq!(config.role(STARTS_IN), None);
        assert_eq!(
            config.role(RELEASES_SHELLS_IN),
            Some(ColumnId::new(RELEASES_SHELLS_IN))
        );
    }

    #[test]
    fn a_column_id_is_written_down_as_the_plain_string_it_has_always_been() {
        let encoded = serde_json::to_string(&ColumnId::new("quality_review")).expect("json");

        assert_eq!(encoded, "\"quality_review\"");
        assert_eq!(
            serde_json::from_str::<ColumnId>(&encoded).expect("expected a column"),
            ColumnId::new("quality_review")
        );
    }

    #[test]
    fn the_columns_survive_a_round_trip_through_the_board_file() {
        let repo = temp_repo("board-roundtrip");
        let config = BoardConfig {
            columns: vec![
                BoardColumn {
                    id: ColumnId::new("todo"),
                    label: "BACKLOG".to_string(),
                    default_agent: Some(AgentKind::Claude),
                },
                BoardColumn {
                    id: ColumnId::new("shipped"),
                    label: "SHIPPED".to_string(),
                    default_agent: None,
                },
            ],
        };

        write_board(&repo, &config).expect("expected the board to be written");

        assert_eq!(read_board(&repo), config);

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A board file that makes no sense is the defaults, the same way a broken `metadata.json`
    /// is a card left out rather than a board that will not draw.
    #[test]
    fn a_board_file_that_cannot_be_read_falls_back_on_the_defaults() {
        let repo = temp_repo("board-broken");
        write_board(&repo, &BoardConfig::default()).expect("expected the board to be written");
        fs::write(tasks_root(&repo).join("board.json"), "{ not json")
            .expect("failed to write the board file");

        assert_eq!(read_board(&repo), BoardConfig::default());

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A board with no columns has nowhere to put a card, which is worse than one that was
    /// never customised.
    #[test]
    fn a_board_with_no_columns_falls_back_on_the_defaults() {
        let repo = temp_repo("board-empty");
        write_board(
            &repo,
            &BoardConfig {
                columns: Vec::new(),
            },
        )
        .expect("expected the board to be written");

        assert_eq!(read_board(&repo), BoardConfig::default());

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }
}
