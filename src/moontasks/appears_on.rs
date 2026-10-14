//! Appears on - the day a card made ahead of time shows up on the board.
//!
//! `moon tasks new --appears-on 2026-11-02 "renew the certificate"` writes the card now, and the
//! board leaves it out until that day comes on this machine's clock. `moon tasks list` lists it
//! all along, with its day, so it can be found before then.
//!
//! The day is written in `metadata.json` as `"appears_on": "2026-11-02"`, which reads the same
//! to a person editing the file as to `--appears-on`.

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serializer};
use time::{Date, Month, format_description::BorrowedFormatItem};

/// How the day is spelled, on the command line and in the file alike.
const DAY_FORMAT: &str = "[year]-[month]-[day]";

fn day_format() -> Vec<BorrowedFormatItem<'static>> {
    time::format_description::parse_borrowed::<2>(DAY_FORMAT)
        .expect("DAY_FORMAT is a valid description")
}

/// A day spelled `2026-11-02`.
pub(crate) fn parse_day(day: &str) -> Result<Date> {
    Date::parse(day, &day_format())
        .with_context(|| format!("{day:?} is no day: a day is written like 2026-11-02"))
}

/// Whether a card that appears on `day` is still to come, `today` being the day it is here.
pub(crate) fn still_to_come(appears_on: Option<Date>, today: Date) -> bool {
    appears_on.is_some_and(|day| day > today)
}

/// The day it is on this machine's clock, in its time zone - the one the person reading the
/// board is in, and so the one "the 2nd" means.
pub(crate) fn today_here() -> Date {
    let now = super::store::now_unix() as libc::time_t;
    // SAFETY: zeroes are a valid `libc::tm`, which `localtime_r` writes over.
    let mut written: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to locals that outlive the call.
    let placed = unsafe { libc::localtime_r(&now, &mut written) };
    assert!(
        !placed.is_null(),
        "localtime_r could not place {now} in this machine's zone"
    );
    let month = Month::try_from(written.tm_mon as u8 + 1).expect("localtime_r gives months 0-11");
    Date::from_calendar_date(written.tm_year + 1900, month, written.tm_mday as u8)
        .expect("localtime_r gives a day of its month")
}

/// `serde` for [`crate::moontasks::store::TaskMetadata::appears_on`]: the day as `2026-11-02`.
/// A file with anything else there is refused, like any other unreadable `metadata.json`.
pub(crate) mod in_metadata {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(day: &Option<Date>, out: S) -> Result<S::Ok, S::Error> {
        match day {
            Some(day) => out.serialize_str(&day.to_string()),
            None => out.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        read: D,
    ) -> Result<Option<Date>, D::Error> {
        let day = String::deserialize(read)?;
        parse_day(&day)
            .map(Some)
            .map_err(|error| serde::de::Error::custom(format!("{error:#}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_to_come_until_its_day() {
        let day = parse_day("2026-11-02").expect("expected a day");
        assert_eq!(day.to_string(), "2026-11-02");
        let before = parse_day("2026-11-01").expect("expected a day");

        assert!(still_to_come(Some(day), before));
        assert!(!still_to_come(Some(day), day));
        assert!(!still_to_come(Some(before), day));
        assert!(!still_to_come(None, before));
    }

    #[test]
    fn a_day_is_written_year_month_day() {
        for no_day in ["2026-13-01", "02/11/2026", "tomorrow", "2026-11-2"] {
            let error = parse_day(no_day).expect_err("expected a refusal");
            assert_eq!(
                error.to_string(),
                format!("{no_day:?} is no day: a day is written like 2026-11-02")
            );
        }
    }
}
