//! The small marks a column and its cards wear: the dot that says how a run is doing, the file
//! and chart marks, and the `+` that starts a task.

use egui::{Ui, vec2};

use crate::native::{theme::Palette, widgets};

/// What a run's dot says about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Activity {
    /// Going, and printing: green.
    Running,
    /// Asking for a person - a question, a permission, a bell - and nobody has typed into
    /// it since: red. See [`crate::attention`].
    Attention,
    /// Going, but it has printed nothing for a while - an agent waiting on a question it
    /// asked, or on the person, or stuck: amber, the colour of a thing half done.
    Quiet,
    /// Over: hollow.
    Ended,
}

/// Whether a resource is still going: a filled dot for running, a hollow one for ended.
pub(super) fn running_dot(ui: &mut Ui, running: bool, palette: &Palette) {
    activity_dot(
        ui,
        if running {
            Activity::Running
        } else {
            Activity::Ended
        },
        palette,
    );
}

/// The same dot, with the third state a run can be in - see [`Activity`].
///
/// Drawn rather than typeset, because the bundled fonts have no circle glyph - the system
/// font that a shell's output borrows is not there to fall back on in a snapshot.
pub(super) fn activity_dot(ui: &mut Ui, activity: Activity, palette: &Palette) {
    const DIAMETER: f32 = 7.0;

    let (rect, _) = ui.allocate_exact_size(vec2(DIAMETER, DIAMETER), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let center = rect.center();
    match activity {
        Activity::Running => ui
            .painter()
            .circle_filled(center, DIAMETER / 2.0, palette.added),
        Activity::Quiet => ui
            .painter()
            .circle_filled(center, DIAMETER / 2.0, palette.partial),
        Activity::Attention => ui
            .painter()
            .circle_filled(center, DIAMETER / 2.0, palette.warn),
        Activity::Ended => ui.painter().circle_stroke(
            center,
            DIAMETER / 2.0 - 0.5,
            egui::Stroke::new(1.0, palette.muted),
        ),
    };
}

/// A linked file's mark, in the place a shell's or a run's dot goes: a small page, so the row
/// reads as a file at a glance and lines up with the rows above it.
///
/// Drawn for the same reason the dot is - the bundled fonts have no page glyph either.
pub(super) fn file_mark(ui: &mut Ui, palette: &Palette) {
    const WIDTH: f32 = 7.0;
    const HEIGHT: f32 = 8.0;
    const FOLD: f32 = 2.5;

    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, HEIGHT), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let rect = rect.shrink(0.5);
    let stroke = egui::Stroke::new(1.0, palette.muted);
    // The page: the corner at the top right is folded, so the outline goes round it.
    let outline = [
        rect.left_top(),
        egui::pos2(rect.max.x - FOLD, rect.min.y),
        egui::pos2(rect.max.x, rect.min.y + FOLD),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    ui.painter()
        .add(egui::Shape::line(outline.to_vec(), stroke));
    ui.painter().add(egui::Shape::line(
        vec![
            egui::pos2(rect.max.x - FOLD, rect.min.y),
            egui::pos2(rect.max.x - FOLD, rect.min.y + FOLD),
            egui::pos2(rect.max.x, rect.min.y + FOLD),
        ],
        stroke,
    ));
}

/// A kept visualization's mark, in the place a file's page goes: three bars of a chart, so the
/// row reads as something to look at rather than a file to edit.
pub(super) fn chart_mark(ui: &mut Ui, palette: &Palette) {
    const WIDTH: f32 = 7.0;
    const HEIGHT: f32 = 8.0;
    /// How tall each bar stands, as a share of the mark's height, left to right.
    const BARS: [f32; 3] = [0.5, 1.0, 0.75];

    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, HEIGHT), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let bar_width = rect.width() / BARS.len() as f32;
    for (index, share) in BARS.iter().enumerate() {
        let left = rect.min.x + bar_width * index as f32;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(left + 0.5, rect.max.y - rect.height() * share),
                egui::pos2(left + bar_width - 0.5, rect.max.y),
            ),
            0.0,
            palette.muted,
        );
    }
}

/// A `+` on a filled disc, the same button the tab strips carry for a new tab.
///
/// It is drawn rather than taken from `egui_frames`, which only offers it as part of a tab
/// strip - but it is the same shape, because it means the same thing.
pub(super) fn plus_button(ui: &mut Ui, palette: &Palette) -> egui::Response {
    const DIAMETER: f32 = 15.0;

    let (rect, response) = ui.allocate_exact_size(vec2(DIAMETER, DIAMETER), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, ink) = if response.hovered() {
            (palette.control_active_bg, palette.ink)
        } else {
            (palette.control_bg, palette.muted)
        };
        ui.painter()
            .circle_filled(rect.center(), DIAMETER / 2.0, fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(DIAMETER * 0.72),
            ink,
        );
    }
    widgets::clickable(response)
}
