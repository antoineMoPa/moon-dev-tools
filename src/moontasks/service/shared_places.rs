//! Cards that sit at the same place in a column.
//!
//! Every card carries its own `position`, and dragging renumbers the column it lands in from
//! zero, so two cards only share one when something wrote a card in from outside: a task
//! folder moved by hand from one project's board to another's keeps the number it had there.
//! The column then draws its tied cards by creation time, which works until one of them is
//! dragged and the others jump.
//!
//! Gaps are left alone - only a tie is a disagreement - and so is a board that has no
//! positions at all, which reads as a column of zeroes and is repaired the same way.

use std::collections::HashMap;

use crate::moontasks::store::{ColumnId, TaskMetadata};

use super::place_of;

/// Renumber, from zero, every column in which two cards share a position, keeping the order
/// the column was already drawn in. Returns the ids of the cards whose number changed, which
/// are the ones to write back.
pub(super) fn renumber_tied_columns(tasks: &mut [(String, TaskMetadata)]) -> Vec<String> {
    let mut columns: HashMap<ColumnId, Vec<usize>> = HashMap::new();
    for (index, (_, metadata)) in tasks.iter().enumerate() {
        columns
            .entry(metadata.column().clone())
            .or_default()
            .push(index);
    }

    let mut changed = Vec::new();
    for mut indices in columns.into_values() {
        indices.sort_by(|a, b| {
            // The id settles what creation time cannot, so a repair never depends on the
            // order the folders were listed in.
            place_of(&tasks[*a].1)
                .cmp(&place_of(&tasks[*b].1))
                .then_with(|| tasks[*a].0.cmp(&tasks[*b].0))
        });
        let tied = indices
            .windows(2)
            .any(|pair| tasks[pair[0]].1.position == tasks[pair[1]].1.position);
        if !tied {
            continue;
        }
        for (place, index) in indices.into_iter().enumerate() {
            let (id, metadata) = &mut tasks[index];
            if metadata.position != place as u32 {
                metadata.position = place as u32;
                changed.push(id.clone());
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(
        title: &str,
        status: &str,
        position: u32,
        created_at_unix: u64,
    ) -> (String, TaskMetadata) {
        let metadata = TaskMetadata {
            title: title.to_string(),
            status: Some(ColumnId::new(status)),
            created_at_unix,
            entered_column_at_unix: None,
            position,
            tags: Vec::new(),
            remote_task_tracker_url: String::new(),
            resources: Vec::new(),
        };
        (title.to_string(), metadata)
    }

    fn positions(tasks: &[(String, TaskMetadata)]) -> Vec<(&str, u32)> {
        tasks
            .iter()
            .map(|(id, metadata)| (id.as_str(), metadata.position))
            .collect()
    }

    #[test]
    fn a_card_carried_in_with_a_taken_position_is_renumbered_in_drawn_order() {
        let mut tasks = vec![
            card("a", "todo", 0, 10),
            card("b", "todo", 1, 20),
            card("carried", "todo", 1, 5),
            card("elsewhere", "done", 7, 1),
        ];

        let changed = renumber_tied_columns(&mut tasks);

        // The tie drew by creation time, so `carried` (older than `b`) stays above it.
        assert_eq!(
            positions(&tasks),
            [("a", 0), ("b", 2), ("carried", 1), ("elsewhere", 7)]
        );
        assert_eq!(changed, ["b"]);
    }

    #[test]
    fn a_column_with_gaps_but_no_ties_is_left_alone() {
        let mut tasks = vec![card("a", "todo", 0, 1), card("b", "todo", 5, 2)];

        assert!(renumber_tied_columns(&mut tasks).is_empty());
        assert_eq!(positions(&tasks), [("a", 0), ("b", 5)]);
    }
}
