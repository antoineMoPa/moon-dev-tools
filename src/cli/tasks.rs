//! Tasks - the cards of a board made, listed and moved from a command line: the words after
//! `moon tasks` that open no window.
//!
//! The board is a folder of files, so a card can be made, read and moved without a window -
//! which is what an agent asked to write itself a task, or to say how far it has got with
//! the one it is on, needs. What somebody running them is told is in the window's own help,
//! see `help_text_for` in [`super::command`].

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{Context, Result, bail};

use super::{MoonCommand, PROGRAM, task_shell::TaskOfShell};
use crate::{
    git::project_root,
    moontasks::{
        ColumnEnd, TASK_DIR_ENV_VAR, service,
        store::{self, BoardColumn},
        wire::handles,
    },
    terminal::TerminalRegistry,
};

/// What `moon tasks` does to the board's cards rather than open its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CardCommand {
    New,
    List,
    Move,
}

/// The words after `moon tasks` that are not something to open the window on, and what each
/// asks for. A file or a folder called one of these is opened as `./new`.
const CARD_COMMANDS: &[(&str, CardCommand)] = &[
    ("new", CardCommand::New),
    ("list", CardCommand::List),
    ("move", CardCommand::Move),
];

pub(super) fn card_command_named(word: &str) -> Option<CardCommand> {
    CARD_COMMANDS
        .iter()
        .find(|(named, _)| *named == word)
        .map(|(_, command)| *command)
}

/// What a command line asks of the cards, from the words after `moon tasks new|list|move`.
pub(super) fn parse(command: CardCommand, words: &[String]) -> Result<MoonCommand> {
    match command {
        CardCommand::New => parse_new(words),
        CardCommand::List => match words.is_empty() {
            true => Ok(MoonCommand::ListTasks),
            false => bail!(
                "`{PROGRAM} tasks list` lists the cards of this repo's board, so it takes nothing"
            ),
        },
        CardCommand::Move => parse_move(words),
    }
}

/// `moon tasks new <title>`. The title is the whole of the rest, joined, so it needs no
/// quoting - though it usually gets some.
fn parse_new(args: &[String]) -> Result<MoonCommand> {
    if let Some(option) = args.iter().find(|arg| arg.starts_with('-')) {
        bail!("`{PROGRAM} tasks new` takes the card's title and no options, not {option}");
    }
    let title = args.join(" ");
    if title.trim().is_empty() {
        bail!(
            "`{PROGRAM} tasks new` needs the card's title, e.g. `{PROGRAM} tasks new \"fix the races\"`"
        );
    }
    Ok(MoonCommand::NewTask { title })
}

/// `moon tasks move <column>`. The column's name is the whole of the rest, joined, so
/// `IN PROGRESS` needs no quoting - and it is kept as it was typed, for the board to be held
/// to: see [`column_called`].
fn parse_move(args: &[String]) -> Result<MoonCommand> {
    if let Some(option) = args.iter().find(|arg| arg.starts_with('-')) {
        bail!("`{PROGRAM} tasks move` takes the column's name and no options, not {option}");
    }
    let column = args.join(" ");
    if column.trim().is_empty() {
        bail!(
            "`{PROGRAM} tasks move` needs the column to move this shell's task to, e.g. `{PROGRAM} tasks move IN PROGRESS`"
        );
    }
    Ok(MoonCommand::MoveTask { column })
}

/// Make a card on the board of the repo this shell is in and print the folder it was given.
///
/// It joins the top of the board's first column, which is where the board itself puts a card
/// nobody said anything else about: the leftmost column is the one work starts in.
pub(super) fn new_task(title: &str) -> Result<()> {
    let repo_path = repo_of_this_shell()?;
    let board = store::read_board(&repo_path);
    let column = board
        .columns
        .first()
        .context("the board has no columns to put a card in")?
        .id
        .clone();

    let task_id = store::create_task(&repo_path, title, &column, ColumnEnd::Top)?;
    println!("{}", store::task_dir(&repo_path, &task_id)?.display());
    Ok(())
}

/// The repo whose board `new` and `list` are about: the one this shell is in - or, for a
/// person in a work tree of their own, the checkout the board everybody has is in.
fn repo_of_this_shell() -> Result<PathBuf> {
    store::board_checkout(&project_root(
        &env::current_dir().context("failed to read the current directory")?,
    )?)
}

pub(super) fn list_tasks() -> Result<()> {
    print!("{}", list(&repo_of_this_shell()?)?);
    Ok(())
}

/// Every column of the board in a repo by the name the board shows it under, left to right,
/// and under each its cards from the top, a line each: the task's folder, which is what
/// `new` prints and what an agent works from, and the card's title.
///
/// A column's name starts its line and a card's is set in from it, so the two are told
/// apart by `grep`. A column with no cards is still named.
fn list(repo_path: &Path) -> Result<String> {
    let mut columns: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (column, cards) in service::cards_by_column(repo_path)? {
        let mut listed = Vec::new();
        for (task_id, metadata) in cards {
            let folder = store::task_dir(repo_path, &task_id)?;
            listed.push((folder.display().to_string(), metadata.title));
        }
        columns.push((column.label, listed));
    }
    let widest = columns
        .iter()
        .flat_map(|(_, cards)| cards)
        .map(|(folder, _)| folder.chars().count())
        .max()
        .unwrap_or(0);

    let mut text = String::new();
    for (label, cards) in columns {
        text.push_str(&format!("{label}\n"));
        for (folder, title) in cards {
            text.push_str(&format!("  {folder:<widest$}  {title}\n"));
        }
    }
    Ok(text)
}

pub(super) fn move_task(column: &str) -> Result<()> {
    print!("{}", move_card(column, env::var_os(TASK_DIR_ENV_VAR))?);
    Ok(())
}

/// Past the end of any column, which is how the board is asked for a column's bottom. It is
/// where a card moved from the board's own menus lands in a column that names no end for its
/// arrivals - see `arrive_in_column` in `crate::native::board::column` - and a command line
/// has no drop to say otherwise either.
const BOTTOM_OF_THE_COLUMN: usize = usize::MAX;

/// Move the card of the task a shell belongs to into the column called `column`, and say what
/// moved where.
///
/// The move is the board's own - see [`service::place_tasks`] - so the card arrives the way
/// one dragged there does: at the end the column takes its arrivals at, with the time it
/// arrived written down. A card already in that column is left where it is in it.
fn move_card(column: &str, task_dir: Option<OsString>) -> Result<String> {
    let task = TaskOfShell::of(&format!("{PROGRAM} tasks move"), task_dir.as_deref())?;
    let handle = handles::handle_of(&task.task_id, &task.folders_of_its_board()?);
    let board = store::read_board(&task.repo_path);
    let column = column_called(&board.columns, column)?;

    let metadata = store::read_task(&task.repo_path, &task.task_id)?;
    if metadata.status.as_ref() == Some(&column.id) {
        return Ok(format!("@{handle} is already in {}\n", column.label));
    }
    // The shells of this process, which has none: the task's own are held by the moon whose
    // board started them.
    let shells_held_here = TerminalRegistry::new(Arc::new(Mutex::new(Instant::now())));
    service::place_tasks_in_repo(
        &shells_held_here,
        &task.repo_path,
        std::slice::from_ref(&task.task_id),
        column.id.clone(),
        BOTTOM_OF_THE_COLUMN,
    )?;
    Ok(format!("moved @{handle} to {}\n", column.label))
}

/// The column the board shows under this name, letter for letter: `IN PROGRESS`, not
/// `in progress`, and not the `in_progress` its cards are written down as being in. A name
/// the board does not show is refused with the ones it does.
fn column_called<'board>(
    columns: &'board [BoardColumn],
    name: &str,
) -> Result<&'board BoardColumn> {
    let called: Vec<&BoardColumn> = columns
        .iter()
        .filter(|column| column.label == name)
        .collect();
    match called.as_slice() {
        [column] => Ok(column),
        [] => bail!(
            "{name:?} is no column of this board: its columns are {}",
            columns
                .iter()
                .map(|column| column.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!(
            "{} columns of this board are called {name:?}, so the name does not say which",
            called.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moontasks::{
        ColumnId,
        store::{BoardConfig, ColumnSort},
    };

    /// A repo whose board has the three columns named, the last of them sorted by title.
    fn board(name: &str) -> PathBuf {
        let repo = std::env::temp_dir().join(format!(
            "moonreview-tasks-cli-{}-{name}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        std::fs::create_dir_all(&repo).expect("failed to create the test repo");
        let column = |id: &str, label: &str, arrivals, sort| BoardColumn {
            id: ColumnId::new(id),
            label: label.to_string(),
            arrivals,
            sort,
            marks_a_days_work: None,
        };
        let columns = vec![
            column("big", "TODO", None, None),
            column("in_progress", "IN PROGRESS", None, None),
            column("done", "DONE", Some(ColumnEnd::Top), None),
            column("later", "later", None, Some(ColumnSort::Alphabetical)),
        ];
        store::write_board(&repo, &BoardConfig { columns }).expect("expected the board");
        repo
    }

    fn card(repo: &Path, title: &str, column: &str) -> String {
        store::create_task(repo, title, &ColumnId::new(column), ColumnEnd::Bottom)
            .expect("expected the card to be made")
    }

    /// The folder a shell of this task is told it is in.
    fn task_dir(repo: &Path, task_id: &str) -> Option<OsString> {
        Some(
            store::task_dir(repo, task_id)
                .expect("expected the task's folder")
                .into_os_string(),
        )
    }

    #[test]
    fn the_list_is_every_column_by_its_name_with_its_cards_under_it() {
        let repo = board("list");
        let races = card(&repo, "Fix the races", "big");
        let step_10 = card(&repo, "Step 10", "later");
        let step_2 = card(&repo, "Step 2", "later");
        let folder = |task_id: &str| {
            store::task_dir(&repo, task_id)
                .expect("expected the task's folder")
                .display()
                .to_string()
        };
        let widest = folder(&races).len();

        assert_eq!(
            list(&repo).expect("expected the list"),
            format!(
                "TODO\n  {}  Fix the races\nIN PROGRESS\nDONE\nlater\n  {:<widest$}  Step 2\n  {:<widest$}  Step 10\n",
                folder(&races),
                folder(&step_2),
                folder(&step_10),
            )
        );
    }

    /// Moved from a task's shell, a card arrives the way one dragged there does: at the end
    /// the column takes its arrivals at, and dated.
    #[test]
    fn a_card_is_moved_to_the_column_called_what_the_board_shows() {
        let repo = board("move");
        let races = card(&repo, "Fix the races", "big");
        let finished = card(&repo, "Bing bong", "done");
        let mut made_earlier = store::read_task(&repo, &races).expect("expected the task");
        made_earlier.entered_column_at_unix = Some(1);
        store::write_task(&repo, &races, &made_earlier).expect("expected the task written");

        assert_eq!(
            move_card("IN PROGRESS", task_dir(&repo, &races)).expect("expected the move"),
            "moved @fix-the-races to IN PROGRESS\n"
        );
        let moved = store::read_task(&repo, &races).expect("expected the task");
        assert_eq!(moved.status, Some(ColumnId::new("in_progress")));
        assert!(
            moved.entered_column_at_unix > Some(1),
            "the arrival is dated"
        );

        assert_eq!(
            move_card("IN PROGRESS", task_dir(&repo, &races)).expect("expected no refusal"),
            "@fix-the-races is already in IN PROGRESS\n"
        );

        assert_eq!(
            move_card("DONE", task_dir(&repo, &races)).expect("expected the move"),
            "moved @fix-the-races to DONE\n"
        );
        let place = |task_id: &str| {
            let card = store::read_task(&repo, task_id).expect("expected the task");
            (card.status, card.position)
        };
        assert_eq!(place(&races), (Some(ColumnId::new("done")), 0));
        assert_eq!(place(&finished), (Some(ColumnId::new("done")), 1));
    }

    #[test]
    fn a_move_from_outside_a_tasks_shell_or_to_no_column_of_the_board_is_refused() {
        let repo = board("refused");
        let races = card(&repo, "Fix the races", "big");

        let error = move_card("DONE", None).expect_err("expected a refusal");
        assert_eq!(
            error.to_string(),
            "`moon tasks move` is run from a task's shell, and this is not one: \
             MOONREVIEW_TASK_DIR is not set"
        );

        // The name the board shows, not the one it is written down under, and letter for
        // letter.
        for no_column in ["in_progress", "in progress", "Done"] {
            let error =
                move_card(no_column, task_dir(&repo, &races)).expect_err("expected a refusal");
            assert_eq!(
                error.to_string(),
                format!(
                    "{no_column:?} is no column of this board: its columns are TODO, IN \
                     PROGRESS, DONE, later"
                )
            );
        }
        let untouched = store::read_task(&repo, &races).expect("expected the task");
        assert_eq!(untouched.status, Some(ColumnId::new("big")));
    }
}
