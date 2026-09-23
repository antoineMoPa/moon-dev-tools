//! The date lines the DONE column draws between its cards: `---- September 21 ----` wherever,
//! reading down the column, the day a card arrived on changes. A column read newest first is
//! a record of what was finished when, and the lines are what makes the "when" legible.
//!
//! Only the column pinned by [`store::DATES_ARRIVALS_IN`] draws them, and only while it keeps
//! its cards in the order they were put: a column sorted by title has its days scattered, and
//! a line at every change of day would be noise rather than a record.

use egui::{FontId, Label, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::{
    moontasks::{BoardColumn, ColumnId, store},
    native::theme::{Palette, SMALL_SIZE},
};

/// How tall the line is, text and all - a little under a card's row of tags.
const LINE_HEIGHT: f32 = 16.0;
/// Between the rule and the text either side of it.
const TEXT_GAP: f32 = 6.0;
/// How far the rule stops short of the column's edges.
const RULE_INSET: f32 = 4.0;

/// Whether a column is one that draws these lines.
pub(super) fn drawn_in(column: &BoardColumn) -> bool {
    column.id == ColumnId::new(store::DATES_ARRIVALS_IN) && column.sort.is_none()
}

/// A calendar day in this machine's time zone - which is the zone the person reading the
/// board is in, and the one "yesterday" means anything in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct LocalDay {
    year: i32,
    /// January is 1.
    month: u32,
    day: u32,
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl LocalDay {
    /// The day a moment falls on, in this machine's zone. `localtime_r` rather than a date
    /// crate: the zone and its daylight-saving rules are the OS's to know, and a zone read
    /// once at startup would be wrong across the night the clocks change.
    pub(super) fn of(unix: u64) -> Self {
        let time = unix as libc::time_t;
        // SAFETY: `libc::tm` is plain data - integers, and on macOS a pointer to a zone name
        // that `localtime_r` sets or leaves null - so zeroes are a valid value of it.
        let mut written: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers are to locals that outlive the call, and `localtime_r`
        // writes to no other memory, which is what makes it the thread-safe one.
        let placed = unsafe { libc::localtime_r(&time, &mut written) };
        assert!(
            !placed.is_null(),
            "localtime_r could not place {unix} in this machine's zone"
        );
        Self {
            year: written.tm_year + 1900,
            month: (written.tm_mon + 1) as u32,
            day: written.tm_mday as u32,
        }
    }

    /// What the line says: the two days everyone knows by name, and every other by its date -
    /// with the year only once it is not this year's.
    pub(super) fn label(self, today: LocalDay) -> String {
        if self == today {
            return "today".to_string();
        }
        if self.days_since_epoch() + 1 == today.days_since_epoch() {
            return "yesterday".to_string();
        }
        let month = MONTH_NAMES[(self.month - 1) as usize];
        if self.year == today.year {
            format!("{month} {}", self.day)
        } else {
            format!("{month} {}, {}", self.day, self.year)
        }
    }

    /// Days from 1970-01-01, which is what makes "the day before" a subtraction. The civil
    /// calendar arithmetic is Howard Hinnant's, with March as the first month of the year so
    /// that the leap day falls at the end.
    fn days_since_epoch(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
            + i64::from(self.day)
            - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }
}

/// The walk down a column: which cards get a line above them, and what it says.
pub(super) struct DayLines {
    today: LocalDay,
    /// The day of the last dated card, which the next card's day is a change from or not.
    previous: Option<LocalDay>,
}

impl DayLines {
    pub(super) fn starting_now() -> Self {
        Self::from(LocalDay::of(store::now_unix()))
    }

    pub(super) fn from(today: LocalDay) -> Self {
        Self {
            today,
            previous: None,
        }
    }

    /// The line above the next card down, if its day is a change from the card before it. A
    /// card with no day - one moved in before the board wrote days down - gets no line and
    /// changes nothing: the cards under it read as the day above until one says otherwise.
    pub(super) fn line_above(&mut self, day: Option<LocalDay>) -> Option<String> {
        let day = day?;
        if self.previous == Some(day) {
            return None;
        }
        self.previous = Some(day);
        Some(day.label(self.today))
    }
}

/// The line itself: a rule across the column with the day in the middle of it.
pub(super) fn draw(ui: &mut Ui, palette: &Palette, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LINE_HEIGHT), Sense::hover());
    let font = FontId::proportional(SMALL_SIZE - 1.0);
    let text_size = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), palette.muted)
        .size();
    let text_left = rect.center().x - text_size.x / 2.0;
    let rule = Stroke::new(1.0, palette.line);
    ui.painter().hline(
        rect.min.x + RULE_INSET..=text_left - TEXT_GAP,
        rect.center().y,
        rule,
    );
    ui.painter().hline(
        text_left + text_size.x + TEXT_GAP..=rect.max.x - RULE_INSET,
        rect.center().y,
        rule,
    );
    // A label rather than a painted galley, so the day reads out the way a card's title does.
    ui.put(
        Rect::from_center_size(rect.center(), text_size),
        Label::new(RichText::new(label).font(font).color(palette.muted)).selectable(false),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> LocalDay {
        LocalDay { year, month, day }
    }

    #[test]
    fn a_day_is_named_by_how_far_back_it_is() {
        let today = day(2026, 9, 22);

        assert_eq!(today.label(today), "today");
        assert_eq!(day(2026, 9, 21).label(today), "yesterday");
        assert_eq!(day(2026, 9, 20).label(today), "September 20");
        assert_eq!(day(2025, 12, 31).label(today), "December 31, 2025");
    }

    /// Yesterday across a month, a year and a leap day: the day arithmetic, not the label.
    #[test]
    fn yesterday_is_the_day_before_whatever_the_calendar_does() {
        assert_eq!(day(2026, 8, 31).label(day(2026, 9, 1)), "yesterday");
        assert_eq!(day(2025, 12, 31).label(day(2026, 1, 1)), "yesterday");
        assert_eq!(day(2024, 2, 29).label(day(2024, 3, 1)), "yesterday");
        assert_eq!(day(2026, 9, 20).label(day(2026, 9, 22)), "September 20");
    }

    /// The lines go where the day changes, reading down; a card with no day is passed over.
    #[test]
    fn a_line_stands_wherever_the_day_changes() {
        let mut lines = DayLines::from(day(2026, 9, 22));
        let cards = [
            Some(day(2026, 9, 22)),
            Some(day(2026, 9, 22)),
            None,
            Some(day(2026, 9, 21)),
            Some(day(2026, 9, 21)),
            Some(day(2026, 9, 19)),
        ];

        let above: Vec<Option<String>> = cards
            .into_iter()
            .map(|card| lines.line_above(card))
            .collect();

        assert_eq!(
            above,
            [
                Some("today".to_string()),
                None,
                None,
                Some("yesterday".to_string()),
                None,
                Some("September 19".to_string()),
            ]
        );
    }

    /// `localtime_r` against `date`, which reads the same zone.
    #[test]
    fn a_moment_falls_on_the_day_the_system_says() {
        let unix = 1_758_000_000_u64;
        // BSD `date` reads a moment as `-r`, GNU `date` as `-d @`.
        let moment = if cfg!(target_os = "macos") {
            vec!["-r".to_string(), unix.to_string()]
        } else {
            vec!["-d".to_string(), format!("@{unix}")]
        };
        let output = std::process::Command::new("date")
            .args(moment)
            .arg("+%Y-%m-%d")
            .output()
            .expect("expected `date` to run");
        let expected = String::from_utf8(output.stdout)
            .expect("expected a date")
            .trim()
            .to_string();

        let day = LocalDay::of(unix);

        assert_eq!(
            format!("{}-{:02}-{:02}", day.year, day.month, day.day),
            expected
        );
    }
}
