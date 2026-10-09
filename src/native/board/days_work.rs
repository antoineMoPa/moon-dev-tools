//! The lines a queue is split into days by: reading down TODO, the cards under `today`
//! are as many as the rest of today finishes, the ones under `tomorrow` are a day's work, and
//! so on down the column.
//!
//! How many a day finishes comes off the board's own record. Every card in the column pinned
//! by [`store::COUNTS_A_DAYS_WORK_IN`] says which day it arrived there on, so the board knows
//! how many were finished on each day - and a day's work is the median of the latest of those
//! days, once they hold enough cards to be worth reading a pace from.
//!
//! Today is not a whole day, though, and less of one every hour. What it still has room for
//! is the share of a day's work that fits in what is left of it - a day being
//! [`WORKDAY_MINUTES`] long and ending when the project says it does, see
//! [`crate::project::ProjectConfig::day_ends_at`] - and never more than a day's work less what
//! today has finished already. Once the day has ended, or done its share, there is no `today`
//! line at all and the column starts at `tomorrow`.
//!
//! The days after tomorrow are counted rather than named: a day here is a day that is worked,
//! and the board does not know which days of the week those are.
//!
//! Only a column that says so draws them - see [`store::BoardColumn::marks_a_days_work`]. A
//! queue that does not fill a day draws nothing to say so: a line counting the room left would
//! be an invitation to make up work to fill it.
//!
//! Today's line is yellow and the line of every day after it is blue, so a queue that starts
//! at tomorrow is not read as one that starts at today.

use std::collections::BTreeMap;

use egui::Ui;

use crate::{
    moontasks::{ColumnId, TaskView, store},
    native::theme::Palette,
    project::TimeOfDay,
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

/// How long a day of work is taken to be, counted back from when the project says it ends.
/// It is what the hours left of today are a share of.
const WORKDAY_MINUTES: u32 = 8 * 60;

/// What the heading's menu calls the setting, and says about it.
pub(super) const MENU_LABEL: &str = "mark a day's work";
pub(super) const MENU_HOVER: &str = "lines that split the column into days: as many cards under \
                                     each as that day usually finishes";

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

/// How many of a day's cards today still has room for - see the module's own words on it.
///
/// Rounded to the nearest card, since a share of a day is rarely a whole number of them: half
/// a day left of a three-card day has room for two, and a quarter of one for one.
fn cards_left_today(cards_a_day: usize, finished_today: usize, minutes_left: u32) -> usize {
    let share_left = minutes_left.min(WORKDAY_MINUTES) as f32 / WORKDAY_MINUTES as f32;
    let by_the_clock = (cards_a_day as f32 * share_left).round() as usize;
    by_the_clock.min(cards_a_day.saturating_sub(finished_today))
}

/// A day the queue reaches: today is `0`, tomorrow `1`, and so on in days that are worked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct DayAhead(usize);

impl DayAhead {
    fn name(self) -> String {
        match self.0 {
            0 => "today".to_string(),
            1 => "tomorrow".to_string(),
            days => format!("in {days} days"),
        }
    }
}

/// The days a queue is split into, reading down it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Days {
    cards_a_day: usize,
    finished_today: usize,
    left_today: usize,
    day_ends_at: TimeOfDay,
}

impl Days {
    /// The days as they stand now, for a project whose day ends at `day_ends_at`. `None`
    /// while the board has no pace to read - see [`cards_a_day_finished_before`].
    pub(super) fn reckoned_now(tasks: &[TaskView], day_ends_at: TimeOfDay) -> Option<Self> {
        let now = store::now_unix();
        Self::reckoned(
            tasks,
            LocalDay::of(now),
            day_lines::minutes_into_local_day(now),
            day_ends_at,
        )
    }

    fn reckoned(
        tasks: &[TaskView],
        today: LocalDay,
        minutes_into_today: u32,
        day_ends_at: TimeOfDay,
    ) -> Option<Self> {
        let cards_a_day = cards_a_day_finished_before(tasks, today)?;
        let finished_in = ColumnId::new(store::COUNTS_A_DAYS_WORK_IN);
        let finished_today = day_lines::cards_arrived_on(tasks, &finished_in, today);
        let minutes_left = day_ends_at
            .minutes_into_day()
            .saturating_sub(minutes_into_today);
        Some(Self {
            cards_a_day,
            finished_today,
            left_today: cards_left_today(cards_a_day, finished_today, minutes_left),
            day_ends_at,
        })
    }

    /// The first day the queue reaches: today while it has room, and tomorrow once it has none.
    fn first(self) -> DayAhead {
        DayAhead(usize::from(self.left_today == 0))
    }

    /// The day that starts at the card with `cards_above` it, if one does: the day whose line
    /// is drawn over that card.
    pub(super) fn starting_at(self, cards_above: usize) -> Option<DayAhead> {
        if cards_above == 0 {
            return Some(self.first());
        }
        let past_today = cards_above.checked_sub(self.left_today)?;
        (past_today.is_multiple_of(self.cards_a_day))
            .then(|| DayAhead(past_today / self.cards_a_day + 1))
    }

    /// How the days came by their numbers, for whoever rests the pointer on one of the lines.
    fn reasons(self) -> String {
        let cards = |count: usize| match count {
            1 => "1 card".to_string(),
            count => format!("{count} cards"),
        };
        format!(
            "{} a day: what a day finished, at the median of the last {DAYS_READ} days that \
             finished any.\nToday has room for {}: it has finished {}, and the day ends at {} - \
             which is set in the project's settings.",
            cards(self.cards_a_day),
            cards(self.left_today),
            cards(self.finished_today),
            self.day_ends_at,
        )
    }
}

/// What a day's line is drawn in. Today is what is being worked through, and is the one that
/// stands out from the days after it.
fn color_of(day: DayAhead, palette: &Palette) -> egui::Color32 {
    match day.0 {
        0 => palette.days_work,
        _ => palette.days_work_ahead,
    }
}

/// The line over the first card of a day: a rule in the day's color that names it, and on
/// hover says how the days came by their numbers.
pub(super) fn draw(ui: &mut Ui, palette: &Palette, days: Days, day: DayAhead) {
    let color = color_of(day, palette);
    day_lines::draw_rule(ui, &day.name(), color, color).on_hover_text(days.reasons());
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
            remote_task_tracker_url: String::new(),
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

    /// Days that finish `cards_a_day`, on a day that has `left_today` of them left.
    fn days(cards_a_day: usize, left_today: usize) -> Days {
        Days {
            cards_a_day,
            finished_today: 0,
            left_today,
            day_ends_at: TimeOfDay::new(16, 30),
        }
    }

    /// The day whose line is over each of the first `cards` cards, by its name.
    fn lines_over(days: Days, cards: usize) -> Vec<Option<String>> {
        (0..cards)
            .map(|cards_above| days.starting_at(cards_above).map(DayAhead::name))
            .collect()
    }

    /// Today has room for what is left of it, counted in the share of a day's work that fits
    /// before the day ends - and none once it has ended.
    #[test]
    fn today_has_room_for_the_share_of_a_day_that_is_left() {
        // A four-card day, with nothing finished yet.
        assert_eq!(cards_left_today(4, 0, 8 * 60), 4, "the whole day ahead");
        assert_eq!(cards_left_today(4, 0, 12 * 60), 4, "and no more than a day");
        assert_eq!(cards_left_today(4, 0, 4 * 60), 2, "half of it");
        assert_eq!(cards_left_today(4, 0, 90), 1, "an hour and a half");
        assert_eq!(cards_left_today(4, 0, 30), 0, "half an hour is no card");
        assert_eq!(cards_left_today(4, 0, 0), 0, "the day has ended");
    }

    /// What today has finished already comes off what it has room for: a day does a day's
    /// work, however early it got through it.
    #[test]
    fn today_has_no_more_room_than_a_days_work_less_what_it_finished() {
        assert_eq!(cards_left_today(4, 3, 8 * 60), 1);
        assert_eq!(cards_left_today(4, 6, 8 * 60), 0);
        assert_eq!(
            cards_left_today(4, 1, 4 * 60),
            2,
            "the clock is the tighter of the two"
        );
    }

    /// Reading down the queue: today's line over the first card, tomorrow's over the first
    /// card today has no room for, and a line every day's work after that.
    #[test]
    fn a_queue_is_split_into_today_tomorrow_and_the_days_after() {
        assert_eq!(
            lines_over(days(3, 2), 9),
            [
                Some("today".to_string()),
                None,
                Some("tomorrow".to_string()),
                None,
                None,
                Some("in 2 days".to_string()),
                None,
                None,
                Some("in 3 days".to_string()),
            ]
        );
    }

    /// Once today has no room - it has ended, or done its share - the queue starts at tomorrow.
    #[test]
    fn a_queue_starts_at_tomorrow_once_today_has_no_room() {
        assert_eq!(
            lines_over(days(2, 0), 5),
            [
                Some("tomorrow".to_string()),
                None,
                Some("in 2 days".to_string()),
                None,
                Some("in 3 days".to_string()),
            ]
        );
    }

    /// Today's line is a color of its own, and every day after it shares another.
    #[test]
    fn the_days_after_today_are_not_the_color_of_today() {
        let palette = Palette::of(crate::native::theme::ThemeMode::Dark);

        assert_eq!(color_of(DayAhead(0), &palette), palette.days_work);
        assert_eq!(color_of(DayAhead(1), &palette), palette.days_work_ahead);
        assert_eq!(color_of(DayAhead(2), &palette), palette.days_work_ahead);
        assert_ne!(palette.days_work, palette.days_work_ahead);
    }

    /// The whole of it from the board: a day finished three cards on each of four days, one
    /// is finished today already, and the day ends at half past four.
    #[test]
    fn the_days_are_reckoned_from_the_board_and_the_clock() {
        let mut tasks = finished(&[(1, 3), (2, 3), (3, 3), (4, 3)]);
        tasks.push(card("done", Some(noon_on(5))));
        let today = LocalDay::of(noon_on(5));
        let ends = TimeOfDay::new(16, 30);
        let at = |hour: u32, minute: u32| {
            Days::reckoned(&tasks, today, hour * 60 + minute, ends).expect("expected a pace")
        };

        assert_eq!(
            at(9, 0).left_today,
            2,
            "a day's three, less the one finished"
        );
        assert_eq!(at(14, 30).left_today, 1, "two hours are a quarter of a day");
        assert_eq!(at(16, 30).left_today, 0, "the day has ended");
        assert_eq!(
            at(16, 30).starting_at(0).map(DayAhead::name).as_deref(),
            Some("tomorrow")
        );
        assert_eq!(at(22, 0).left_today, 0);
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
