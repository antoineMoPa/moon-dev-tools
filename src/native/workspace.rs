//! The pane arrangement on screen: nested splits, a tab strip per frame, draggable tabs.
//!
//! [`crate::native::layout`] owns what the arrangement *is*; this file owns how it is drawn
//! and how pointer gestures turn into the next arrangement.

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use crate::native::{
    app::App,
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

const TAB_STRIP_HEIGHT: f32 = 25.0;
const DIVIDER_THICKNESS: f32 = 5.0;
/// How close to a frame's edge a dropped tab has to land to split it there.
const EDGE_FRACTION: f32 = 0.22;

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

        let is_primary = self.model.layout.primary_frame_id() == frame_id;
        ui.scope_builder(UiBuilder::new().max_rect(strip_rect.shrink2(vec2(4.0, 3.0))), |ui| {
            ui.set_clip_rect(strip_rect);
            self.draw_tab_strip(ui, frame_id, is_primary, palette);
        });

        ui.painter().hline(
            rect.x_range(),
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
                    "⌘⇧P to open a pane",
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
            self.draw_drop_hint(ui, rect, strip_rect, palette);
        }
    }

    fn draw_tab_strip(&mut self, ui: &mut Ui, frame_id: &str, is_primary: bool, palette: &Palette) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if is_primary {
                // The app's mark, drawn beside the first tab: this strip is the app header.
                let (rect, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
                if ui.is_rect_visible(rect) {
                    draw_moon(ui.painter(), rect.center(), 5.5, palette.ink, palette.header_bg);
                }
            }

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
                if widgets::quiet_button(ui, "+")
                    .on_hover_text("open a pane (⌘⇧P)")
                    .clicked()
                {
                    self.model.palette.open = true;
                    self.model.palette.query.clear();
                    self.model.palette.highlighted = 0;
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
        let width = galley.size().x + 26.0;
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, TAB_STRIP_HEIGHT - 7.0),
            Sense::click_and_drag(),
        );
        self.tab_rects
            .push((frame_id.to_string(), pane_id.clone(), rect));

        let dragging_this = self.model.dragging_pane.as_deref() == Some(pane_id.as_str());
        if ui.is_rect_visible(rect) {
            let fill = if selected {
                palette.control_active_bg
            } else if response.hovered() {
                palette.control_bg
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
            if dragging_this {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, palette.accent),
                    StrokeKind::Inside,
                );
            }
            ui.painter()
                .galley(rect.min + vec2(7.0, 3.0), galley, palette.ink);

            let close_rect = Rect::from_center_size(
                pos2(rect.max.x - 9.0, rect.center().y),
                vec2(12.0, 12.0),
            );
            let hovering_close = ui
                .input(|input| input.pointer.hover_pos())
                .is_some_and(|at| close_rect.contains(at));
            if response.hovered() || selected {
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

    /// While a tab is being dragged, show where it would land.
    fn draw_drop_hint(&self, ui: &mut Ui, rect: Rect, strip_rect: Rect, palette: &Palette) {
        let Some(at) = ui.input(|input| input.pointer.hover_pos()) else {
            return;
        };
        if !rect.contains(at) {
            return;
        }
        let Some(side) = drop_side(rect, strip_rect, at) else {
            return;
        };

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

        // Landing on a tab strip means "before whichever tab the pointer is left of".
        let before_pane_id = (side == DropSide::Tabs)
            .then(|| {
                self.tab_rects
                    .iter()
                    .filter(|(tab_frame, tab_pane, _)| {
                        tab_frame == &frame_id && tab_pane != &pane_id
                    })
                    .find(|(_, _, rect)| at.x < rect.center().x)
                    .map(|(_, tab_pane, _)| tab_pane.clone())
            })
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

    let from_left = (at.x - rect.min.x) / rect.width();
    let from_right = (rect.max.x - at.x) / rect.width();
    let from_top = (at.y - rect.min.y) / rect.height();
    let from_bottom = (rect.max.y - at.y) / rect.height();
    let nearest = from_left.min(from_right).min(from_top).min(from_bottom);

    if nearest > EDGE_FRACTION {
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
