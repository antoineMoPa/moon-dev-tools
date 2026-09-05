//! ⌘F: a find bar over whichever pane has the keyboard.
//!
//! One bar, belonging to one pane at a time - the pane that had the keyboard when it opened.
//! What "search" means is the pane's own business: a shell looks through its screen and its
//! scrollback, a review looks through every hunk it is showing rather than only the ones on
//! screen. All this file owns is the query, which match of them is the current one, and the
//! bar the two are typed and stepped through in.

use egui::{Color32, CornerRadius, Key, RichText, Stroke};

use egui_frames::PaneId;

use crate::native::{app::App, model::ToastKind, panes::PaneKind, theme::SMALL_SIZE};

/// The panes that have something for a find bar to look through. The agent monitor is a list
/// of what the agents are doing rather than a document, so ⌘F says so instead of opening a
/// bar that could only ever report nothing - and the board is not here because it answers
/// ⌘F with its own filter instead of a bar.
const SEARCHABLE: &[PaneKind] = &[PaneKind::Review, PaneKind::Terminal, PaneKind::File];

/// The find bar, and the search it is running.
pub(crate) struct Find {
    /// The pane being searched. The bar closes when that pane does.
    pub(crate) pane_id: PaneId,
    pub(crate) query: String,
    /// Which match is the current one, counting from zero.
    pub(crate) at: usize,
    /// How many the pane found last time it looked.
    pub(crate) total: usize,
    /// Set whenever the pane has to act on the search again: the query changed, or the
    /// current match moved. The pane clears it once it has caught up.
    pub(crate) pending: bool,
    /// Set on the frame the bar opens, so the box takes the keyboard without a click.
    focus: bool,
    /// Whether the query box held the keyboard when the bar was last drawn. Escape clears
    /// egui's focus before a frame starts, so the bar cannot ask whether it is being typed
    /// into on the frame that Escape arrives - it asks what was true the frame before.
    has_keyboard: bool,
}

impl Find {
    fn new(pane_id: PaneId) -> Self {
        Self {
            pane_id,
            query: String::new(),
            at: 0,
            total: 0,
            pending: false,
            focus: true,
            has_keyboard: false,
        }
    }

    /// Tell the bar what the pane found. Keeps the current match inside the new count.
    pub(crate) fn found(&mut self, total: usize) {
        self.total = total;
        if self.at >= total {
            self.at = 0;
        }
        self.pending = false;
    }

    fn step(&mut self, forward: bool) {
        if self.total == 0 {
            return;
        }
        self.at = if forward {
            (self.at + 1) % self.total
        } else {
            (self.at + self.total - 1) % self.total
        };
        self.pending = true;
    }
}

/// Open the bar over the pane with the keyboard, or bring it back to the front if it is
/// already open on that pane.
pub(crate) fn open(app: &mut App) {
    let Some(pane_id) = app.active_pane_id() else {
        return;
    };
    // The board has a search of its own - a filter over its cards, standing above them - so
    // cmd+F there puts the keyboard in that box rather than opening a bar over the columns.
    if app.active_pane_kind() == Some(PaneKind::Tasks) {
        app.model.board.filter_focus = true;
        return;
    }
    if !app
        .active_pane_kind()
        .is_some_and(|kind| SEARCHABLE.contains(&kind))
    {
        app.model
            .toast(ToastKind::Info, "there is nothing to search in this tab");
        return;
    }
    match &mut app.model.find {
        Some(find) if find.pane_id == pane_id => {
            find.focus = true;
            find.pending = true;
        }
        slot => *slot = Some(Find::new(pane_id)),
    }
}

/// Open the bar over a file that was opened at one of the lines a content search found: the
/// query is what was searched for, and the current match is the one on that line.
///
/// The box is left without the keyboard, unlike cmd+F: the file was opened to be read, so
/// the text under the bar is what arrow keys and typing belong to. cmd+F puts the keyboard
/// in the box, which is also the way to close the bar with Escape.
pub(crate) fn show_match(app: &mut App, pane_id: PaneId, query: String, at: usize) {
    app.model.find = Some(Find {
        pane_id,
        query,
        at,
        total: 0,
        pending: true,
        focus: false,
        has_keyboard: false,
    });
}

/// Draw the bar in the top right of the pane it belongs to.
pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    let Some(find) = &app.model.find else {
        return;
    };
    // A pane that has been closed takes its find bar with it.
    if !app.model.layout.contains(find.pane_id) {
        app.model.find = None;
        return;
    }
    let Some(rect) = app.pane_rect(find.pane_id) else {
        return;
    };

    let palette = app.palette_of();
    let mut closed = false;
    let mut stepped: Option<bool> = None;
    // Whether the query box is the thing being typed into, this frame or the one before.
    let mut has_keyboard = find.has_keyboard;

    egui::Area::new("moonreview-find".into())
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            rect.max.x - OUTER_WIDTH - INSET,
            rect.min.y + INSET,
        ))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.line))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(BAR_MARGIN as i8, 5))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(50),
                })
                .show(ui, |ui| {
                    ui.set_width(BAR_WIDTH);
                    let find = app.model.find.as_mut().expect("the bar is open");

                    // Laid out from the right: the controls take what they need against
                    // that edge and the box gets the rest, so there is no slack to leave
                    // sitting empty and the buttons stay put as the tally's width changes.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::native::widgets::quiet_button(ui, "\u{00D7}").clicked() {
                            closed = true;
                        }
                        if crate::native::widgets::quiet_button(ui, "\u{203A}").clicked() {
                            stepped = Some(true);
                        }
                        if crate::native::widgets::quiet_button(ui, "\u{2039}").clicked() {
                            stepped = Some(false);
                        }
                        ui.label(
                            RichText::new(tally(find))
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                        );

                        let before = find.query.clone();
                        let entry = ui.add(
                            egui::TextEdit::singleline(&mut find.query)
                                .hint_text("Find")
                                .desired_width(f32::INFINITY)
                                // Enter walks the matches rather than ending the entry, so
                                // the box keeps the keyboard and the next Enter steps again.
                                .return_key(None)
                                .margin(egui::Margin::symmetric(6, 3)),
                        );
                        // A bar that has just asked for the keyboard counts as having it:
                        // the request only lands on the next frame, and until then the box
                        // is where anything typed is going.
                        find.has_keyboard = if std::mem::take(&mut find.focus) {
                            entry.request_focus();
                            true
                        } else {
                            entry.has_focus()
                        };
                        has_keyboard |= find.has_keyboard;
                        if find.query != before {
                            // A changed query starts again from the first match.
                            find.at = 0;
                            find.pending = true;
                        }
                    });
                });
        });

    // Enter walks the matches and Escape puts the bar away, both of which belong to the bar
    // rather than to the pane under it, so they are read here instead of in the key table.
    // Only while the query box holds the keyboard, though: an Enter meant for the shell or
    // the file under the bar is the pane's, and stepping the search on it would move the
    // window out from under whoever typed it.
    let (next, previous, dismiss) = ctx.input_mut(|input| {
        if !has_keyboard {
            return (false, false, false);
        }
        (
            input.consume_key(egui::Modifiers::NONE, Key::Enter),
            input.consume_key(egui::Modifiers::SHIFT, Key::Enter),
            input.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if next || previous {
        stepped = Some(next);
    }
    if dismiss || closed {
        app.model.find = None;
        return;
    }
    if let Some(forward) = stepped
        && let Some(find) = &mut app.model.find
    {
        find.step(forward);
    }
}

/// Wide enough for a query worth typing and the controls beside it, and no wider: the box
/// takes whatever the controls leave, so nothing here is padding.
const BAR_WIDTH: f32 = 300.0;
const BAR_MARGIN: f32 = 8.0;
/// What the bar occupies on screen, which is what it has to be placed by - the margin sits
/// outside the width the contents were given.
const OUTER_WIDTH: f32 = BAR_WIDTH + BAR_MARGIN * 2.0;
/// How far the bar sits from the corner of the pane it belongs to.
const INSET: f32 = 10.0;

fn tally(find: &Find) -> String {
    if find.query.is_empty() {
        return String::new();
    }
    if find.total == 0 {
        return "none".to_string();
    }
    format!("{} of {}", find.at + 1, find.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane to hang a bar on. Names come from an arrangement, so the test builds one.
    fn a_pane() -> PaneId {
        egui_frames::Layout::with_pane(())
            .active_pane()
            .expect("expected the pane just opened")
            .0
    }

    fn find_with(total: usize) -> Find {
        let mut find = Find::new(a_pane());
        find.query = "x".to_string();
        find.found(total);
        find
    }

    #[test]
    fn stepping_wraps_round_the_matches_both_ways() {
        let mut find = find_with(3);
        assert_eq!(find.at, 0);

        find.step(true);
        find.step(true);
        assert_eq!(find.at, 2);
        find.step(true);
        assert_eq!(find.at, 0, "past the last match is the first one again");

        find.step(false);
        assert_eq!(find.at, 2, "and before the first is the last");
    }

    #[test]
    fn stepping_with_nothing_found_does_nothing() {
        let mut find = find_with(0);
        find.step(true);

        assert_eq!(find.at, 0);
        assert!(!find.pending, "there is nothing for the pane to go to");
    }

    /// Typing narrows a search down. The match that was current can stop existing, and the
    /// bar has to come back to one that does rather than point past the end.
    #[test]
    fn a_search_that_finds_fewer_matches_pulls_the_current_one_back() {
        let mut find = find_with(5);
        find.step(true);
        find.step(true);
        assert_eq!(find.at, 2);

        find.found(2);
        assert_eq!(find.at, 0);
        assert_eq!(find.total, 2);
    }

    #[test]
    fn the_tally_says_where_in_the_matches_the_bar_is() {
        assert_eq!(tally(&find_with(0)), "none");

        let mut find = find_with(4);
        assert_eq!(tally(&find), "1 of 4");
        find.step(true);
        assert_eq!(tally(&find), "2 of 4");

        find.query.clear();
        assert_eq!(tally(&find), "", "an empty query is not a failed search");
    }
}
