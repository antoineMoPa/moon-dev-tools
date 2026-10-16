//! Placing dropped cards - where a drop lands in a column, and the move made on the board
//! being drawn ahead of the server's answer.

use crate::{
    moontasks::{ColumnId, TaskView, column_sort, sub_tasks},
    native::{
        board::{cards::sort_of, filter::Filter},
        model::Model,
    },
};

/// Where a card let go of among the cards a filter is showing belongs in the column itself.
///
/// A drop is read against what is on screen: let go of above the third card showing means
/// above that card, whatever the filter is hiding between it and the one before. Let go of
/// below the last card showing means the end of the column, the same as it does with no filter
/// on - and with no filter on the two indexes are the same, so nothing is translated at all.
///
/// The cards being dragged are left out of the reckoning, because [`place_in`] takes them out
/// of the column before it counts places in it.
pub(super) fn column_index_of(
    tasks: &[TaskView],
    filter: &Filter,
    status: &ColumnId,
    dragged_ids: &[String],
    showing_index: usize,
) -> usize {
    if !filter.is_on() {
        return showing_index;
    }

    let column: Vec<&TaskView> = tasks
        .iter()
        .filter(|task| task.status == *status && !dragged_ids.contains(&task.id))
        .collect();

    let mut showing = 0;
    for (at, task) in column.iter().enumerate() {
        if !filter.matches(task) {
            continue;
        }
        if showing == showing_index {
            return at;
        }
        showing += 1;
    }
    column.len()
}

/// Take a board the server has answered with, with a drop that it may not have seen yet.
///
/// A read that was already on its way when a card was dropped answers with the card where it
/// was, so the drop is made again on top of it - until an answer comes back with the card
/// where it was put, which is the server having caught up.
pub(crate) fn accept_board(model: &mut Model, mut tasks: Vec<TaskView>) {
    close_the_pages_of_deleted_tasks(model, &tasks);
    let Some(pending) = &model.board.pending_place else {
        model.board.tasks = tasks;
        return;
    };
    let column: Vec<&TaskView> = tasks
        .iter()
        .filter(|task| task.status == pending.status)
        .collect();
    let landed = match sort_of(&model.board.columns, &pending.status) {
        // A sorted column puts the cards where its order says rather than where they were
        // dropped, so they have landed once they are in it at all.
        Some(_) => pending
            .task_ids
            .iter()
            .all(|task_id| column.iter().any(|task| task.id == *task_id)),
        // Landed once the run of dropped cards is where it was dropped and in the order it
        // was dropped in: the last of them can be no further down than the end of the column.
        None => {
            let first = pending
                .index
                .min(column.len().saturating_sub(pending.task_ids.len()));
            pending
                .task_ids
                .iter()
                .enumerate()
                .all(|(offset, task_id)| {
                    column
                        .get(first + offset)
                        .is_some_and(|task| task.id == *task_id)
                })
        }
    };
    // And under the card each was put under.
    let landed = landed
        && pending
            .task_ids
            .iter()
            .zip(&pending.parents)
            .all(|(task_id, parent)| {
                column
                    .iter()
                    .any(|task| task.id == *task_id && task.parent == *parent)
            });

    if landed {
        model.board.pending_place = None;
    } else {
        let (task_ids, status, index, parent) = (
            pending.task_ids.clone(),
            pending.status.clone(),
            pending.index,
            pending.parent.clone(),
        );
        place_in(&mut tasks, &task_ids, &status, index, parent.as_deref());
        column_sort::arrange(&model.board.columns, &mut tasks);
        sub_tasks::under_their_parents(&mut tasks);
    }
    model.board.tasks = tasks;
}

/// A task the board had and no longer has has been deleted - here, or in `.moontasks` by hand -
/// and its page goes with it: a tab standing there saying the task is gone is a tab you have to
/// close yourself.
///
/// Read against the answer before it rather than against the tab: a task created a moment ago
/// is in no answer yet, and a page opened on it must not be closed for the read that was
/// already on its way when it was made.
fn close_the_pages_of_deleted_tasks(model: &mut Model, tasks: &[TaskView]) {
    let deleted: Vec<String> = model
        .board
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .filter(|task_id| !tasks.iter().any(|task| task.id == *task_id))
        .collect();
    // Queued rather than closed here, the way a card let go of queues its page: the answer is
    // taken while the window is being drawn, and a pane is never closed while the tree that
    // holds it is being drawn.
    model.board.pages_to_close.extend(deleted);
}

/// Make the move on the board being drawn, ahead of the server being told about it.
///
/// What the board draws is the last answer the server gave, and the next one is a worker
/// thread and a poll away. Without this the dropped card springs back to where it came from
/// for those few frames and then moves again - which reads as the drop having failed.
///
/// `task_ids` holds the sub-tasks of the cards being moved as well - see
/// [`sub_tasks::with_sub_tasks`] - and `parent` is the card they were dropped onto. Answers
/// with the card each of them is under once it has landed.
pub(super) fn place_in(
    tasks: &mut Vec<TaskView>,
    task_ids: &[String],
    status: &ColumnId,
    index: usize,
    parent: Option<&str>,
) -> Vec<Option<String>> {
    // Taken out in the order the board had them, which is the order they go back in: a drag
    // moves a run of cards without reordering it.
    let mut moving: Vec<TaskView> = Vec::new();
    tasks.retain(|task| {
        if !task_ids.contains(&task.id) {
            return true;
        }
        moving.push(TaskView {
            status: status.clone(),
            parent: sub_tasks::parent_after_move(task.parent.as_deref(), parent, task_ids),
            ..task.clone()
        });
        false
    });
    let parents = task_ids
        .iter()
        .map(|task_id| {
            moving
                .iter()
                .find(|task| task.id == *task_id)
                .and_then(|task| task.parent.clone())
        })
        .collect();

    // Where that place is in the one list the board keeps every column's cards in.
    let column: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.status == *status)
        .map(|(at, _)| at)
        .collect();
    let into = match column.get(index) {
        Some(&at) => at,
        None => column.last().map_or(tasks.len(), |&at| at + 1),
    };
    for (offset, moved) in moving.into_iter().enumerate() {
        tasks.insert(into + offset, moved);
    }
    sub_tasks::under_their_parents(tasks);
    parents
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::board::cards::column_size;

    /// The one card an ordinary drag carries, as the run every drag is read as.
    fn dragging(task_id: &str) -> Vec<String> {
        vec![task_id.to_string()]
    }

    /// A column of cards, each named after what a query would find it by.
    fn column(titles: &[&str]) -> Vec<TaskView> {
        titles
            .iter()
            .map(|title| TaskView {
                id: format!("{title}-1111"),
                title: title.to_string(),
                status: ColumnId::new("todo"),
                created_at_unix: 1700000000,
                entered_column_at_unix: None,
                dir_path: String::new(),
                repo_path: String::new(),
                tags: Vec::new(),
                remote_task_tracker_url: String::new(),
                parent: None,
                notes: String::new(),
                attachments: Vec::new(),
                resources: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn with_no_filter_on_a_drop_lands_where_it_was_let_go_of() {
        let tasks = column(&["one", "two", "three"]);
        let status = ColumnId::new("todo");
        let nothing = Filter::of("");

        for at in 0..3 {
            assert_eq!(
                column_index_of(&tasks, &nothing, &status, &dragging("two-1111"), at),
                at
            );
        }
    }

    /// The cards a filter hides are still in the column, and a drop is read against the ones
    /// on screen: above the card showing that was dropped above.
    #[test]
    fn a_drop_among_filtered_cards_lands_above_the_card_it_was_dropped_above() {
        // Showing: `sing` at column place 1, and `song` at column place 3.
        let tasks = column(&["hidden", "sing", "also hidden", "song"]);
        let status = ColumnId::new("todo");
        let filter = Filter::of("s\u{69}ng");
        let dragged = "song-1111";

        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging(dragged), 0),
            1,
            "above the first card showing is above that card, not the top of the column"
        );
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging(dragged), 1),
            3,
            "below the last card showing is the end of the column, hidden cards and all"
        );
    }

    /// The card being dragged is not one of the places it can be dropped into: `place_in`
    /// takes it out of the column before it counts places in it.
    #[test]
    fn the_dragged_card_is_left_out_of_the_places_it_could_land_in() {
        let tasks = column(&["sing", "hidden", "song"]);
        let status = ColumnId::new("todo");
        let filter = Filter::of("s");

        // With `sing` in the air, the only card showing is `song`, at place 1 of the two the
        // column has left - so the first place a drop can take is that one, not `sing`'s.
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging("sing-1111"), 0),
            1
        );
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging("sing-1111"), 1),
            2,
            "and past it is the end of what the column has left"
        );
    }

    /// A drag made with several cards marked puts them down as one run, in the order the board
    /// had them - whichever of them the drop names first.
    #[test]
    fn cards_dragged_together_land_as_a_run_in_the_order_the_board_had_them() {
        let mut tasks = column(&["one", "two", "three", "four"]);
        let done = ColumnId::new("done");

        place_in(
            &mut tasks,
            &["three-1111".to_string(), "one-1111".to_string()],
            &done,
            0,
            None,
        );

        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["two", "four", "one", "three"]);
        assert_eq!(tasks[2].status, done);
        assert_eq!(tasks[3].status, done);
        assert_eq!(tasks[0].status, ColumnId::new("todo"));
    }

    #[test]
    fn cards_dragged_into_a_column_go_in_at_the_place_they_were_dropped() {
        let mut tasks = column(&["one", "two", "three", "four"]);

        place_in(
            &mut tasks,
            &["one-1111".to_string(), "two-1111".to_string()],
            &ColumnId::new("todo"),
            1,
            None,
        );

        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["three", "one", "two", "four"]);
    }

    /// A card dropped onto another goes under it, after the sub-tasks it already has, and a
    /// card carried off with its sub-tasks keeps them.
    #[test]
    fn cards_dropped_onto_a_card_go_under_it() {
        let mut tasks = column(&["parent", "child", "other", "loose"]);
        tasks[1].parent = Some("parent-1111".to_string());
        let todo = ColumnId::new("todo");

        let parents = place_in(
            &mut tasks,
            &["loose-1111".to_string()],
            &todo,
            3,
            Some("parent-1111"),
        );
        assert_eq!(parents, [Some("parent-1111".to_string())]);
        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["parent", "child", "loose", "other"]);

        let carried = sub_tasks::with_sub_tasks(&tasks, &["parent-1111".to_string()]);
        let parents = place_in(&mut tasks, &carried, &ColumnId::new("done"), 0, None);
        assert_eq!(
            parents,
            [
                None,
                Some("parent-1111".to_string()),
                Some("parent-1111".to_string())
            ]
        );
        assert!(
            tasks
                .iter()
                .filter(|task| task.title != "other")
                .all(|task| task.status == ColumnId::new("done"))
        );
    }

    #[test]
    fn a_columns_size_counts_the_cards_a_filter_is_hiding_too() {
        let mut tasks = column(&["one", "two"]);
        tasks.push(TaskView {
            status: ColumnId::new("done"),
            ..column(&["three"]).remove(0)
        });

        assert_eq!(column_size(&tasks, &ColumnId::new("todo")), 2);
        assert_eq!(column_size(&tasks, &ColumnId::new("done")), 1);
    }
}
