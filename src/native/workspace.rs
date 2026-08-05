//! The pane arrangement on screen: nested splits, a tab strip per frame, draggable tabs.
//!
//! [`crate::native::layout`] owns what the arrangement *is*; this file owns how it is drawn
//! and how pointer gestures turn into the next arrangement.

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use crate::native::{
    app::{App, TerminalPlacement},
    layout::{self, DropSide, LayoutNode, Pane, SplitDirection, WorkspaceLayout},
    theme::{self, Palette, SMALL_SIZE},
    widgets,
};

/// The moon, drawn: a filled disc with a second disc punched out of it in the panel color.
/// This is the app's mark, and it must not depend on an emoji font having a glyph for it.
fn draw_moon(painter: &egui::Painter, center: egui::Pos2, radius: f32, ink: Color32, behind: Color32) {
    painter.circle_filled(center, radius, ink);
    painter.circle_filled(center + vec2(radius * 0.55, -radius * 0.3), radius * 0.85, behind);
}

/// The light/dark switch: a moon in light mode, a sun in dark mode.
fn theme_switch(ui: &mut Ui, theme: crate::native::theme::ThemeMode, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(17.0, 15.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let ink = if response.hovered() {
        palette.accent
    } else {
        palette.muted
    };
    let center = rect.center();
    match theme {
        crate::native::theme::ThemeMode::Light => {
            draw_moon(ui.painter(), center, 5.5, ink, palette.header_bg);
        }
        crate::native::theme::ThemeMode::Dark => {
            ui.painter().circle_filled(center, 3.5, ink);
            for step in 0..8 {
                let angle = std::f32::consts::TAU * step as f32 / 8.0;
                let direction = vec2(angle.cos(), angle.sin());
                ui.painter().line_segment(
                    [center + direction * 5.0, center + direction * 7.0],
                    Stroke::new(1.0, ink),
                );
            }
        }
    }
    response
}

/// The frame's border, one pixel drawn inside its rect.
const FRAME_BORDER: f32 = 1.0;
/// The gap between a tab and the strip's edges — the same on all four sides.
const TAB_MARGIN: f32 = 4.0;
const TAB_HEIGHT: f32 = 18.0;
const TAB_STRIP_HEIGHT: f32 = FRAME_BORDER + TAB_MARGIN * 2.0 + TAB_HEIGHT;
/// Tab padding: text starts here, and the close mark sits this far from the right edge.
const TAB_TEXT_INSET: f32 = 8.0;
const TAB_CLOSE_SIZE: f32 = 12.0;
const TAB_CLOSE_INSET: f32 = 4.0;
/// Space between the end of the title and the close mark.
const TAB_CLOSE_GAP: f32 = 5.0;
/// Space between one tab and the next.
const TAB_GAP: f32 = 3.0;
const DIVIDER_THICKNESS: f32 = 5.0;
/// The narrowest a frame may be left at by opening a shell beside it. Below this, the shell
/// joins a frame's tabs instead of taking a column of its own.
const MIN_COLUMN_WIDTH: f32 = 320.0;
/// How close to a frame's left or right edge a dropped tab has to land to split it there,
/// as a share of the frame's width.
const SIDE_EDGE_FRACTION: f32 = 0.22;
/// The same for the top and bottom edges, as a share of the frame's body. Deeper than the sides:
/// a frame is wider than it is tall, so an equal share of the height is a much shorter band, and
/// reaching a bottom split meant dragging all the way to the window's edge.
const UP_DOWN_EDGE_FRACTION: f32 = 0.38;

impl App {
    pub(crate) fn draw_workspace(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        // Frames and tabs record where they were drawn, so a drop lands on what the user was
        // actually looking at rather than on geometry recomputed from the tree.
        self.frame_rects.clear();
        self.tab_rects.clear();

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ui, |ui| {
                let root = ui.available_rect_before_wrap();
                let node = self.model.layout.root.clone();
                self.draw_node(ui, &node, root, &[], &palette);
            });

        // A drag that ends anywhere resolves here, so releasing outside a frame simply
        // cancels rather than leaving the tab stuck to the pointer.
        let released = ui.ctx().input(|input| input.pointer.any_released());
        if self.model.dragging_pane.is_some() && released {
            let at = ui.ctx().input(|input| input.pointer.latest_pos());
            self.finish_tab_drag(at);
        }
    }

    fn draw_node(
        &mut self,
        ui: &mut Ui,
        node: &LayoutNode,
        rect: Rect,
        path: &[usize],
        palette: &Palette,
    ) {
        match node {
            LayoutNode::Frame { frame_id } => {
                let frame_id = frame_id.clone();
                self.draw_frame(ui, &frame_id, rect, palette);
            }
            LayoutNode::Split {
                direction,
                children,
                sizes,
            } => {
                let horizontal = *direction == SplitDirection::Row;
                let (child_rects, usable) =
                    split_child_rects(rect, *direction, sizes, children.len());
                let mut resized: Option<Vec<f32>> = None;

                for (index, child) in children.iter().enumerate() {
                    let child_rect = child_rects[index];
                    let mut child_path = path.to_vec();
                    child_path.push(index);
                    self.draw_node(ui, child, child_rect, &child_path, palette);

                    if index + 1 < children.len() {
                        let divider_rect = if horizontal {
                            Rect::from_min_size(
                                pos2(child_rect.max.x, rect.min.y),
                                vec2(DIVIDER_THICKNESS, rect.height()),
                            )
                        } else {
                            Rect::from_min_size(
                                pos2(rect.min.x, child_rect.max.y),
                                vec2(rect.width(), DIVIDER_THICKNESS),
                            )
                        };
                        if let Some(next) = self.draw_divider(
                            ui,
                            divider_rect,
                            horizontal,
                            index,
                            sizes,
                            usable,
                            palette,
                        ) {
                            resized = Some(next);
                        }
                    }
                }

                if let Some(next_sizes) = resized {
                    let root = std::mem::replace(
                        &mut self.model.layout.root,
                        LayoutNode::Frame {
                            frame_id: String::new(),
                        },
                    );
                    self.model.layout.root = layout::set_split_sizes(root, path, &next_sizes);
                }
            }
        }
    }

    /// The grab handle between two panes. Returns the split's new sizes while being dragged.
    fn draw_divider(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        horizontal: bool,
        index: usize,
        sizes: &[f32],
        usable: f32,
        palette: &Palette,
    ) -> Option<Vec<f32>> {
        let id = Id::new(("workspace-divider", rect.min.x as i32, rect.min.y as i32, index));
        let response = ui.interact(rect, id, Sense::drag());
        let cursor = if horizontal {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        };
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
            ui.painter().rect_filled(
                rect.shrink2(if horizontal {
                    vec2(1.5, 0.0)
                } else {
                    vec2(0.0, 1.5)
                }),
                CornerRadius::same(1),
                palette.accent,
            );
        }

        if !response.dragged() {
            return None;
        }
        let delta = if horizontal {
            response.drag_delta().x
        } else {
            response.drag_delta().y
        };
        if delta.abs() < f32::EPSILON || usable <= 0.0 {
            return None;
        }

        // Dragging a handle trades space between the two panes it sits between, leaving
        // every other pane in the split alone.
        let shift = delta / usable;
        let mut next = sizes.to_vec();
        if index + 1 >= next.len() {
            return None;
        }
        next[index] += shift;
        next[index + 1] -= shift;
        Some(next)
    }

    fn draw_frame(&mut self, ui: &mut Ui, frame_id: &str, rect: Rect, palette: &Palette) {
        self.frame_rects.push((frame_id.to_string(), rect));
        let is_active = self.model.layout.active_frame_id == frame_id;
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(6), palette.panel);
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(
                1.0,
                if is_active { palette.accent } else { palette.line },
            ),
            StrokeKind::Inside,
        );

        let strip_rect = Rect::from_min_size(rect.min, vec2(rect.width(), TAB_STRIP_HEIGHT));
        let body_rect = Rect::from_min_max(
            pos2(rect.min.x, rect.min.y + TAB_STRIP_HEIGHT),
            rect.max,
        );

        // Tabs sit inside the frame's border with the same margin on every side, so the strip
        // reads as an even band rather than a row pushed against the top-left corner.
        let tabs_rect = Rect::from_min_max(
            pos2(
                strip_rect.min.x + FRAME_BORDER + TAB_MARGIN,
                strip_rect.min.y + FRAME_BORDER + TAB_MARGIN,
            ),
            pos2(
                strip_rect.max.x - FRAME_BORDER - TAB_MARGIN,
                strip_rect.max.y - TAB_MARGIN,
            ),
        );
        let is_primary = self.model.layout.primary_frame_id() == frame_id;
        ui.scope_builder(UiBuilder::new().max_rect(tabs_rect), |ui| {
            ui.set_clip_rect(strip_rect);
            self.draw_tab_strip(ui, frame_id, is_primary, palette);
        });

        // Stop short of the frame's border on both sides: the border, active or not, stays the
        // outermost thing drawn on the frame.
        ui.painter().hline(
            (rect.min.x + FRAME_BORDER)..=(rect.max.x - FRAME_BORDER),
            strip_rect.max.y,
            Stroke::new(1.0, palette.line),
        );

        let active_pane = self
            .model
            .layout
            .frames
            .get(frame_id)
            .and_then(|frame| frame.active_pane_id.clone())
            .and_then(|pane_id| self.model.layout.panes.get(&pane_id).cloned());

        match active_pane {
            Some(pane) => {
                let body = body_rect.shrink(1.0);
                ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                    ui.set_clip_rect(body);
                    self.draw_pane(ui, &pane);
                });
            }
            None => {
                ui.painter().text(
                    body_rect.center(),
                    Align2::CENTER_CENTER,
                    "⌘⇧P to execute a command",
                    egui::FontId::proportional(theme::UI_SIZE),
                    palette.muted,
                );
            }
        }

        // Clicking anywhere in a frame makes it the one the keyboard talks to.
        //
        // This reads the pointer rather than registering a widget: a click-sensing widget the
        // size of the frame would sit on top of everything drawn inside it and swallow every
        // click meant for a tab, a diff line or a shell.
        let pressed_inside = ui.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|at| rect.contains(at))
        });
        if pressed_inside && !is_active {
            let layout = take_layout(&mut self.model.layout);
            self.model.layout = focus_frame(layout, frame_id);
        }

        if self.model.dragging_pane.is_some() {
            self.draw_drop_hint(ui, frame_id, rect, strip_rect, palette);
        }
    }

    fn draw_tab_strip(&mut self, ui: &mut Ui, frame_id: &str, is_primary: bool, palette: &Palette) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TAB_GAP;

            let pane_ids = self
                .model
                .layout
                .frames
                .get(frame_id)
                .map(|frame| frame.pane_ids.clone())
                .unwrap_or_default();
            let active_pane_id = self
                .model
                .layout
                .frames
                .get(frame_id)
                .and_then(|frame| frame.active_pane_id.clone());

            for pane_id in &pane_ids {
                let Some(pane) = self.model.layout.panes.get(pane_id).cloned() else {
                    continue;
                };
                let selected = active_pane_id.as_deref() == Some(pane_id.as_str());
                self.draw_tab(ui, frame_id, &pane, selected, palette);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_primary {
                    self.draw_global_actions(ui, palette);
                }
                if widgets::round_button(ui, "+", TAB_HEIGHT, palette).clicked() {
                    let placement = self.room_for_a_column(frame_id);
                    self.spawn_terminal(None, placement);
                }
            });
        });
    }

    fn draw_tab(
        &mut self,
        ui: &mut Ui,
        frame_id: &str,
        pane: &Pane,
        selected: bool,
        palette: &Palette,
    ) {
        // A shell's tab takes the title the running program set, the way a terminal does.
        let title = match pane {
            Pane::Terminal { terminal_id, .. } => self
                .terminals
                .get(terminal_id)
                .and_then(|terminal| terminal.title())
                .unwrap_or_else(|| pane.tab_title()),
            _ => pane.tab_title(),
        };
        let label = widgets::elide_path(&title, 22);
        let pane_id = pane.pane_id().to_string();

        let galley = ui.painter().layout_no_wrap(
            label.clone(),
            egui::FontId::proportional(SMALL_SIZE + 1.0),
            if selected { palette.ink } else { palette.muted },
        );
        let width = galley.size().x
            + TAB_TEXT_INSET
            + TAB_CLOSE_GAP
            + TAB_CLOSE_SIZE
            + TAB_CLOSE_INSET;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, TAB_HEIGHT), Sense::click_and_drag());
        self.tab_rects
            .push((frame_id.to_string(), pane_id.clone(), rect));

        let dragging_this = self.model.dragging_pane.as_deref() == Some(pane_id.as_str());
        if ui.is_rect_visible(rect) {
            // A dragged tab rides the pointer, drawn above everything else and holding the spot
            // it was picked up by. Its slot in the strip stays behind as an outline, so the
            // strip's other tabs don't jump around underneath the drag.
            let pointer = ui.input(|input| input.pointer.hover_pos());
            let (painter, rect) = match (dragging_this, pointer) {
                (true, Some(at)) => {
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(4),
                        Stroke::new(1.0, palette.line),
                        StrokeKind::Inside,
                    );
                    (
                        ui.ctx()
                            .layer_painter(egui::LayerId::new(
                                egui::Order::Foreground,
                                Id::new("dragged-tab"),
                            )),
                        Rect::from_min_size(at - self.tab_grab_offset, rect.size()),
                    )
                }
                _ => (ui.painter().clone(), rect),
            };

            let fill = if dragging_this || selected {
                palette.control_active_bg
            } else if response.hovered() {
                palette.control_bg
            } else {
                Color32::TRANSPARENT
            };
            painter.rect_filled(rect, CornerRadius::same(4), fill);
            if dragging_this {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, palette.accent),
                    StrokeKind::Inside,
                );
            }
            let text_height = galley.size().y;
            painter.galley(
                pos2(
                    rect.min.x + TAB_TEXT_INSET,
                    (rect.center().y - text_height / 2.0).round(),
                ),
                galley,
                palette.ink,
            );

            let close_rect = Rect::from_center_size(
                pos2(
                    rect.max.x - TAB_CLOSE_INSET - TAB_CLOSE_SIZE / 2.0,
                    rect.center().y,
                ),
                vec2(TAB_CLOSE_SIZE, TAB_CLOSE_SIZE),
            );
            let hovering_close = !dragging_this && pointer.is_some_and(|at| close_rect.contains(at));
            if !dragging_this && (response.hovered() || selected) {
                ui.painter().text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    "\u{1F5D9}",
                    egui::FontId::proportional(SMALL_SIZE - 1.0),
                    if hovering_close {
                        palette.warn
                    } else {
                        palette.muted
                    },
                );
            }

            if response.clicked() && hovering_close {
                self.pending_close = Some(pane_id.clone());
                return;
            }
        }

        if response.clicked() {
            let layout = take_layout(&mut self.model.layout);
            self.model.layout = layout::focus_pane(layout, &pane_id);
        }
        // Middle-click closes a tab, as it does in a browser.
        if response.middle_clicked() {
            self.pending_close = Some(pane_id.clone());
        }
        if response.drag_started() {
            self.model.dragging_pane = Some(pane_id.clone());
            self.tab_grab_offset = ui
                .input(|input| input.pointer.press_origin())
                .map(|at| at - rect.min)
                .unwrap_or_else(|| rect.size() / 2.0);
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }

    fn draw_global_actions(&mut self, ui: &mut Ui, palette: &Palette) {
        // The bundled fonts have no sun or moon glyph, so the switch is drawn rather than
        // typeset — see the glyph test in `ui_tests`.
        let next = self.model.theme.toggled();
        if theme_switch(ui, self.model.theme, palette)
            .on_hover_text(format!("switch to {} (⌘J)", next.label()))
            .clicked()
        {
            self.set_theme(next);
        }

    }

    /// A new shell goes in its own column down the right of the workspace, unless that would
    /// squeeze that column — or whatever it takes the room from — below a usable width, in
    /// which case it becomes another tab in the frame it was asked for.
    fn room_for_a_column(&self, frame_id: &str) -> TerminalPlacement {
        let Some(workspace) = self
            .frame_rects
            .iter()
            .map(|(_, rect)| *rect)
            .reduce(|whole, rect| whole.union(rect))
        else {
            return TerminalPlacement::RightColumn;
        };
        let narrowest = self
            .frame_rects
            .iter()
            .map(|(_, rect)| rect.width())
            .fold(f32::INFINITY, f32::min);

        let new_column = workspace.width() * layout::RIGHT_COLUMN_FRACTION;
        let squeezed = narrowest * (1.0 - layout::RIGHT_COLUMN_FRACTION);
        if new_column >= MIN_COLUMN_WIDTH && squeezed >= MIN_COLUMN_WIDTH {
            TerminalPlacement::RightColumn
        } else {
            TerminalPlacement::Tab(frame_id.to_string())
        }
    }

    /// Where a tab dropped on this frame's strip would be inserted: the pane it lands before,
    /// `None` for the end of the strip, and the x a caret would mark that spot at.
    fn tab_insertion(
        &self,
        frame_id: &str,
        dragged_pane_id: &str,
        strip_rect: Rect,
        at: egui::Pos2,
    ) -> (Option<String>, f32) {
        let mut after_last = strip_rect.min.x + FRAME_BORDER + TAB_MARGIN;
        for (tab_frame, tab_pane, rect) in &self.tab_rects {
            if tab_frame != frame_id || tab_pane == dragged_pane_id {
                continue;
            }
            if at.x < rect.center().x {
                return (Some(tab_pane.clone()), rect.min.x - TAB_GAP / 2.0);
            }
            after_last = rect.max.x + TAB_GAP / 2.0;
        }
        (None, after_last)
    }

    /// While a tab is being dragged, show where it would land.
    fn draw_drop_hint(
        &self,
        ui: &mut Ui,
        frame_id: &str,
        rect: Rect,
        strip_rect: Rect,
        palette: &Palette,
    ) {
        let Some(at) = ui.input(|input| input.pointer.hover_pos()) else {
            return;
        };
        if !rect.contains(at) {
            return;
        }
        let Some(side) = drop_side(rect, strip_rect, at) else {
            return;
        };

        // Landing among the tabs is a caret between two of them — the precise spot the tab takes
        // — rather than a wash over the whole strip.
        if side == DropSide::Tabs {
            let dragged = self.model.dragging_pane.clone().unwrap_or_default();
            let (_, x) = self.tab_insertion(frame_id, &dragged, strip_rect, at);
            let top = strip_rect.min.y + FRAME_BORDER + TAB_MARGIN;
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(x - 1.0, top), pos2(x + 1.0, top + TAB_HEIGHT)),
                CornerRadius::same(1),
                palette.accent,
            );
            return;
        }

        let hint = match side {
            DropSide::Tabs => strip_rect,
            DropSide::Left => rect.with_max_x(rect.min.x + rect.width() * 0.5),
            DropSide::Right => rect.with_min_x(rect.min.x + rect.width() * 0.5),
            DropSide::Top => rect.with_max_y(rect.min.y + rect.height() * 0.5),
            DropSide::Bottom => rect.with_min_y(rect.min.y + rect.height() * 0.5),
        };
        ui.painter().rect_filled(
            hint.shrink(3.0),
            CornerRadius::same(5),
            palette.accent.linear_multiply(0.18),
        );
        ui.painter().rect_stroke(
            hint.shrink(3.0),
            CornerRadius::same(5),
            Stroke::new(1.5, palette.accent),
            StrokeKind::Inside,
        );
    }

    fn finish_tab_drag(&mut self, at: Option<egui::Pos2>) {
        let Some(pane_id) = self.model.dragging_pane.take() else {
            return;
        };
        let Some(at) = at else {
            return;
        };

        let Some((frame_id, frame_rect)) = self
            .frame_rects
            .iter()
            .find(|(_, rect)| rect.contains(at))
            .cloned()
        else {
            return;
        };
        let strip_rect =
            Rect::from_min_size(frame_rect.min, vec2(frame_rect.width(), TAB_STRIP_HEIGHT));
        let Some(side) = drop_side(frame_rect, strip_rect, at) else {
            return;
        };

        // Landing on a tab strip means "before whichever tab the pointer is left of" — the same
        // spot the caret marked while the drag was in flight.
        let before_pane_id = (side == DropSide::Tabs)
            .then(|| self.tab_insertion(&frame_id, &pane_id, strip_rect, at).0)
            .flatten();

        let layout = take_layout(&mut self.model.layout);
        self.model.layout = layout::move_pane_to_frame(
            layout,
            &pane_id,
            &frame_id,
            side,
            before_pane_id.as_deref(),
        );
    }
}

fn drop_side(rect: Rect, strip_rect: Rect, at: egui::Pos2) -> Option<DropSide> {
    if strip_rect.contains(at) {
        return Some(DropSide::Tabs);
    }
    if !rect.contains(at) {
        return None;
    }

    // Each distance is measured in units of its own edge's band, so a value below 1.0 means the
    // pointer is inside that band and the smallest value is the band it is deepest into. The
    // vertical ones start below the tab strip, which claims the top of the frame for itself.
    let side_band = (rect.width() * SIDE_EDGE_FRACTION).max(1.0);
    let body_top = strip_rect.max.y;
    let up_down_band = ((rect.max.y - body_top) * UP_DOWN_EDGE_FRACTION).max(1.0);

    let from_left = (at.x - rect.min.x) / side_band;
    let from_right = (rect.max.x - at.x) / side_band;
    let from_top = (at.y - body_top) / up_down_band;
    let from_bottom = (rect.max.y - at.y) / up_down_band;
    let nearest = from_left.min(from_right).min(from_top).min(from_bottom);

    if nearest > 1.0 {
        // Dropped well inside a frame: join its tabs rather than split it.
        return Some(DropSide::Tabs);
    }
    Some(if nearest == from_left {
        DropSide::Left
    } else if nearest == from_right {
        DropSide::Right
    } else if nearest == from_top {
        DropSide::Top
    } else {
        DropSide::Bottom
    })
}

/// Divide a split's area between its children, leaving room for the handles between them.
/// Returns the child rects and the space the shares were taken from, which is what a drag
/// on a handle converts pixels into fractions with.
fn split_child_rects(
    rect: Rect,
    direction: SplitDirection,
    sizes: &[f32],
    count: usize,
) -> (Vec<Rect>, f32) {
    let horizontal = direction == SplitDirection::Row;
    let total = if horizontal { rect.width() } else { rect.height() };
    let gaps = DIVIDER_THICKNESS * count.saturating_sub(1) as f32;
    let usable = (total - gaps).max(1.0);
    let even = 1.0 / count.max(1) as f32;

    let mut rects = Vec::with_capacity(count);
    let mut offset = 0.0;
    for index in 0..count {
        let extent = usable * sizes.get(index).copied().unwrap_or(even);
        rects.push(if horizontal {
            Rect::from_min_size(
                pos2(rect.min.x + offset, rect.min.y),
                vec2(extent, rect.height()),
            )
        } else {
            Rect::from_min_size(
                pos2(rect.min.x, rect.min.y + offset),
                vec2(rect.width(), extent),
            )
        });
        offset += extent + DIVIDER_THICKNESS;
    }

    (rects, usable)
}

fn take_layout(layout: &mut WorkspaceLayout) -> WorkspaceLayout {
    std::mem::replace(layout, layout::empty_layout())
}

fn focus_frame(mut layout: WorkspaceLayout, frame_id: &str) -> WorkspaceLayout {
    if layout.frames.contains_key(frame_id) {
        layout.active_frame_id = frame_id.to_string();
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: f32, height: f32) -> (Rect, Rect) {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, height));
        let strip = Rect::from_min_size(rect.min, vec2(width, TAB_STRIP_HEIGHT));
        (rect, strip)
    }

    #[test]
    fn dropping_on_the_tab_strip_joins_its_tabs() {
        let (rect, strip) = frame(400.0, 300.0);
        assert_eq!(drop_side(rect, strip, pos2(200.0, 10.0)), Some(DropSide::Tabs));
    }

    #[test]
    fn dropping_near_an_edge_splits_on_that_side() {
        let (rect, strip) = frame(400.0, 300.0);
        assert_eq!(drop_side(rect, strip, pos2(10.0, 150.0)), Some(DropSide::Left));
        assert_eq!(drop_side(rect, strip, pos2(390.0, 150.0)), Some(DropSide::Right));
        assert_eq!(drop_side(rect, strip, pos2(200.0, 295.0)), Some(DropSide::Bottom));
    }

    #[test]
    fn dropping_in_the_middle_joins_the_tabs_rather_than_splitting() {
        let (rect, strip) = frame(400.0, 300.0);
        assert_eq!(drop_side(rect, strip, pos2(200.0, 150.0)), Some(DropSide::Tabs));
    }

    #[test]
    fn a_bottom_split_is_reachable_without_dragging_to_the_very_edge() {
        // A wide frame: two thirds of the way down the body is already the bottom band, so the
        // drag doesn't have to travel to the window's edge to split downwards.
        let (rect, strip) = frame(1400.0, 900.0);
        let two_thirds_down = strip.max.y + (rect.max.y - strip.max.y) * 0.7;
        assert_eq!(
            drop_side(rect, strip, pos2(700.0, two_thirds_down)),
            Some(DropSide::Bottom)
        );
    }

    #[test]
    fn dropping_outside_a_frame_is_not_a_drop() {
        let (rect, strip) = frame(400.0, 300.0);
        assert_eq!(drop_side(rect, strip, pos2(800.0, 150.0)), None);
    }

    #[test]
    fn split_children_tile_the_area_minus_the_handles() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));

        let (rects, usable) = split_child_rects(area, SplitDirection::Row, &[0.65, 0.35], 2);

        assert_eq!(rects.len(), 2);
        assert!((usable - (area.width() - DIVIDER_THICKNESS)).abs() < f32::EPSILON);
        let covered: f32 = rects.iter().map(Rect::width).sum();
        assert!(
            (covered - usable).abs() < 0.01,
            "columns should fill the usable width, got {covered} of {usable}"
        );
        assert!(rects.iter().all(|rect| rect.height() == area.height()));
        // The second column starts after the first one plus the handle between them.
        assert!((rects[1].min.x - (rects[0].max.x + DIVIDER_THICKNESS)).abs() < 0.01);
    }

    #[test]
    fn a_column_split_divides_height_instead_of_width() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 900.0));

        let (rects, usable) = split_child_rects(area, SplitDirection::Column, &[0.5, 0.5], 2);

        assert!((usable - (area.height() - DIVIDER_THICKNESS)).abs() < f32::EPSILON);
        assert!(rects.iter().all(|rect| rect.width() == area.width()));
        assert!((rects[0].height() - rects[1].height()).abs() < 0.01);
    }

    #[test]
    fn missing_sizes_fall_back_to_an_even_split() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 100.0));

        let (rects, _) = split_child_rects(area, SplitDirection::Row, &[], 3);

        assert_eq!(rects.len(), 3);
        assert!((rects[0].width() - rects[2].width()).abs() < 0.01);
    }
}
