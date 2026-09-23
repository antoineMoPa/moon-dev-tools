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
fn a_tag_is_kept_in_one_spelling() {
    assert_eq!(tag_of("Needs Tests"), Some("needs-tests".to_string()));
    assert_eq!(tag_of("  needs   tests "), Some("needs-tests".to_string()));
    assert_eq!(tag_of("needs-tests"), Some("needs-tests".to_string()));
    assert_eq!(tag_of("v2_api"), Some("v2_api".to_string()));
    assert_eq!(tag_of("Ünïcode!"), Some("ünïcode".to_string()));
    assert_eq!(tag_of("***"), None);
    assert_eq!(tag_of(""), None);
}

#[test]
fn a_list_of_tags_says_nothing_twice_and_keeps_its_order() {
    assert_eq!(
        tags_of(["Bug", "needs tests", "bug", "", "Needs-Tests", "ui"]),
        ["bug", "needs-tests", "ui"]
    );
}

#[test]
fn a_task_written_before_tags_reads_back_without_any() {
    let repo = temp_repo("no-tags");
    let dir = tasks_root(&repo).join("old-task-1111");
    fs::create_dir_all(&dir).expect("failed to create the task folder");
    fs::write(
        dir.join(METADATA_FILE_NAME),
        "{\n  \"title\": \"Old task\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    )
    .expect("failed to write the task");

    let metadata = read_task(&repo, "old-task-1111").expect("expected the task to read");
    assert!(metadata.tags.is_empty());

    // And a task with none keeps its file free of the field: a board nobody has tagged
    // reads exactly as it did.
    write_task(&repo, "old-task-1111", &metadata).expect("expected the task written");
    let text = fs::read_to_string(dir.join(METADATA_FILE_NAME)).expect("expected the file");
    assert!(!text.contains("tags"), "{text}");

    fs::remove_dir_all(&repo).ok();
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

    let task_id = create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .expect("expected a task");
    assert!(task_id.starts_with("fix-the-login-page-"));

    let metadata = read_task(&repo, &task_id).expect("expected metadata");
    assert_eq!(metadata.title, "Fix the login page");
    assert_eq!(metadata.status, ColumnId::new("todo"));
    assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

/// Deleting a task takes its card off the board and keeps the folder, with whatever was
/// left in it, where it can be moved back from.
#[test]
fn a_deleted_task_leaves_the_board_and_keeps_its_folder() {
    let repo = temp_repo("soft-delete");
    let task_id = create_task(&repo, "Keep me", &ColumnId::new("todo"), ColumnEnd::Top)
        .expect("expected a task");
    write_notes(&repo, &task_id, "what the agent found").expect("expected notes written");

    delete_task(&repo, &task_id).expect("expected the task deleted");

    assert!(list_task_ids(&repo).expect("expected a listing").is_empty());
    assert!(read_task(&repo, &task_id).is_err());
    let kept_at = tasks_root(&repo).join(DELETED_TASKS_DIR_NAME).join(&task_id);
    assert!(kept_at.join(METADATA_FILE_NAME).is_file());
    assert_eq!(
        fs::read_to_string(kept_at.join(crate::moontasks::NOTES_FILE_NAME))
            .expect("expected the notes kept"),
        "what the agent found"
    );

    // Moved back up by hand, it is a task again.
    fs::rename(&kept_at, tasks_root(&repo).join(&task_id)).expect("expected it moved back");
    assert_eq!(list_task_ids(&repo).expect("expected a listing"), [task_id]);

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

/// A card joins the end of the column its `+` was pressed at, and the cards already there
/// keep the order they were in.
#[test]
fn a_card_joins_the_end_of_the_column_it_was_asked_for() {
    let repo = temp_repo("column-ends");
    let todo = ColumnId::new("todo");

    let first = create_task(&repo, "First", &todo, ColumnEnd::Top).expect("expected a task");
    let second = create_task(&repo, "Second", &todo, ColumnEnd::Top).expect("expected a task");
    let last = create_task(&repo, "Last", &todo, ColumnEnd::Bottom).expect("expected a task");

    let place_of = |task_id: &str| {
        read_task(&repo, task_id)
            .expect("expected metadata")
            .position
    };
    assert_eq!(place_of(&second), 0, "the newest card asked for the top");
    assert_eq!(place_of(&first), 1, "the card it pushed down");
    assert_eq!(place_of(&last), 2, "the card that asked for the bottom");

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

/// The notes file is there from the moment the task is, because the file pane can only
/// open a file that really exists.
#[test]
fn a_created_task_has_an_empty_notes_file() {
    let repo = temp_repo("notes-created");
    let task_id = create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .expect("expected a task");

    let path = tasks_root(&repo).join(&task_id).join("notes.md");
    assert!(path.is_file(), "expected {} to exist", path.display());
    assert_eq!(read_notes(&repo, &task_id), "");

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

#[test]
fn notes_round_trip_and_are_never_clobbered_by_ensure() {
    let repo = temp_repo("notes-roundtrip");
    let task_id = create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
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

    create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .expect("expected a task");

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
    create_task(&repo, "First", &ColumnId::new("todo"), ColumnEnd::Top)
        .expect("expected a task");
    let ignore = tasks_root(&repo).join(".gitignore");
    fs::remove_file(&ignore).expect("failed to remove the .gitignore");

    create_task(&repo, "Second", &ColumnId::new("todo"), ColumnEnd::Top)
        .expect("expected another task");

    assert!(!ignore.exists(), "the .gitignore should not have come back");

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

/// The `.gitignore` sits beside the task folders, and is not one of them.
#[test]
fn the_boards_own_files_are_not_listed_as_tasks() {
    let repo = temp_repo("gitignore-listing");
    let task_id = create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
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

    assert_eq!(order, ["todo", "in_progress", "done"]);

    fs::remove_dir_all(repo).expect("failed to remove the test repo");
}

/// The one rule the board applies on its own points at a default column, so a board that
/// has not been changed behaves exactly as it always did.
#[test]
fn the_board_s_own_rule_points_at_a_column_it_starts_with() {
    let config = BoardConfig::default();

    assert_eq!(
        config.role(RELEASES_SHELLS_IN),
        Some(ColumnId::new(RELEASES_SHELLS_IN)),
        "{RELEASES_SHELLS_IN} should be a column of a new board"
    );
}

/// A rule whose column has been deleted is off rather than pointing somewhere else: a card
/// must not have its shells taken away in a column nobody pinned the rule to.
#[test]
fn a_rule_whose_column_is_gone_points_nowhere() {
    let mut config = BoardConfig::default();
    config
        .columns
        .retain(|column| column.id.as_str() != RELEASES_SHELLS_IN);

    assert_eq!(config.role(RELEASES_SHELLS_IN), None);
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
                arrivals: None,
                sort: Some(ColumnSort::Alphabetical),
            },
            BoardColumn {
                id: ColumnId::new("shipped"),
                label: "SHIPPED".to_string(),
                arrivals: Some(ColumnEnd::Top),
                sort: None,
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

/// A card is stamped with the moment it joined its column, which for a card just made is the
/// moment it was made.
#[test]
fn a_new_task_is_stamped_with_when_it_joined_its_column() {
    let repo = temp_repo("stamped");
    let before = now_unix();

    let task_id = create_task(
        &repo,
        "Fix the races",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .expect("expected the task to be created");

    let metadata = read_task(&repo, &task_id).expect("expected the task to read");
    let stamped = metadata
        .entered_column_at_unix
        .expect("expected the arrival to be written down");
    assert!(stamped >= before && stamped <= now_unix(), "{stamped}");
    assert_eq!(stamped, metadata.created_at_unix);
}
