//! The yellow line a queue draws under a day's work: reading down TODO, the cards above it are
//! as many as a day usually finishes, and the cards under it are for another day.
//!
//! How many that is comes off the board's own record. Every card in the column pinned by
//! [`store::COUNTS_A_DAYS_WORK_IN`] says which day it arrived there on, so the board knows how
//! many were finished on each day - and a day's work is the median of the latest of those days,
//! once they hold enough cards to be worth reading a pace from.
//!
//! Only a column that says so draws it - see [`store::BoardColumn::marks_a_days_work`]. A
//! column holding less than a day's work draws it under its last card, saying how many more
//! cards the day has room for: a short queue is the one that wants filling.

use std::collections::BTreeMap;

use egui::Ui;

use crate::{
    moontasks::{ColumnId, TaskView, store},
    native::theme::Palette,
};

use super::day_lines::{self, LocalDay};

/// How many days the estimate reads: the latest ones that finished anything. Few enough that
/// the line follows the pace of this month rather than of last spring, and enough that one slow
/// day does not move it.
const DAYS_READ: usize = 14;

/// How many finished cards those days have to hold between them for there to be an estimate at
/// all. A board a few cards old has a pace nobody would bet a day on, and a line drawn from it
/// would be read as if it knew something.
const CARDS_NEEDED: usize = 10;

/// What the line says when the cards above it are as many as a day finishes.
const LINE_LABEL: &str = "about a day's work";

/// What the heading's menu calls the setting, and says about it.
pub(super) const MENU_LABEL: &str = "mark a day's work";
pub(super) const MENU_HOVER: &str = "a yellow line under as many cards, counted from the top, as \
                                     a day usually finishes";

/// How many cards a day finishes, going by the board as it stands now.
pub(super) fn cards_a_day_finishes(tasks: &[TaskView]) -> Option<usize> {
    cards_a_day_finished_before(tasks, LocalDay::of(store::now_unix()))
}

/// How many cards a day finished, over the [`DAYS_READ`] latest days before `today` that
/// finished any. Nothing while those days hold fewer than [`CARDS_NEEDED`] cards between them:
/// there is no pace to read yet, and a line at a guess would be worse than none.
///
/// Today is left out because it is not over, and a morning with one card finished is not a day
/// that finished one card. A day that finished nothing is not counted as a zero either: the
/// board cannot tell a day off from a day spent on one long task, so what is estimated is a day
/// that is worked.
///
/// The median rather than the mean, because of the day a pile of old cards is swept into the
/// column at once - that is one day of the fourteen, not thirty cards to share out among them.
/// Of an even number of days it is the lower of the two in the middle, so the line promises a
/// card too few rather than one too many.
fn cards_a_day_finished_before(tasks: &[TaskView], today: LocalDay) -> Option<usize> {
    let finished_in = ColumnId::new(store::COUNTS_A_DAYS_WORK_IN);
    // Keyed by the day's number so the days read back in order, latest last.
    let mut finished_on: BTreeMap<i64, usize> = BTreeMap::new();
    for task in tasks.iter().filter(|task| task.status == finished_in) {
        // A card moved in before the board wrote days down is finished work nobody can date.
        let Some(arrived) = task.entered_column_at_unix else {
            continue;
        };
        let day = LocalDay::of(arrived);
        if day == today {
            continue;
        }
        *finished_on.entry(day.days_since_epoch()).or_default() += 1;
    }

    let mut a_day: Vec<usize> = finished_on.into_values().rev().take(DAYS_READ).collect();
    if a_day.iter().sum::<usize>() < CARDS_NEEDED {
        return None;
    }
    a_day.sort_unstable();
    Some(a_day[(a_day.len() - 1) / 2])
}

/// What the line says with `cards_above` it: that they are a day's work, or - under the last
/// card of a column holding fewer - how many more the day has room for.
fn label(cards_a_day: usize, cards_above: usize) -> String {
    assert!(
        cards_above <= cards_a_day,
        "the line stands under at most a day's work: {cards_above} cards above it, \
         {cards_a_day} a day"
    );
    if cards_above == cards_a_day {
        return LINE_LABEL.to_string();
    }
    format!("a day has room for {} more", cards_a_day - cards_above)
}

/// The line itself, under the `cards_above` it: a yellow rule that says what it is, and on
/// hover how it came by the number.
pub(super) fn draw(ui: &mut Ui, palette: &Palette, cards_a_day: usize, cards_above: usize) {
    day_lines::draw_rule(
        ui,
        &label(cards_a_day, cards_above),
        palette.days_work,
        palette.days_work,
    )
    .on_hover_text(format!(
        "{cards_a_day} {} a day: what a day finished, at the median of the last {DAYS_READ} \
         days that finished any",
        if cards_a_day == 1 { "card" } else { "cards" }
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon UTC on a day in November 2023, which is that day in every zone a board is read in.
    fn noon_on(day_of_november: u64) -> u64 {
        1_698_840_000 + (day_of_november - 1) * 86_400
    }

    fn card(column: &str, arrived: Option<u64>) -> TaskView {
        TaskView {
            id: String::new(),
            title: String::new(),
            status: ColumnId::new(column),
            created_at_unix: 0,
            entered_column_at_unix: arrived,
            dir_path: String::new(),
            repo_path: String::new(),
            tags: Vec::new(),
            notes: String::new(),
            attachments: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// As many finished cards on each day as the list says, in the order given.
    fn finished(a_day: &[(u64, usize)]) -> Vec<TaskView> {
        a_day
            .iter()
            .flat_map(|(day, cards)| {
                std::iter::repeat_with(|| card("done", Some(noon_on(*day)))).take(*cards)
            })
            .collect()
    }

    #[test]
    fn a_short_queue_is_told_how_much_more_the_day_takes() {
        assert_eq!(label(6, 6), "about a day's work");
        assert_eq!(label(6, 3), "a day has room for 3 more");
    }

    #[test]
    fn a_days_work_is_the_median_of_the_days_that_finished_anything() {
        let tasks = finished(&[(1, 2), (2, 5), (3, 30), (6, 4), (7, 6)]);

        // 2, 4, 5, 6, 30: the day thirty cards were swept in moves the middle by nothing.
        assert_eq!(
            cards_a_day_finished_before(&tasks, LocalDay::of(noon_on(8))),
            Some(5)
        );
    }

    /// Of an even number of days the lower middle is the one taken.
    #[test]
    fn an_even_number_of_days_takes_the_lower_of_the_two_in_the_middle() {
        let tasks = finished(&[(1, 3), (2, 8), (3, 4), (4, 9)]);

        assert_eq!(
            cards_a_day_finished_before(&tasks, LocalDay::of(noon_on(8))),
            Some(4)
        );
    }

    #[test]
    fn today_is_not_a_day_yet() {
        let tasks = finished(&[(1, 6), (2, 6), (3, 6), (8, 1)]);

        assert_eq!(
            cards_a_day_finished_before(&tasks, LocalDay::of(noon_on(8))),
            Some(6)
        );
        assert_eq!(
            cards_a_day_finished_before(&finished(&[(8, 30)]), LocalDay::of(noon_on(8))),
            None,
            "a board that has only today has no day to go by"
        );
    }

    /// Under ten finished cards there is no pace to speak of, however many days they are over.
    #[test]
    fn a_handful_of_finished_cards_is_not_a_pace() {
        let today = LocalDay::of(noon_on(8));

        assert_eq!(
            cards_a_day_finished_before(&finished(&[(1, 4), (2, 5)]), today),
            None
        );
        assert_eq!(
            cards_a_day_finished_before(&finished(&[(1, 4), (2, 6)]), today),
            Some(4)
        );
        // Ten of them, one of which is today's and so not read.
        assert_eq!(
            cards_a_day_finished_before(&finished(&[(1, 4), (2, 5), (8, 1)]), today),
            None
        );
    }

    /// Only the latest days are read, so the pace of a month ago does not hold the line.
    #[test]
    fn only_the_latest_days_are_read() {
        let mut a_day: Vec<(u64, usize)> = (1..=5).map(|day| (day, 20)).collect();
        a_day.extend((6..6 + DAYS_READ as u64).map(|day| (day, 3)));
        let tasks = finished(&a_day);

        assert_eq!(
            cards_a_day_finished_before(&tasks, LocalDay::of(noon_on(30))),
            Some(3)
        );
    }

    /// What is counted is what was finished: cards waiting in other columns say nothing about
    /// the pace, and neither does a finished card nobody wrote a day on.
    #[test]
    fn only_dated_cards_in_the_finished_column_are_counted() {
        let mut tasks = finished(&[(1, 12)]);
        tasks.push(card("todo", Some(noon_on(1))));
        tasks.push(card("in_progress", Some(noon_on(2))));
        tasks.push(card("done", None));

        assert_eq!(
            cards_a_day_finished_before(&tasks, LocalDay::of(noon_on(8))),
            Some(12)
        );
        let waiting: Vec<TaskView> = std::iter::repeat_with(|| card("todo", Some(noon_on(1))))
            .take(12)
            .collect();
        assert_eq!(
            cards_a_day_finished_before(&waiting, LocalDay::of(noon_on(8))),
            None
        );
    }
}
