//! The squares at the right of the status bar, one per space, the one the window is on
//! picked out; a click on another goes there.
//!
//! The square is the ground color of the space's project with its arrangement drawn small on
//! it: a block per frame, laid out the way the frames are. That is enough to tell one space
//! from another before reading a name - two on the same project are told apart by how they
//! are split.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_frames::{FrameId, LayoutNode, SplitDirection};

use crate::native::{
    app::{App, SpaceView},
    palette::CommandAction,
    theme::{Palette, ThemeMode},
    widgets,
};

/// The square in the tab strip.
const STRIP_SQUARE: egui::Vec2 = vec2(22.0, 16.0);
/// The gap between two blocks of an arrangement, in points.
const BLOCK_GAP: f32 = 1.5;
/// Where the squares stop being drawn, since a block narrower than this is not one.
const SMALLEST_BLOCK: f32 = 2.0;

/// How long a square is held, without moving, before it lifts off the strip to be dragged.
const LONG_PRESS: f64 = 0.35;
/// How far the pointer may wander during that time and still be holding.
const HOLD_SLACK: f32 = 5.0;
/// How long a square takes to slide to its place, in seconds.
const SLIDE: f32 = 0.12;

/// A square lifted off the strip by a long press, and following the pointer.
#[derive(Clone, Copy, Default)]
struct Lift {
    /// Which space it is, by its place in the list before the drag.
    index: usize,
    /// How far in from the square's left edge the pointer took hold of it.
    grab: f32,
}

fn lift_id() -> egui::Id {
    egui::Id::new("space-lift")
}

fn slide_id(index: usize) -> egui::Id {
    egui::Id::new(("space-slide", index))
}

/// The row of squares, one per space, drawn in the status bar's strip. Clicking one goes to
/// that space; right-clicking one offers to empty it; holding one lifts it, and it can be
/// dragged to another place in the list.
///
/// The status bar is drawn outside the workspace, so every space's arrangement is there to
/// read - inside it, the one in front is lent out to the frames.
pub(crate) fn draw(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let ctx = ui.ctx().clone();
    let mode = app.model.theme;
    let front = app.spaces.front();
    let views = app.space_views();
    let count = views.len();
    let pitch = STRIP_SQUARE.x + ui.spacing().item_spacing.x;

    let (strip, _) = ui.allocate_exact_size(
        vec2(pitch * count as f32 - ui.spacing().item_spacing.x, STRIP_SQUARE.y),
        Sense::hover(),
    );
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let lifted: Option<Lift> = ctx.data(|data| data.get_temp(lift_id()));

    // Where the lifted square would land if let go now, and so where the others make room.
    let landing = lifted.zip(pointer).map(|(lift, at)| {
        let center = at.x - lift.grab + STRIP_SQUARE.x / 2.0 - strip.min.x;
        ((center / pitch).floor().max(0.0) as usize).min(count - 1)
    });
    // The place in the strip each space is shown in.
    let slot_of = |index: usize| -> usize {
        match (lifted, landing) {
            (Some(lift), Some(landing)) if index != lift.index => {
                let without: Vec<usize> = (0..count).filter(|other| *other != lift.index).collect();
                let mut order = without;
                order.insert(landing, lift.index);
                order.iter().position(|other| *other == index).expect("every space has a place")
            }
            (Some(_), Some(landing)) => landing,
            _ => index,
        }
    };

    let mut picked: Option<CommandAction> = None;
    let mut started: Option<Lift> = None;
    // The squares as they are shown this frame, the lifted one last so it is over the others.
    let mut order: Vec<usize> = (0..count).collect();
    if let Some(lift) = lifted {
        order.retain(|index| *index != lift.index);
        order.push(lift.index);
    }
    let mut shown_at: Vec<f32> = vec![0.0; count];
    for index in order {
        let space = &views[index];
        let is_lifted = lifted.is_some_and(|lift| lift.index == index);
        let target = strip.min.x + slot_of(index) as f32 * pitch;
        let x = match (is_lifted, lifted, pointer) {
            // Held to the pointer, with no easing: it is in the hand.
            (true, Some(lift), Some(at)) => {
                let x = (at.x - lift.grab).clamp(strip.min.x, strip.max.x - STRIP_SQUARE.x);
                ctx.animate_value_with_time(slide_id(index), x, 0.0)
            }
            _ => ctx.animate_value_with_time(slide_id(index), target, SLIDE),
        };
        shown_at[index] = x;
        let rect = Rect::from_min_size(pos2(x, strip.min.y), STRIP_SQUARE);
        let response = ui
            .interact(rect, egui::Id::new(("space-square", index)), Sense::click_and_drag());
        let response = widgets::clickable(response);
        draw_square(ui, rect, space, palette, mode, index == front, response.hovered() || is_lifted);
        if space.wants_attention {
            ui.painter()
                .circle_filled(rect.right_top() + vec2(-1.0, 1.0), 3.0, palette.warn);
        }
        let response = response.on_hover_text(if space.is_untouched() {
            format!("space {}: empty - opens on {}", index + 1, space.name())
        } else {
            format!("space {}: {}", index + 1, space.name())
        });

        // A hold that has not moved lifts the square.
        if lifted.is_none() && response.is_pointer_button_down_on() {
            let held = ui.input(|input| {
                let since = input.pointer.press_start_time()?;
                let origin = input.pointer.press_origin()?;
                let now = input.pointer.latest_pos()?;
                (now.distance(origin) <= HOLD_SLACK).then_some((input.time - since, origin))
            });
            match held {
                Some((seconds, origin)) if seconds >= LONG_PRESS => {
                    started = Some(Lift {
                        index,
                        grab: origin.x - rect.min.x,
                    });
                }
                Some((seconds, _)) => ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    LONG_PRESS - seconds,
                )),
                None => {}
            }
        }
        if response.clicked() && lifted.is_none() {
            picked = Some(CommandAction::GoToSpace(index));
        }
        if !space.is_untouched() && lifted.is_none() {
            response.context_menu(|ui| {
                if widgets::clickable(ui.button("close space"))
                    .on_hover_text("Empty this space; its shells keep running")
                    .clicked()
                {
                    picked = Some(CommandAction::CloseSpaceAt(index));
                    ui.close();
                }
            });
        }
    }

    if let Some(lift) = started {
        ctx.data_mut(|data| data.insert_temp(lift_id(), lift));
        ctx.request_repaint();
    }
    if let (Some(lift), Some(landing)) = (lifted, landing) {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        if !ui.input(|input| input.pointer.any_down()) {
            // Let go: the squares are where the list will have them, so each one goes on from
            // where it is shown rather than jumping to where its old place was.
            ctx.data_mut(|data| data.remove_temp::<Lift>(lift_id()));
            if landing != lift.index {
                let mut new_places = vec![0.0; count];
                for index in 0..count {
                    new_places[slot_of(index)] = shown_at[index];
                }
                for (place, x) in new_places.into_iter().enumerate() {
                    ctx.animate_value_with_time(slide_id(place), x, 0.0);
                }
                picked = Some(CommandAction::MoveSpace {
                    from: lift.index,
                    to: landing,
                });
            }
        }
    }

    drop(views);
    if let Some(action) = picked {
        // Deferred with the rest of the window's own actions: switching swaps the app that is
        // drawing this.
        app.pending_action = Some(action);
    }
}

/// A space's preview: one small rectangle per frame of its arrangement, laid out the way the
/// frames are, each in the ground color of the project. The space the window is on has its
/// rectangles outlined in the accent, and - when it is split - the frame the keyboard is in
/// filled with it.
fn draw_square(
    ui: &Ui,
    rect: Rect,
    space: &SpaceView<'_>,
    palette: &Palette,
    mode: ThemeMode,
    in_front: bool,
    hovered: bool,
) {
    let outline = Stroke::new(
        if in_front { 1.5 } else { 1.0 },
        if in_front {
            palette.accent
        } else if hovered {
            palette.ink
        } else {
            palette.muted
        },
    );
    let ground = space.color.bg(mode);
    let Some(arrangement) = space.arrangement else {
        // Nothing to tile yet: one block, the space as it will open.
        ui.painter().rect(
            rect,
            CornerRadius::same(2),
            ground,
            Stroke::new(1.0, palette.muted.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
        return;
    };
    let active = (in_front && arrangement.frame_count() > 1).then(|| arrangement.active_frame());
    let blocks = Blocks {
        painter: ui.painter(),
        ground,
        outline,
        active_fill: palette.accent.gamma_multiply(0.6),
    };
    blocks.draw(rect, arrangement.root(), active);
}

/// What the rectangles of one preview are painted with.
struct Blocks<'a> {
    painter: &'a egui::Painter,
    ground: Color32,
    outline: Stroke,
    active_fill: Color32,
}

impl Blocks<'_> {
    /// Divide `area` among the children of a split the way the frames divide the workspace,
    /// and paint each frame's share.
    fn draw(&self, area: Rect, node: &LayoutNode, active: Option<FrameId>) {
        match node {
            LayoutNode::Frame { frame } => {
                let block = area.shrink(BLOCK_GAP / 2.0);
                if block.width() < SMALLEST_BLOCK || block.height() < SMALLEST_BLOCK {
                    return;
                }
                let fill = if Some(*frame) == active {
                    self.active_fill
                } else {
                    self.ground
                };
                self.painter
                    .rect(block, CornerRadius::same(1), fill, self.outline, StrokeKind::Inside);
            }
            LayoutNode::Split {
                direction,
                children,
                sizes,
            } => {
                let mut offset = 0.0;
                for (child, share) in children.iter().zip(sizes) {
                    let part = match direction {
                        SplitDirection::Row => Rect::from_min_size(
                            pos2(area.min.x + offset * area.width(), area.min.y),
                            vec2(share * area.width(), area.height()),
                        ),
                        SplitDirection::Column => Rect::from_min_size(
                            pos2(area.min.x, area.min.y + offset * area.height()),
                            vec2(area.width(), share * area.height()),
                        ),
                    };
                    offset += share;
                    self.draw(part, child, active);
                }
            }
        }
    }
}
