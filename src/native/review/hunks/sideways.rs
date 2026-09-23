//! Scrolling a hunk's code sideways, so a line longer than the card is wide can be read to its
//! end instead of stopping at the card's edge.
//!
//! Each hunk scrolls on its own: one generated line should not drag every other hunk of the
//! review off to the right with it. The line numbers and the `+`/`-` marker stay where they
//! are; only the code slides under them. A sideways swipe over the card scrolls it - on a
//! trackpad that is the swipe, on a mouse the wheel with shift held - and so does dragging the
//! bar along the card's bottom, which shows while the pointer is over a card that overflows.

use egui::{CornerRadius, Rect, Sense, Ui, pos2, vec2};

use crate::native::{
    app::App,
    theme::{CODE_SIZE, Palette},
};

use super::code_rect;

/// How tall the bar along a card's bottom is. A hunk that overflows leaves this much room
/// under its last row, so the bar never sits over code.
pub(super) const BAR_HEIGHT: f32 = 6.0;
/// The thumb never gets narrower than this, however long the line: it has to stay grabbable.
const MIN_THUMB_WIDTH: f32 = 24.0;

/// How far a hunk's code is scrolled sideways, in points.
pub(super) fn scroll_x(app: &App, session_id: &str, hunk_id: &str) -> f32 {
    app.model
        .review_ref(session_id)
        .and_then(|review| review.code_scroll_x.get(hunk_id))
        .copied()
        .unwrap_or(0.0)
}

/// How far past the edge of rows `row_width` wide a hunk's longest line runs, in points.
/// Nothing when all of it fits. Asked after the hunk's lines are built.
pub(super) fn overflow(app: &App, ui: &Ui, hunk_id: &str, row_width: f32) -> f32 {
    let code = code_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(row_width, 0.0)));
    (content_width(app, ui, hunk_id) - code.width()).max(0.0)
}

/// How wide the longest line of a hunk is laid out: one column more than it holds, so its last
/// character is not flush with the card's edge when scrolled all the way.
fn content_width(app: &App, ui: &Ui, hunk_id: &str) -> f32 {
    let advance = ui.fonts_mut(|fonts| fonts.glyph_width(&egui::FontId::monospace(CODE_SIZE), 'm'));
    (app.widest_diff_body(hunk_id) + 1) as f32 * advance
}

/// After a hunk's card of code is drawn: take a sideways scroll over it, and the drag of the
/// bar along its bottom. `card` is the card as it was drawn. What this changes is drawn on the
/// next frame, which it asks for.
pub(super) fn scroll_card(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk_id: &str,
    card: &egui::Response,
    palette: &Palette,
) {
    // The rows run inside the card's one-point margin, so that is where their code starts.
    let code = code_rect(card.rect.shrink(1.0));
    let overflow = overflow(app, ui, hunk_id, card.rect.shrink(1.0).width());
    let content_width = content_width(app, ui, hunk_id);

    let before = scroll_x(app, session_id, hunk_id);
    // The card may have got wider since it was scrolled - the window, or the pane - and a
    // scroll past the end of the longest line would show nothing but the card's background.
    let mut after = before.min(overflow);

    if overflow > 0.0 && card.contains_pointer() {
        let swipe = ui.input(|input| input.smooth_scroll_delta.x);
        if swipe != 0.0 {
            after = (after - swipe).clamp(0.0, overflow);
            // Taken, so the pane around the card does not also act on it.
            ui.input_mut(|input| input.smooth_scroll_delta.x = 0.0);
        }
    }

    if overflow > 0.0 {
        after = drag_bar(
            ui,
            hunk_id,
            card,
            code,
            after,
            overflow,
            content_width,
            palette,
        );
    }

    if after == before {
        return;
    }
    let review = app.model.review(session_id);
    if after == 0.0 {
        review.code_scroll_x.remove(hunk_id);
    } else {
        review.code_scroll_x.insert(hunk_id.to_string(), after);
    }
    ui.ctx().request_repaint();
}

/// The bar along the bottom of a card that overflows, under the code: where the code is
/// scrolled to, and a thumb to drag it by. Answers the scroll after the drag.
#[allow(
    clippy::too_many_arguments,
    reason = "one call site; the alternative is a \
    parameter struct that only exists to be destructured immediately"
)]
fn drag_bar(
    ui: &mut Ui,
    hunk_id: &str,
    card: &egui::Response,
    code: Rect,
    scroll_x: f32,
    overflow: f32,
    content_width: f32,
    palette: &Palette,
) -> f32 {
    let track = Rect::from_min_max(
        pos2(code.min.x, code.max.y - BAR_HEIGHT),
        pos2(code.max.x, code.max.y),
    );
    let thumb_width = (track.width() * track.width() / content_width)
        .max(MIN_THUMB_WIDTH)
        .min(track.width());
    let travel = track.width() - thumb_width;

    let bar = ui.interact(
        track,
        egui::Id::new(("moonreview-code-scroll-bar", hunk_id)),
        Sense::drag(),
    );
    let mut scrolled = scroll_x;
    if bar.dragged() && travel > 0.0 {
        scrolled = (scrolled + bar.drag_delta().x * overflow / travel).clamp(0.0, overflow);
    }

    // Only while the pointer is on the card, or dragging it: a bar under every long hunk all
    // the time is clutter, and it is the card being read that it is about.
    if !card.contains_pointer() && !bar.dragged() {
        return scrolled;
    }
    let thumb = Rect::from_min_size(
        pos2(
            track.min.x + travel * scrolled / overflow,
            track.min.y + 1.0,
        ),
        vec2(thumb_width, BAR_HEIGHT - 2.0),
    );
    let ink = if bar.dragged() || bar.hovered() {
        palette.ink
    } else {
        palette.muted
    };
    ui.painter().rect_filled(thumb, CornerRadius::same(2), ink);
    scrolled
}
