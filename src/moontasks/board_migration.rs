//! Bringing a board written before a column's name was the whole of it up to date.
//!
//! A column used to carry two things: an `id` that cards were filed under, and a `label` the
//! heading drew. They were made from each other once and then drifted — renaming a column
//! changed only the label — so a board could end up drawing "IN REVIEW" over a column every
//! script had to call `in_local_review`. There is one name now.
//!
//! This reads the old shape and writes the new one, once per board: the label becomes the
//! name, and every card filed under the old id is refiled under it. The three columns a board
//! started with are handled whether or not that board ever grew a `board.json`, because
//! `in_progress` is not the same string as `IN PROGRESS` and those cards would otherwise be
//! filed in a column the board does not have.
//!
//! It can go once no board in use has been left unopened since. Deleting this file and its one
//! call in [`super::store::read_board`] is the whole of it.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use serde::Deserialize;

use crate::api::AgentKind;

use super::store::{BoardColumn, BoardConfig, ColumnName};

/// The board file as it was written when a column had an id behind its label.
#[derive(Deserialize)]
struct BoardWithColumnIds {
    columns: Vec<ColumnWithId>,
    #[serde(default)]
    hooks_running: bool,
}

#[derive(Deserialize)]
struct ColumnWithId {
    id: String,
    label: String,
    #[serde(default)]
    default_agent: Option<AgentKind>,
}

/// What the columns a board started with were called before and after.
const DEFAULT_COLUMN_IDS: &[(&str, &str)] = &[
    ("todo", "TODO"),
    ("in_progress", "IN PROGRESS"),
    ("done", "DONE"),
];

/// The boards this process has already been through, so opening one is not a sweep of every
/// card each time the board is read.
fn already_done() -> &'static Mutex<HashSet<PathBuf>> {
    static DONE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    DONE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Upgrade a board's file and its cards, and hand back the columns the file now has.
///
/// `None` means there was nothing of the old shape to do, and the board's own reading of the
/// file stands. The card sweep still runs in that case: a board that never grew a file has old
/// ids on its cards all the same.
///
/// Nothing here is worth failing a read over. A card that cannot be read or written is left as
/// it is, and is as unreachable as it was before — the board draws either way.
pub(super) fn upgrade(repo_path: &Path, board_file: Option<&str>) -> Option<BoardConfig> {
    {
        let mut done = already_done().lock().expect("the upgrade list is poisoned");
        if !done.insert(repo_path.to_path_buf()) {
            return None;
        }
    }

    let upgraded = board_file.and_then(board_with_names);
    let mut renames: HashMap<String, ColumnName> = DEFAULT_COLUMN_IDS
        .iter()
        .map(|(id, name)| ((*id).to_string(), ColumnName::new(*name)))
        .collect();
    if let Some((config, from_ids)) = &upgraded {
        renames.extend(from_ids.clone());
        // A card that cannot be refiled is worse than a file left alone, so the cards go first
        // and the board file only follows if that worked.
        if super::store::write_board(repo_path, config).is_err() {
            return None;
        }
    }
    refile_cards(repo_path, &renames);

    upgraded.map(|(config, _)| config)
}

/// The old board file read as the new one, with what each old id is now called beside it.
fn board_with_names(text: &str) -> Option<(BoardConfig, HashMap<String, ColumnName>)> {
    let old: BoardWithColumnIds = serde_json::from_str(text).ok()?;
    if old.columns.is_empty() {
        return None;
    }

    let mut config = BoardConfig {
        columns: Vec::new(),
        hooks_running: old.hooks_running,
    };
    let mut renames = HashMap::new();
    for column in old.columns {
        // Two columns whose labels only differ by case were two columns before and have to
        // stay two, so the one that cannot have its label keeps the id it was filed under.
        let mut name = ColumnName::new(column.label);
        if config.has(&name) {
            name = ColumnName::new(column.id.clone());
        }
        if config.has(&name) {
            continue;
        }
        renames.insert(column.id, name.clone());
        config.columns.push(BoardColumn {
            name,
            default_agent: column.default_agent,
        });
    }
    Some((config, renames))
}

/// Refile every card whose column is named by one of the old ids.
fn refile_cards(repo_path: &Path, renames: &HashMap<String, ColumnName>) {
    let Ok(task_ids) = super::store::list_task_ids(repo_path) else {
        return;
    };
    for task_id in task_ids {
        let Ok(mut metadata) = super::store::read_task(repo_path, &task_id) else {
            continue;
        };
        // Only an exact match: a card already saying `TODO` is not an old id, and one saying
        // `todo` on a board with a TODO column is already in it.
        let Some(name) = renames.get(metadata.status.as_str()) else {
            continue;
        };
        // Spelling rather than [`ColumnName`]'s own reading of it: `backlog` and `BACKLOG` are
        // the same column already, and the point of this is that the file says what the
        // heading says.
        if metadata.status.as_str() == name.as_str() {
            continue;
        }
        metadata.status = name.clone();
        let _ = super::store::write_task(repo_path, &task_id, &metadata);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::store::{self, ColumnName};

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonreview-migration-{}-{name}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        fs::create_dir_all(path.join(store::TASKS_DIR_NAME)).expect("failed to create the repo");
        path
    }

    fn write_card(repo: &std::path::Path, task_id: &str, status: &str) {
        let dir = repo.join(store::TASKS_DIR_NAME).join(task_id);
        fs::create_dir_all(&dir).expect("failed to create the task folder");
        fs::write(
            dir.join("metadata.json"),
            format!(
                "{{\n  \"title\": \"Fix the login page\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        )
        .expect("failed to write the task");
    }

    /// The board this was written for: a column whose heading said one thing and whose cards
    /// were filed under another.
    #[test]
    fn a_column_with_an_id_behind_its_label_becomes_the_label() {
        let repo = temp_repo("labels");
        fs::write(
            repo.join(store::TASKS_DIR_NAME).join("board.json"),
            r#"{"columns":[{"id":"todo","label":"BACKLOG","default_agent":"claude"},
                           {"id":"in_local_review","label":"IN REVIEW"}],"hooks_running":true}"#,
        )
        .expect("failed to write the board");
        write_card(&repo, "waiting-1111", "in_local_review");

        let board = store::read_board(&repo);

        let names: Vec<&str> = board
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        assert_eq!(names, ["BACKLOG", "IN REVIEW"]);
        assert!(board.hooks_running, "the board was left running");
        assert_eq!(
            store::read_task(&repo, "waiting-1111")
                .expect("expected the card")
                .status,
            ColumnName::new("IN REVIEW"),
            "the card should have followed its column's heading"
        );

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A board that never grew a file has cards filed under the ids the three default columns
    /// used to have, and `in_progress` is not `IN PROGRESS` to anyone.
    #[test]
    fn cards_filed_under_the_old_default_ids_are_refiled() {
        let repo = temp_repo("defaults");
        write_card(&repo, "working-1111", "in_progress");

        let board = store::read_board(&repo);

        assert_eq!(board, store::BoardConfig::default());
        assert_eq!(
            store::read_task(&repo, "working-1111")
                .expect("expected the card")
                .status,
            ColumnName::new("IN PROGRESS")
        );

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A board already written the new way is read as it is, cards and all.
    #[test]
    fn a_board_that_is_already_names_is_left_alone() {
        let repo = temp_repo("current");
        fs::write(
            repo.join(store::TASKS_DIR_NAME).join("board.json"),
            r#"{"columns":[{"name":"TODO"},{"name":"SHIPPED"}],"hooks_running":false}"#,
        )
        .expect("failed to write the board");
        write_card(&repo, "shipped-1111", "SHIPPED");

        let board = store::read_board(&repo);

        let names: Vec<&str> = board
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect();
        assert_eq!(names, ["TODO", "SHIPPED"]);
        assert_eq!(
            store::read_task(&repo, "shipped-1111")
                .expect("expected the card")
                .status,
            ColumnName::new("SHIPPED")
        );

        fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }
}
