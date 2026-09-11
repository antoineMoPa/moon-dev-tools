//! Every message the window has posted, and the pane that reads them back.
//!
//! A message is a toast: it appears in the corner, it is up for a few seconds, and then it
//! is gone. That is right for the moment it happens and wrong for a minute later - a message
//! that faded while the person was reading code is a message they never saw, and until now
//! there was nowhere to go and look. This is that somewhere, and it is emacs's `*Messages*`
//! buffer: everything the window has said, oldest at the top, newest at the bottom, with the
//! time it was said.
//!
//! [`crate::native::model::Model::toast`] is the one place a message is born, so it is the
//! one place a message is written down. It de-duplicates the toast - a message repeated is
//! the same message, and stacking copies of it in the corner helps nobody - but the log
//! records every posting, because "this happened four times" is exactly the thing a log is
//! read to find out.

use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use egui::{Align, RichText, Ui};

use crate::native::{
    app::App,
    model::ToastKind,
    theme::{Palette, SMALL_SIZE},
    widgets,
};

/// How many messages are kept. Beyond this the oldest goes.
///
/// A window is open for days and a chatty afternoon posts hundreds of messages, so the log
/// cannot be a `Vec` that only grows. Five hundred is far more than anyone scrolls back
/// through and small enough to be nothing at all in memory - a few tens of kilobytes of
/// short lines.
pub(crate) const KEPT_MESSAGES: usize = 500;

/// How long the status bar reads out the last message after it was posted.
///
/// Long past the toast, which is the point of the bar - it is what is left once the toast in
/// the corner has faded - and short enough that a line from a quarter of an hour ago is not
/// still sitting along the bottom as though it were news. The log keeps it either way.
pub(crate) const STATUS_BAR_READS_A_MESSAGE_FOR: Duration = Duration::from_secs(20);

/// One thing the window said.
pub(crate) struct Message {
    pub(crate) kind: ToastKind,
    pub(crate) text: String,
    /// When it was said, in seconds since the epoch. Seconds rather than an [`Instant`]
    /// because it is read out as a clock time, and an instant has no clock behind it.
    pub(crate) at_unix: u64,
    /// The same moment on the monotonic clock, which is what the status bar measures how long
    /// the message has stood against: a wall clock moved under a running window must neither
    /// bring back an old line nor clear a new one.
    pub(crate) posted_at: Instant,
}

/// The messages the window has posted, newest last, oldest dropped at [`KEPT_MESSAGES`].
#[derive(Default)]
pub(crate) struct MessageLog {
    posted: VecDeque<Message>,
    /// Whether the last message was put away from the status bar by hand. The next message
    /// posted is news again, so recording one clears it.
    latest_dismissed: bool,
}

impl MessageLog {
    /// Write one down. The caller passes the time so the log has no clock of its own to be
    /// tested around.
    pub(crate) fn record(
        &mut self,
        kind: ToastKind,
        text: String,
        at_unix: u64,
        posted_at: Instant,
    ) {
        if self.posted.len() == KEPT_MESSAGES {
            self.posted.pop_front();
        }
        self.posted.push_back(Message {
            kind,
            text,
            at_unix,
            posted_at,
        });
        self.latest_dismissed = false;
    }

    /// The most recent one.
    pub(crate) fn latest(&self) -> Option<&Message> {
        self.posted.back()
    }

    /// The message the status bar reads out at `now`: the most recent one, until it has stood
    /// for [`STATUS_BAR_READS_A_MESSAGE_FOR`] or was dismissed.
    pub(crate) fn standing(&self, now: Instant) -> Option<&Message> {
        if self.latest_dismissed {
            return None;
        }
        self.latest().filter(|latest| {
            now.saturating_duration_since(latest.posted_at) < STATUS_BAR_READS_A_MESSAGE_FOR
        })
    }

    /// Put the last message away from the status bar. It stays in the log.
    pub(crate) fn dismiss_latest(&mut self) {
        self.latest_dismissed = true;
    }

    pub(crate) fn len(&self) -> usize {
        self.posted.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.posted.is_empty()
    }

    /// Oldest first, the way the pane draws them.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Message> {
        self.posted.iter()
    }
}

/// The moment a message is posted at, for [`MessageLog::record`]. A clock that has been set
/// before 1970 is a clock, not a reason to lose the message.
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// The time of day a message was posted, as `14:32:07`.
///
/// UTC, because there is no timezone database in this build and inventing an offset would be
/// worse than being plainly in one zone. What the timestamp is for is telling one message
/// from the one before it and seeing how long ago the run of them was, and it does that in
/// any zone.
pub(crate) fn clock_label(at_unix: u64) -> String {
    let seconds_into_the_day = at_unix % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        seconds_into_the_day / 3_600,
        (seconds_into_the_day % 3_600) / 60,
        seconds_into_the_day % 60
    )
}

/// The log pane: every message, with the newest at the bottom where a log's newest line is.
pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Messages ({})", app.model.messages.len()))
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            widgets::divider(ui, &palette);
            ui.add_space(6.0);

            if app.model.messages.is_empty() {
                ui.label(
                    RichText::new("nothing has been said yet")
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                return;
            }

            // Stuck to the bottom, so opening the pane shows the last thing that happened
            // rather than the first thing that ever did.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for message in app.model.messages.iter() {
                        draw_message(ui, message, &palette);
                    }
                });
        });
}

fn draw_message(ui: &mut Ui, message: &Message, palette: &Palette) {
    // A failure and a note read alike as prose, so the kind is carried by the ink of the
    // text, the way the stripe down a toast carries it.
    let ink = match message.kind {
        ToastKind::Info => palette.ink,
        ToastKind::Error => palette.warn,
    };
    ui.horizontal_top(|ui| {
        ui.with_layout(egui::Layout::left_to_right(Align::TOP), |ui| {
            ui.label(
                RichText::new(clock_label(message.at_unix))
                    .monospace()
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            ui.add(egui::Label::new(RichText::new(&message.text).color(ink)).wrap());
        });
    });
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_drops_the_oldest_message_once_it_is_full() {
        let mut log = MessageLog::default();
        for number in 0..KEPT_MESSAGES + 3 {
            log.record(
                ToastKind::Info,
                format!("message {number}"),
                1_700_000_000,
                Instant::now(),
            );
        }

        assert_eq!(log.len(), KEPT_MESSAGES);
        assert_eq!(
            log.iter().next().expect("expected a first message").text,
            "message 3",
            "the three oldest should have gone"
        );
        assert_eq!(
            log.latest().expect("expected a last message").text,
            format!("message {}", KEPT_MESSAGES + 2)
        );
    }

    #[test]
    fn the_status_bar_reads_a_message_out_until_it_has_stood_long_enough() {
        // Arrange
        let mut log = MessageLog::default();
        let posted_at = Instant::now();
        log.record(ToastKind::Info, "staged".to_string(), 0, posted_at);

        // Act
        let just_after = log.standing(posted_at + Duration::from_secs(1));
        let long_after = log.standing(posted_at + STATUS_BAR_READS_A_MESSAGE_FOR);

        // Assert
        assert_eq!(just_after.expect("expected the message").text, "staged");
        assert!(long_after.is_none(), "the line should have gone by now");
        assert_eq!(log.len(), 1, "the log keeps it all the same");
    }

    #[test]
    fn a_dismissed_message_is_off_the_status_bar_until_the_next_one() {
        // Arrange
        let mut log = MessageLog::default();
        let posted_at = Instant::now();
        log.record(
            ToastKind::Error,
            "failed to stage".to_string(),
            0,
            posted_at,
        );

        // Act
        log.dismiss_latest();
        let dismissed_is_off = log.standing(posted_at).is_none();
        log.record(ToastKind::Info, "staged".to_string(), 0, posted_at);
        let next = log.standing(posted_at);

        // Assert
        assert!(
            dismissed_is_off,
            "a dismissed message should be off the bar"
        );
        assert_eq!(next.expect("expected the next message").text, "staged");
        assert_eq!(log.len(), 2, "dismissing takes nothing out of the log");
    }

    #[test]
    fn a_message_carries_the_time_of_day_it_was_posted_at() {
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(clock_label(1_700_000_000), "22:13:20");
        assert_eq!(clock_label(0), "00:00:00");
    }
}
