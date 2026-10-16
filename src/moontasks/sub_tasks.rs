//! Sub-tasks - a card dropped onto another card is that card's sub-task, drawn under it.
//!
//! A sub-task is a link in its card's `subtasks` folder - see [`super::store::parents_by_link`]
//! - and goes one deep: a card that is itself under another takes no sub-tasks of its own. A
//! sub-task is drawn under its card while the two are in the same column, and moves with it to
//! another one - see [`with_sub_tasks`]. One that ends up in a column without its card is an
//! ordinary card there.
//!
//! Both ends read it: the server lists and renumbers each column in this order, and the board
//! puts dropped cards in it while the server's answer is on its way. One function for both,
//! so the two never disagree about where a card is.

use std::collections::HashMap;

use crate::moontasks::{ColumnId, TaskView};

/// What the order is worked out from, for cards held as the board draws them and as the
/// server reads them off disk.
pub(crate) trait Card {
    fn id(&self) -> &str;
    fn parent(&self) -> Option<&str>;
    fn column(&self) -> &ColumnId;
}

impl Card for TaskView {
    fn id(&self) -> &str {
        &self.id
    }
    fn parent(&self) -> Option<&str> {
        self.parent.as_deref()
    }
    fn column(&self) -> &ColumnId {
        &self.status
    }
}

/// The card `card` is drawn under, by its place in `cards`: its parent, when the parent is in
/// the same column and is under no card itself.
fn drawn_under<C: Card>(at: &HashMap<&str, usize>, cards: &[C], card: &C) -> Option<usize> {
    let parent = at.get(card.parent()?).copied()?;
    let parent_card = &cards[parent];
    (parent_card.column() == card.column() && parent_card.parent().is_none()).then_some(parent)
}

fn places_of<C: Card>(cards: &[C]) -> HashMap<&str, usize> {
    cards
        .iter()
        .enumerate()
        .map(|(at, card)| (card.id(), at))
        .collect()
}

/// Put each sub-task right after its card, the sub-tasks of one card in the order they were
/// in. Every other card keeps its order, so a list of several columns' cards stays one.
pub(crate) fn under_their_parents<C: Card>(cards: &mut Vec<C>) {
    let at = places_of(cards);
    let under: Vec<Option<usize>> = cards
        .iter()
        .map(|card| drawn_under(&at, cards, card))
        .collect();
    let mut order = Vec::with_capacity(cards.len());
    for (parent, _) in under.iter().enumerate().filter(|(_, up)| up.is_none()) {
        order.push(parent);
        order.extend(
            under
                .iter()
                .enumerate()
                .filter(|(_, up)| **up == Some(parent))
                .map(|(child, _)| child),
        );
    }
    let mut taken: Vec<Option<C>> = std::mem::take(cards).into_iter().map(Some).collect();
    cards.extend(
        order
            .into_iter()
            .map(|at| taken[at].take().expect("each card is in the order once")),
    );
}

/// The card each of `cards` is drawn under - indented, as its sub-task - by its id, or `None`
/// for one drawn as a card of its own.
pub(crate) fn drawn_under_ids<'cards, C: Card>(cards: &'cards [C]) -> Vec<Option<&'cards str>> {
    let at = places_of(cards);
    cards
        .iter()
        .map(|card| drawn_under(&at, cards, card).map(|parent| cards[parent].id()))
        .collect()
}

/// The ids of `task_ids` and of the sub-tasks drawn under any of them, in the order `cards`
/// has them: what moves when those cards are moved, since a sub-task goes where its card goes.
pub(crate) fn with_sub_tasks<C: Card>(cards: &[C], task_ids: &[String]) -> Vec<String> {
    let at = places_of(cards);
    cards
        .iter()
        .filter(|card| {
            task_ids.iter().any(|id| id == card.id())
                || drawn_under(&at, cards, *card)
                    .is_some_and(|parent| task_ids.iter().any(|id| id == cards[parent].id()))
        })
        .map(|card| card.id().to_string())
        .collect()
}

/// The card a moved card ends up under: the one it was dropped onto or among the sub-tasks
/// of, or - dropped anywhere else - the card it was under, if that card moved with it.
pub(crate) fn parent_after_move(
    own_parent: Option<&str>,
    dropped_under: Option<&str>,
    moving: &[String],
) -> Option<String> {
    match dropped_under {
        Some(parent) => Some(parent.to_string()),
        None => own_parent
            .filter(|parent| moving.iter().any(|id| id == parent))
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain(&'static str, Option<&'static str>, ColumnId);

    impl Card for Plain {
        fn id(&self) -> &str {
            self.0
        }
        fn parent(&self) -> Option<&str> {
            self.1
        }
        fn column(&self) -> &ColumnId {
            &self.2
        }
    }

    fn card(id: &'static str, parent: Option<&'static str>, column: &str) -> Plain {
        Plain(id, parent, ColumnId::new(column))
    }

    fn ids(cards: &[Plain]) -> Vec<&str> {
        cards.iter().map(|card| card.0).collect()
    }

    /// Sub-tasks follow their card in the order they had; a sub-task whose card is in another
    /// column, or is itself a sub-task, stays where it was as an ordinary card.
    #[test]
    fn sub_tasks_are_put_right_after_their_card() {
        let mut cards = vec![
            card("b1", Some("a"), "todo"),
            card("a", None, "todo"),
            card("x", None, "todo"),
            card("elsewhere", Some("x"), "done"),
            card("b2", Some("a"), "todo"),
            card("deep", Some("b1"), "todo"),
        ];
        under_their_parents(&mut cards);
        assert_eq!(ids(&cards), ["a", "b1", "b2", "x", "elsewhere", "deep"]);
        assert_eq!(
            drawn_under_ids(&cards),
            [None, Some("a"), Some("a"), None, None, None]
        );
        assert_eq!(
            with_sub_tasks(&cards, &["a".to_string()]),
            ["a", "b1", "b2"]
        );
    }
}
