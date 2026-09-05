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
    time::{SystemTime, UNIX_EPOCH},
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

/// One thing the window said.
pub(crate) struct Message {
    pub(crate) kind: ToastKind,
    pub(crate) text: String,
    /// When it was said, in seconds since the epoch. Seconds rather than an [`std::time::Instant`]
    /// because it is read out as a clock time, and an instant has no clock behind it.
    pub(crate) at_unix: u64,
}

/// The messages the window has posted, newest last, oldest dropped at [`KEPT_MESSAGES`].
#[derive(Default)]
pub(crate) struct MessageLog {
    posted: VecDeque<Message>,
}

impl MessageLog {
    /// Write one down. The caller passes the time so the log has no clock of its own to be
    /// tested around.
    pub(crate) fn record(&mut self, kind: ToastKind, text: String, at_unix: u64) {
        if self.posted.len() == KEPT_MESSAGES {
            self.posted.pop_front();
        }
        self.posted.push_back(Message {
            kind,
            text,
            at_unix,
        });
    }

    /// The most recent one, which is what the status bar reads out when nothing is working.
    pub(crate) fn latest(&self) -> Option<&Message> {
        self.posted.back()
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
            log.record(ToastKind::Info, format!("message {number}"), 1_700_000_000);
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
    fn a_message_carries_the_time_of_day_it_was_posted_at() {
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(clock_label(1_700_000_000), "22:13:20");
        assert_eq!(clock_label(0), "00:00:00");
    }
}
