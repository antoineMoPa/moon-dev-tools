//! How the board moves: the one axis a scrolling gesture is let through on, and the slide that
//! walks a card or a column to a new place instead of jumping it there.

use egui::{Ui, vec2};

use crate::native::app::App;

/// The part of this frame's scroll the board is not letting through, taken out of the input
/// while the columns draw and put back afterwards.
///
/// The board is a horizontal scroll area with a vertical one in every column, and egui hands
/// the same delta to both, so an unlocked gesture scrolls the board and a column at once. The
/// axis is settled on the first frame of a gesture and kept until the scrolling stops.
pub(super) struct HeldBack(egui::Vec2);

impl HeldBack {
    /// Put the held part back, so panes drawn after the board see the gesture as it came in.
    pub(super) fn give_it_back(self, ui: &Ui) {
        ui.input_mut(|input| input.smooth_scroll_delta += self.0);
    }
}

/// Which way a gesture is moving the board: the axis it was already settled on, or - on the
/// first frame of one - whichever of the two the delta is mostly along. Nothing while nothing
/// is scrolling, which is what ends a gesture and lets the next one pick for itself.
///
/// Sticky on purpose: fingers drift on a trackpad, and an axis worked out afresh every frame
/// would swap halfway through a flick.
fn scroll_axis(settled: Option<Axis>, along: egui::Vec2) -> Option<Axis> {
    if along == egui::Vec2::ZERO {
        return None;
    }
    Some(settled.unwrap_or(if along.x.abs() > along.y.abs() {
        Axis::Horizontal
    } else {
        Axis::Vertical
    }))
}

pub(super) fn hold_the_off_axis(app: &mut App, ui: &Ui) -> HeldBack {
    let along = ui.input(|input| input.smooth_scroll_delta);
    let Some(axis) = scroll_axis(app.model.board.scroll_axis, along) else {
        // The gesture is over, and the next one picks its own axis.
        app.model.board.scroll_axis = None;
        return HeldBack(egui::Vec2::ZERO);
    };
    app.model.board.scroll_axis = Some(axis);

    // What the columns are allowed to see is the chosen axis alone; the rest is held here.
    let held = HeldBack(match axis {
        Axis::Horizontal => vec2(0.0, along.y),
        Axis::Vertical => vec2(along.x, 0.0),
    });
    ui.input_mut(|input| input.smooth_scroll_delta -= held.0);
    held
}

/// How long a card takes to walk to a new place in its column. Long enough to be followed by
/// eye, short enough that the board is never waiting on it.
const CARD_SLIDE: f32 = 0.12;

/// Which way a run of things is laid out, and so which way one of them slides to a new place.
///
/// Cards stack down a column and columns run across the board, and both make room for one
/// being dragged in exactly the same way - so the animation is written once and told which
/// axis it is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    /// Where the next thing will be laid out, along this axis.
    fn cursor_start(self, ui: &Ui) -> f32 {
        match self {
            Self::Vertical => ui.cursor().top(),
            Self::Horizontal => ui.cursor().left(),
        }
    }

    fn offset(self, along: f32) -> egui::Vec2 {
        match self {
            Self::Vertical => vec2(0.0, along),
            Self::Horizontal => vec2(along, 0.0),
        }
    }
}

/// Draw something at the place the layout gives it, moving there from wherever it was drawn
/// last rather than appearing there.
///
/// This is what makes the things a dragged one is being put between move out of its way
/// instead of jumping: the layout answers where each belongs, and this walks it there over
/// [`CARD_SLIDE`]. One that has not moved is drawn where it is with no work done, and one
/// drawn for the first time starts where it belongs rather than sliding in from the edge.
///
/// `origin` is what the place is measured from - the top of the column's contents for a card,
/// the left of the row for a column - so that scrolling, which moves everything at once, is
/// not read as everything having moved.
pub(super) fn slide_into_place(
    ui: &mut Ui,
    axis: Axis,
    id: egui::Id,
    origin: f32,
    draw: impl FnOnce(&mut Ui) -> egui::Rect,
) -> egui::Rect {
    let belongs_at = axis.cursor_start(ui) - origin;
    let drawn_at = ui
        .ctx()
        .animate_value_with_time(id.with("slide"), belongs_at, CARD_SLIDE);
    let offset = axis.offset(drawn_at - belongs_at);
    if offset.length() < 0.5 {
        return draw(ui);
    }

    // Drawn into a layer of its own so the shapes can be moved once they are made - the same
    // way the dragged card is. Its clip is moved the other way first, so one on its way between
    // two places is still cut off at the pane it is in rather than drawn over what is beside it.
    let layer_id = egui::LayerId::new(egui::Order::Middle, id.with("sliding"));
    let clip = ui.clip_rect();
    let rect = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
            ui.set_clip_rect(clip.translate(-offset));
            draw(ui)
        })
        .inner;
    ui.ctx()
        .transform_layer_shapes(layer_id, egui::emath::TSTransform::from_translation(offset));
    rect
}

/// Say that something is at the place the layout gives it right now, without walking there.
///
/// One that is somewhere for a reason of its own - carried by the cursor - is still at a
/// place, and the next thing to draw it has to know that place is where it already is.
pub(super) fn stamp_place(ui: &Ui, axis: Axis, id: egui::Id, origin: f32) {
    ui.ctx()
        .animate_value_with_time(id.with("slide"), axis.cursor_start(ui) - origin, 0.0);
}

#[cfg(test)]
mod tests {
    use super::{Axis, scroll_axis};
    use egui::vec2;

    /// A gesture picks its axis once and keeps it, so a flick that starts across the board and
    /// drifts downwards goes on moving the board across rather than turning into a column
    /// scroll partway through.
    #[test]
    fn a_scrolling_gesture_keeps_the_axis_it_started_on() {
        assert_eq!(
            scroll_axis(None, vec2(0.0, 0.0)),
            None,
            "nothing is scrolling"
        );
        assert_eq!(
            scroll_axis(None, vec2(-30.0, 4.0)),
            Some(Axis::Horizontal),
            "mostly sideways is the board moving across"
        );
        assert_eq!(
            scroll_axis(None, vec2(4.0, -30.0)),
            Some(Axis::Vertical),
            "mostly up and down is a column"
        );
        assert_eq!(
            scroll_axis(Some(Axis::Horizontal), vec2(2.0, -30.0)),
            Some(Axis::Horizontal),
            "the drift of a gesture already under way does not turn it"
        );
        assert_eq!(
            scroll_axis(Some(Axis::Horizontal), vec2(0.0, 0.0)),
            None,
            "the gesture ends when the scrolling stops, and the next one picks again"
        );
    }
}
