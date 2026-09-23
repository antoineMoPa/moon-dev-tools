//! The board's columns: added, renamed, sorted, moved and deleted, and the cards a deleted one
//! leaves behind.

use anyhow::{Result, bail};

use crate::{
    api::AppState,
    moontasks::store::{self, BoardColumn, ColumnEnd, ColumnId, ColumnSort},
};

use super::repo_of;

/// The board's columns, left to right.
pub(crate) fn list_columns(state: &AppState, session_id: &str) -> Result<Vec<BoardColumn>> {
    let repo_path = repo_of(state, session_id)?;
    Ok(store::read_board(&repo_path).columns)
}

/// Add a column, `at` columns from the left - or at the right-hand end when nothing says.
///
/// Its id is made from its name the same way a task folder's is, so the board file stays
/// readable - and made unique, because two columns sharing an id would be one column with two
/// headings and every card in either would be in both.
pub(crate) fn add_column(
    state: &AppState,
    session_id: &str,
    label: &str,
    at: Option<usize>,
) -> Result<BoardColumn> {
    let label = label.trim();
    if label.is_empty() {
        bail!("a column needs a name");
    }

    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);

    let base = store::slug_of(label);
    let mut id = ColumnId::new(base.clone());
    for suffix in 2.. {
        if !board.has(&id) {
            break;
        }
        id = ColumnId::new(format!("{base}-{suffix}"));
    }

    let column = BoardColumn {
        id,
        label: label.to_string(),
        arrivals: None,
        sort: None,
    };
    let at = at.unwrap_or(board.columns.len()).min(board.columns.len());
    board.columns.insert(at, column.clone());
    store::write_board(&repo_path, &board)?;
    Ok(column)
}

/// Rename a column. The id is left alone, so every card in it stays in it.
pub(crate) fn rename_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    label: &str,
) -> Result<()> {
    let label = label.trim();
    if label.is_empty() {
        bail!("a column needs a name");
    }

    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    let Some(column) = board
        .columns
        .iter_mut()
        .find(|column| column.id == *column_id)
    else {
        bail!("{column_id} is not a column of this board");
    };
    column.label = label.to_string();
    store::write_board(&repo_path, &board)
}

/// Say which end of a column cards moved in from elsewhere go to, or let the drop decide
/// again - see [`store::BoardColumn::arrivals`].
pub(crate) fn set_column_arrivals(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    arrivals: Option<ColumnEnd>,
) -> Result<()> {
    change_column(state, session_id, column_id, |column| {
        column.arrivals = arrivals
    })
}

/// Which order a column keeps its cards in by itself - see [`column_sort`]. `None` is the
/// order they are dragged into.
pub(crate) fn set_column_sort(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    sort: Option<ColumnSort>,
) -> Result<()> {
    change_column(state, session_id, column_id, |column| column.sort = sort)
}

/// Change one of a column's settings in the board's file.
fn change_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    change: impl FnOnce(&mut BoardColumn),
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    let Some(column) = board
        .columns
        .iter_mut()
        .find(|column| column.id == *column_id)
    else {
        bail!("{column_id} is not a column of this board");
    };
    change(column);
    store::write_board(&repo_path, &board)
}

/// Take a column off the board.
///
/// Only an empty one: a column holding cards is the only record of where those cards are, and
/// deleting it would either lose them or move them somewhere nobody asked for. The board says
/// as much rather than choosing on the user's behalf.
pub(crate) fn delete_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    if !board.has(column_id) {
        bail!("{column_id} is not a column of this board");
    }
    if board.columns.len() == 1 {
        bail!("a board needs a column to put its cards in");
    }

    let holding = store::list_task_ids(&repo_path)?
        .iter()
        .filter_map(|task_id| store::read_task(&repo_path, task_id).ok())
        .filter(|metadata| metadata.status == *column_id)
        .count();
    if holding > 0 {
        bail!(
            "there {} still {holding} card{} in this column - move {} out first",
            if holding == 1 { "is" } else { "are" },
            if holding == 1 { "" } else { "s" },
            if holding == 1 { "it" } else { "them" }
        );
    }

    board.columns.retain(|column| column.id != *column_id);
    store::write_board(&repo_path, &board)
}

/// Put a column at a place among the others, which is what dragging its heading does.
///
/// The cards go with it: a card names its column, so where the column is drawn is where its
/// cards are drawn, and nothing about any of them has to be rewritten.
pub(crate) fn place_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    position: usize,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    let Some(at) = board.position_of(column_id) else {
        bail!("{column_id} is not a column of this board");
    };

    let column = board.columns.remove(at);
    board
        .columns
        .insert(position.min(board.columns.len()), column);
    store::write_board(&repo_path, &board)
}
