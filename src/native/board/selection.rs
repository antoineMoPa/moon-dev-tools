//! The cards the board has marked, and what a click does to them.
//!
//! There is one mark and one place it is kept. A click on a card marks that card and opens its
//! page; cmd+click puts one card in or out; shift+click takes the run between the anchor and
//! the card clicked; and a task's own tab coming to the front marks its card, because that is
//! the same thing said another way. One card marked is a task to read, and several are a group
//! to drag.

use egui::Modifiers;

use std::collections::HashSet;

use crate::native::{
    board::filter::Filter,
    model::{BoardState, Carrying},
};

pub(crate) fn is_marked(board: &BoardState, task_id: &str) -> bool {
    board.marked.contains(task_id)
}

pub(crate) fn clear(board: &mut BoardState) {
    board.marked.clear();
    board.mark_anchor = None;
}

/// Let every mark go, by a click or a key rather than by a tab coming forward - so the pages
/// of the cards let go of are put away with them.
pub(crate) fn let_go_of_all(board: &mut BoardState) {
    let was_marked = board.marked.clone();
    clear(board);
    pages_of_cards_let_go(board, was_marked);
}

/// Remember the cards that were marked and no longer are, so their pages can be put away once
/// the window has drawn.
///
/// Only the board's own clicks come through here. A mark that moves because a task's tab came
/// to the front leaves every page where it is: you are reading that task, not putting it away.
fn pages_of_cards_let_go(board: &mut BoardState, was_marked: HashSet<String>) {
    let let_go = was_marked
        .into_iter()
        .filter(|task_id| !board.marked.contains(task_id));
    board.pages_to_close.extend(let_go);
}

/// Mark this card and nothing else, which is what a plain click and a task's tab both do.
pub(crate) fn mark_only(board: &mut BoardState, task_id: String) {
    board.marked.clear();
    board.marked.insert(task_id.clone());
    board.mark_anchor = Some(task_id);
}

/// What a click on the board did to the marks, and the task whose page it asks for.
///
/// `on` is the card clicked, or `None` for the board beside the cards - which is where a
/// selection is let go of.
pub(crate) fn clicked(
    board: &mut BoardState,
    on: Option<&str>,
    modifiers: Modifiers,
) -> Option<String> {
    let Some(task_id) = on else {
        let_go_of_all(board);
        return None;
    };
    let was_marked = board.marked.clone();
    let opens = marked_by_the_click(board, task_id, modifiers);
    pages_of_cards_let_go(board, was_marked);
    opens
}

/// What the click marks, which is the whole of the difference between the two keys.
fn marked_by_the_click(
    board: &mut BoardState,
    task_id: &str,
    modifiers: Modifiers,
) -> Option<String> {
    if modifiers.command {
        toggle(board, task_id);
    } else if modifiers.shift {
        extend_to(board, task_id);
    } else {
        mark_only(board, task_id.to_string());
        return Some(task_id.to_string());
    }
    None
}

/// Put a card in the marks or take it back out - the one-card key, whatever else is marked.
fn toggle(board: &mut BoardState, task_id: &str) {
    if !board.marked.remove(task_id) {
        board.marked.insert(task_id.to_string());
    }
    board.mark_anchor = Some(task_id.to_string());
}

/// Mark the run between the anchor and this card - the run key.
///
/// Read down the column the two are in, among the cards the filter is showing, so what is
/// taken is what the eye sees between them. A card with no anchor, or one in another column,
/// starts a run of its own rather than ending one measured from somewhere out of sight.
fn extend_to(board: &mut BoardState, task_id: &str) {
    let showing = column_showing(board, task_id);
    let run = run_between(&showing, board.mark_anchor.as_deref(), task_id);

    board.marked = run.iter().cloned().collect();
    // The anchor stays where it was, so a second shift+click stretches the same run rather
    // than measuring from the end of the last one.
    if board.mark_anchor.is_none() {
        board.mark_anchor = Some(task_id.to_string());
    }
}

/// The cards of a column a shift+click takes: those between the anchor and the card clicked,
/// both ends in.
fn run_between<'a>(showing: &'a [String], anchor: Option<&str>, task_id: &str) -> &'a [String] {
    let Some(clicked) = showing.iter().position(|id| id == task_id) else {
        return &[];
    };
    let from = anchor
        .and_then(|anchor| showing.iter().position(|id| id == anchor))
        .unwrap_or(clicked);
    &showing[from.min(clicked)..=from.max(clicked)]
}

/// The cards of this card's column that the filter is showing, top to bottom.
fn column_showing(board: &BoardState, task_id: &str) -> Vec<String> {
    let Some(status) = board
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .map(|task| task.status.clone())
    else {
        return Vec::new();
    };
    let filter = Filter::of(&board.filter);
    board
        .tasks
        .iter()
        .filter(|task| task.status == status && filter.matches(task))
        .map(|task| task.id.clone())
        .collect()
}

/// What a drag begun on this card carries.
///
/// A marked card brings every marked card with it, in the order the board holds them. One that
/// is not marked is carried alone and becomes the mark - the way dragging one icon out of a
/// selected group does in a file manager - unless cmd or shift is held, which is a card joining
/// the group rather than replacing it.
///
/// Whatever is carried ends up marked, which is what shows where a run of cards landed once it
/// is let go of.
pub(crate) fn carried_by(board: &mut BoardState, primary: &str, modifiers: Modifiers) -> Carrying {
    let was_marked = board.marked.clone();
    if modifiers.command || modifiers.shift {
        board.marked.insert(primary.to_string());
        board.mark_anchor = Some(primary.to_string());
    } else if !is_marked(board, primary) {
        mark_only(board, primary.to_string());
    }
    pages_of_cards_let_go(board, was_marked);

    Carrying {
        primary: primary.to_string(),
        task_ids: board
            .tasks
            .iter()
            .filter(|task| is_marked(board, &task.id))
            .map(|task| task.id.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moontasks::{ColumnId, TaskView};

    fn board(titles: &[&str]) -> BoardState {
        BoardState {
            tasks: titles
                .iter()
                .map(|title| TaskView {
                    id: format!("{title}-1111"),
                    title: title.to_string(),
                    status: ColumnId::new("todo"),
                    created_at_unix: 1700000000,
                    dir_path: String::new(),
                    repo_path: String::new(),
                    notes: String::new(),
                    resources: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    fn marks(board: &BoardState) -> Vec<String> {
        let mut marked: Vec<String> = board.marked.iter().cloned().collect();
        marked.sort();
        marked
    }

    #[test]
    fn a_plain_click_marks_that_card_alone_and_asks_for_its_page() {
        let mut board = board(&["one", "two"]);
        assert_eq!(
            clicked(&mut board, Some("one-1111"), Modifiers::NONE),
            Some("one-1111".to_string())
        );
        assert_eq!(marks(&board), ["one-1111"]);

        clicked(&mut board, Some("two-1111"), Modifiers::NONE);
        assert_eq!(marks(&board), ["two-1111"], "and lets the last one go");
    }

    #[test]
    fn cmd_puts_one_card_in_and_takes_the_same_card_out() {
        let mut board = board(&["one", "two", "three"]);
        for id in ["one-1111", "two-1111"] {
            assert_eq!(clicked(&mut board, Some(id), Modifiers::COMMAND), None);
        }
        assert_eq!(marks(&board), ["one-1111", "two-1111"]);

        clicked(&mut board, Some("one-1111"), Modifiers::COMMAND);
        assert_eq!(
            marks(&board),
            ["two-1111"],
            "cmd+clicking a marked card takes it back out"
        );
    }

    /// The two keys do two things, and the one-card key undoes what the run key did to one
    /// card - which is how a card in the middle of a run is let go of.
    #[test]
    fn shift_takes_the_run_and_cmd_takes_one_card_out_of_it() {
        let mut board = board(&["one", "two", "three"]);
        clicked(&mut board, Some("one-1111"), Modifiers::COMMAND);
        clicked(&mut board, Some("three-1111"), Modifiers::SHIFT);
        assert_eq!(marks(&board), ["one-1111", "three-1111", "two-1111"]);

        clicked(&mut board, Some("two-1111"), Modifiers::COMMAND);
        assert_eq!(marks(&board), ["one-1111", "three-1111"]);
    }

    #[test]
    fn a_second_shift_click_stretches_the_run_from_the_same_anchor() {
        let mut board = board(&["one", "two", "three"]);
        clicked(&mut board, Some("two-1111"), Modifiers::COMMAND);
        clicked(&mut board, Some("three-1111"), Modifiers::SHIFT);
        clicked(&mut board, Some("one-1111"), Modifiers::SHIFT);
        assert_eq!(
            marks(&board),
            ["one-1111", "two-1111"],
            "measured from the anchor, not from the end of the last run"
        );
    }

    #[test]
    fn a_click_beside_the_cards_lets_the_marks_go() {
        let mut board = board(&["one", "two"]);
        clicked(&mut board, Some("one-1111"), Modifiers::COMMAND);
        assert_eq!(clicked(&mut board, None, Modifiers::NONE), None);
        assert!(marks(&board).is_empty());
    }

    #[test]
    fn a_marked_card_carries_the_marks_and_an_unmarked_one_carries_itself() {
        let mut board = board(&["one", "two", "three"]);
        clicked(&mut board, Some("one-1111"), Modifiers::COMMAND);
        clicked(&mut board, Some("two-1111"), Modifiers::COMMAND);

        let carrying = carried_by(&mut board, "two-1111", Modifiers::NONE);
        assert_eq!(carrying.task_ids, ["one-1111", "two-1111"]);
        assert_eq!(carrying.primary, "two-1111");

        let carrying = carried_by(&mut board, "three-1111", Modifiers::NONE);
        assert_eq!(carrying.task_ids, ["three-1111"]);
        assert_eq!(
            marks(&board),
            ["three-1111"],
            "and it is the mark now, which is what shows where it lands"
        );
    }

    /// The keys that gather cards are held down over the drag that takes them somewhere, so a
    /// card picked up with one held joins the group rather than replacing it - even one that
    /// was never marked.
    #[test]
    fn a_card_picked_up_with_cmd_held_joins_the_marks() {
        let mut board = board(&["one", "two", "three"]);
        clicked(&mut board, Some("one-1111"), Modifiers::COMMAND);

        let carrying = carried_by(&mut board, "three-1111", Modifiers::COMMAND);
        assert_eq!(carrying.task_ids, ["one-1111", "three-1111"]);
    }
}
