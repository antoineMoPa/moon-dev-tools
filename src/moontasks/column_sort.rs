//! The order a column keeps its cards in when it is told to keep one, rather than the order
//! they were dragged into - see [`crate::moontasks::store::BoardColumn::sort`].
//!
//! Both ends read it: the server lists each sorted column in this order, and the board puts a
//! card dropped into one where this order says while the server's answer is on its way. One
//! function for both, so the two never disagree about where a card goes.
//!
//! A title is read as runs of letters and runs of digits, and a run of digits is compared as
//! the number it is. So `Step 2` comes before `Step 10` and `01` before `02`, the way a person
//! numbering their cards means them to read - where plain string order would put `10` before
//! `2`.

use std::cmp::Ordering;

use crate::moontasks::{BoardColumn, ColumnSort, TaskView};

/// Put the cards of every sorted column in that column's order.
///
/// Each column's cards are put back into the slots of the list they already had, so the
/// columns that keep the order they were dragged into are left exactly as they were.
pub(crate) fn arrange(columns: &[BoardColumn], tasks: &mut [TaskView]) {
    for column in columns {
        let Some(sort) = column.sort else {
            continue;
        };
        let slots: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.status == column.id)
            .map(|(at, _)| at)
            .collect();
        let mut cards: Vec<TaskView> = slots.iter().map(|&at| tasks[at].clone()).collect();
        sort_cards(sort, &mut cards);
        for (at, card) in slots.into_iter().zip(cards) {
            tasks[at] = card;
        }
    }
}

/// One column's cards, in the order `sort` keeps. Stable, so cards the order cannot tell
/// apart - two cards titled the same - keep the order they were dragged into.
pub(crate) fn sort_cards(sort: ColumnSort, cards: &mut [TaskView]) {
    cards.sort_by(|left, right| compare(sort, left, right));
}

fn compare(sort: ColumnSort, left: &TaskView, right: &TaskView) -> Ordering {
    match sort {
        ColumnSort::Alphabetical => title_order(&left.title, &right.title),
        ColumnSort::Numerical => numbers_order(&left.title, &right.title)
            .then_with(|| title_order(&left.title, &right.title)),
        ColumnSort::NewestFirst => right.created_at_unix.cmp(&left.created_at_unix),
        ColumnSort::OldestFirst => left.created_at_unix.cmp(&right.created_at_unix),
    }
}

/// A run of a title: digits, or everything between them.
#[derive(Clone, Copy)]
enum Run<'a> {
    Digits(&'a str),
    Text(&'a str),
}

/// A title cut into its runs of digits and the text between them.
fn runs(title: &str) -> Vec<Run<'_>> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut in_digits = None;
    for (at, character) in title.char_indices() {
        let digit = character.is_ascii_digit();
        if let Some(was_digits) = in_digits
            && was_digits != digit
        {
            runs.push(run_of(&title[start..at], was_digits));
            start = at;
        }
        in_digits = Some(digit);
    }
    if let Some(was_digits) = in_digits {
        runs.push(run_of(&title[start..], was_digits));
    }
    runs
}

fn run_of(text: &str, digits: bool) -> Run<'_> {
    if digits {
        Run::Digits(text)
    } else {
        Run::Text(text)
    }
}

/// Titles in reading order: letters without regard to case, digits by the number they are,
/// and a number before a word, the way digits come before letters in any listing.
fn title_order(left: &str, right: &str) -> Ordering {
    let (left, right) = (runs(left), runs(right));
    for (left_run, right_run) in left.iter().zip(&right) {
        let order = match (left_run, right_run) {
            (Run::Digits(left), Run::Digits(right)) => number_order(left, right),
            (Run::Text(left), Run::Text(right)) => text_order(left, right),
            (Run::Digits(_), Run::Text(_)) => Ordering::Less,
            (Run::Text(_), Run::Digits(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

/// Titles by the numbers in them, the first of them first. A title with no number goes after
/// every title with one: it is a card the numbering does not place.
fn numbers_order(left: &str, right: &str) -> Ordering {
    let numbers_in = |title| -> Vec<&str> {
        runs(title)
            .into_iter()
            .filter_map(|run| match run {
                Run::Digits(digits) => Some(digits),
                Run::Text(_) => None,
            })
            .collect()
    };
    let (left, right) = (numbers_in(left), numbers_in(right));
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    for (left_number, right_number) in left.iter().zip(&right) {
        let order = number_order(left_number, right_number);
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

/// Two runs of digits as the numbers they are. Compared as text once their leading zeros are
/// gone - the longer is the larger, and two as long read in digit order - so a number too long
/// for any integer type is still a number.
fn number_order(left: &str, right: &str) -> Ordering {
    let (left, right) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn text_order(left: &str, right: &str) -> Ordering {
    left.chars()
        .flat_map(char::to_lowercase)
        .cmp(right.chars().flat_map(char::to_lowercase))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moontasks::ColumnId;

    fn card(title: &str, status: &str, created_at_unix: u64) -> TaskView {
        TaskView {
            id: format!("{title}-1111"),
            title: title.to_string(),
            status: ColumnId::new(status),
            created_at_unix,
            dir_path: String::new(),
            repo_path: String::new(),
            notes: String::new(),
            resources: Vec::new(),
        }
    }

    fn sorted(sort: ColumnSort, titles: &[&str]) -> Vec<String> {
        let mut cards: Vec<TaskView> = titles.iter().map(|title| card(title, "todo", 0)).collect();
        sort_cards(sort, &mut cards);
        cards.into_iter().map(|card| card.title).collect()
    }

    #[test]
    fn alphabetical_reads_numbers_as_numbers() {
        assert_eq!(
            sorted(
                ColumnSort::Alphabetical,
                &["02 deploy", "10 celebrate", "01 build"]
            ),
            ["01 build", "02 deploy", "10 celebrate"]
        );
        assert_eq!(
            sorted(
                ColumnSort::Alphabetical,
                &["Task 10", "task 2", "Task 01 b", "apple", "Task 1"]
            ),
            ["apple", "Task 1", "Task 01 b", "task 2", "Task 10"],
            "case is not an order, and 01 is the same number as 1"
        );
    }

    #[test]
    fn numerical_goes_by_the_numbers_wherever_they_are_in_the_title() {
        assert_eq!(
            sorted(
                ColumnSort::Numerical,
                &[
                    "fix login",
                    "release 2.0",
                    "v10 cleanup",
                    "#3 docs",
                    "issue 3 part 1"
                ]
            ),
            [
                "release 2.0",
                "#3 docs",
                "issue 3 part 1",
                "v10 cleanup",
                "fix login"
            ],
            "the first number first, then the next, and no number at all last"
        );
    }

    #[test]
    fn titles_the_numbers_cannot_tell_apart_go_alphabetically() {
        assert_eq!(
            sorted(ColumnSort::Numerical, &["zebra", "7 b", "apple", "7 a"]),
            ["7 a", "7 b", "apple", "zebra"]
        );
    }

    #[test]
    fn a_number_longer_than_any_integer_is_still_a_number() {
        assert_eq!(
            sorted(
                ColumnSort::Alphabetical,
                &["123456789012345678901234567890", "99"]
            ),
            ["99", "123456789012345678901234567890"]
        );
    }

    #[test]
    fn newest_and_oldest_go_by_when_the_card_was_made() {
        let mut cards = vec![
            card("b", "todo", 2),
            card("c", "todo", 3),
            card("a", "todo", 1),
        ];

        sort_cards(ColumnSort::NewestFirst, &mut cards);
        let newest: Vec<&str> = cards.iter().map(|card| card.title.as_str()).collect();
        assert_eq!(newest, ["c", "b", "a"]);

        sort_cards(ColumnSort::OldestFirst, &mut cards);
        let oldest: Vec<&str> = cards.iter().map(|card| card.title.as_str()).collect();
        assert_eq!(oldest, ["a", "b", "c"]);
    }

    /// A sorted column is put in order in the slots its cards already had in the board's one
    /// list, and a column that is not sorted is left as it was dragged.
    #[test]
    fn only_sorted_columns_are_put_in_order() {
        let columns = vec![
            BoardColumn {
                id: ColumnId::new("todo"),
                label: "TODO".to_string(),
                arrivals: None,
                sort: Some(ColumnSort::Alphabetical),
            },
            BoardColumn {
                id: ColumnId::new("done"),
                label: "DONE".to_string(),
                arrivals: None,
                sort: None,
            },
        ];
        let mut tasks = vec![
            card("b", "todo", 0),
            card("z", "done", 0),
            card("a", "todo", 0),
            card("y", "done", 0),
        ];

        arrange(&columns, &mut tasks);

        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["a", "z", "b", "y"]);
    }
}
