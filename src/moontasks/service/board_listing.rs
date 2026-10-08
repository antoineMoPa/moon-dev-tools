//! Board listing - the cards of a board, column by column in the order its window draws them,
//! read from the board's folder and nothing written to it.
//!
//! [`super::list_tasks`] is the window's reading, and it catches the board up as it reads: a
//! run whose shell is gone is written down as ended, and a card carried in from another board
//! is written into a place. This is the reading of a caller that only looks - `moon tasks
//! list` - so it works out the same places and writes none of them.

use std::path::Path;

use anyhow::Result;

use crate::moontasks::{
    column_sort,
    store::{self, BoardColumn, TaskMetadata},
};

use super::{place_of, shared_places, stray_cards};

/// Every column of the board in a repo, left to right, each with its cards from the top: the
/// name of a card's folder, and its `metadata.json`.
pub(crate) fn cards_by_column(
    repo_path: &Path,
) -> Result<Vec<(BoardColumn, Vec<(String, TaskMetadata)>)>> {
    let mut read: Vec<(String, TaskMetadata)> = Vec::new();
    for task_id in store::list_task_ids(repo_path)? {
        // Skipped for the reasons `list_tasks` skips them: a `metadata.json` that cannot be
        // read is no card to show, and the board task is on no column.
        let Ok(metadata) = store::read_task(repo_path, &task_id) else {
            continue;
        };
        if metadata.status.is_none() {
            continue;
        }
        read.push((task_id, metadata));
    }

    // Where the window will draw them the next time it reads the board, which is when it
    // writes these places down.
    let board = store::read_board(repo_path);
    stray_cards::gather_strays(&board, &mut read);
    shared_places::renumber_tied_columns(&mut read);
    read.sort_by_key(|(_, metadata)| place_of(metadata));

    Ok(board
        .columns
        .into_iter()
        .map(|column| {
            let mut cards: Vec<(String, TaskMetadata)> = read
                .iter()
                .filter(|(_, metadata)| *metadata.column() == column.id)
                .cloned()
                .collect();
            if let Some(sort) = column.sort {
                column_sort::sort_cards_read_as(sort, &mut cards, |(_, metadata)| {
                    (&metadata.title, metadata.created_at_unix)
                });
            }
            (column, cards)
        })
        .collect())
}
