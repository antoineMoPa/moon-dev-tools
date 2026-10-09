//! Cards that name a column the board does not have.
//!
//! A task folder moved by hand from one project's board to another's still says which column
//! it was in on the old board - `todo` where this board calls it `big` - and a card in a
//! column nobody draws is on no column at all: it is on disk and nowhere on the board.

use crate::moontasks::store::{BoardConfig, TaskMetadata};

use super::place_of;

/// Move every card whose column is not on the board to the top of the board's first column,
/// pushing the cards already there down. Returns the ids of the cards whose record changed,
/// which are the ones to write back.
pub(super) fn gather_strays(
    board: &BoardConfig,
    tasks: &mut [(String, TaskMetadata)],
) -> Vec<String> {
    let first = board
        .columns
        .first()
        .expect("a board always has a column")
        .id
        .clone();

    let mut strays: Vec<usize> = (0..tasks.len())
        .filter(|index| !board.has(tasks[*index].1.column()))
        .collect();
    if strays.is_empty() {
        return Vec::new();
    }
    // Among themselves they keep the order they were drawn in on the board they came from.
    strays.sort_by(|a, b| {
        place_of(&tasks[*a].1)
            .cmp(&place_of(&tasks[*b].1))
            .then_with(|| tasks[*a].0.cmp(&tasks[*b].0))
    });

    let mut changed = Vec::new();
    // Making room keeps the order the column's own cards were in among themselves.
    for (id, metadata) in tasks.iter_mut() {
        if *metadata.column() == first {
            metadata.position += strays.len() as u32;
            changed.push(id.clone());
        }
    }
    for (position, index) in strays.into_iter().enumerate() {
        let (id, metadata) = &mut tasks[index];
        metadata.status = Some(first.clone());
        metadata.position = position as u32;
        changed.push(id.clone());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moontasks::store::ColumnId;

    fn card(title: &str, status: &str, position: u32) -> (String, TaskMetadata) {
        let metadata = TaskMetadata {
            title: title.to_string(),
            status: Some(ColumnId::new(status)),
            created_at_unix: 0,
            entered_column_at_unix: None,
            position,
            tags: Vec::new(),
            remote_task_tracker_url: String::new(),
            resources: Vec::new(),
            made_by: None,
        };
        (title.to_string(), metadata)
    }

    #[test]
    fn a_card_from_another_boards_column_lands_above_the_first_columns_own_cards() {
        let board = BoardConfig::default();
        let first = board.columns[0].id.clone();
        let mut tasks = vec![
            (
                "here".to_string(),
                TaskMetadata {
                    status: Some(first.clone()),
                    ..card("here", "", 3).1
                },
            ),
            card("stray", "not-a-column-of-this-board", 0),
        ];

        let moved = gather_strays(&board, &mut tasks);

        assert_eq!(moved, ["here", "stray"]);
        assert_eq!(tasks[1].1.status, Some(first));
        assert_eq!(tasks[1].1.position, 0);
        assert_eq!(tasks[0].1.position, 4);
    }
}
