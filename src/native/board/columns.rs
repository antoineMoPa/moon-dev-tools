//! The board's columns: what they are called, what order they are in, and the drag that
//! changes it.
//!
//! A column is a name cards are in rather than a place on screen, so moving one moves nothing
//! about any card - the cards are simply drawn wherever their column ends up. That is why
//! dragging a heading takes its whole column with it and costs one line in the board's file.

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, Ui, vec2};

use crate::{
    moontasks::{BoardColumn, ColumnEnd, ColumnId},
    native::{
        app::App,
        board::{
            Axis, BoardAction, cards::DRAGGED_CARD_OPACITY, close_button, close_mark, plus_button,
            slide_into_place, stamp_place,
        },
        model::ColumnRename,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// The column a drag is carrying. A type of its own so the board can tell a column being moved
/// from a card being moved, and never read one as the other.
#[derive(Clone)]
pub(super) struct DraggedColumn(pub(super) ColumnId);

/// How wide the box for a new column's name is. Narrower than a column, because it is a name
/// rather than a column of cards.
const NEW_COLUMN_WIDTH: f32 = 150.0;

/// The id a column is dragged by, which is also the layer its ghost is drawn into.
pub(super) fn column_drag_id(column_id: &ColumnId) -> egui::Id {
    egui::Id::new(("moontask-column", column_id.as_str()))
}

/// The columns of the board, in the order they are drawn - with the one being dragged already
/// moved to where it is being held.
///
/// The move is made as it is being made rather than once it is over, so the columns around it
/// are already out of the way when it is let go of and nothing jumps.
pub(super) fn ordered_columns(app: &App, dragged: Option<&ColumnId>) -> Vec<BoardColumn> {
    let mut columns = app.model.board.columns.clone();
    let Some(landing) = app.model.board.column_landing else {
        return columns;
    };
    let Some(dragged) = dragged else {
        return columns;
    };
    let Some(at) = columns.iter().position(|column| column.id == *dragged) else {
        return columns;
    };

    let column = columns.remove(at);
    columns.insert(landing.min(columns.len()), column);
    columns
}

/// Move a column on the board being drawn, ahead of the server being told about it - the same
/// reasoning [`super::place_in`] has for cards.
pub(crate) fn place_column_in(columns: &mut Vec<BoardColumn>, column_id: &ColumnId, index: usize) {
    let Some(at) = columns.iter().position(|column| column.id == *column_id) else {
        return;
    };
    let column = columns.remove(at);
    columns.insert(index.min(columns.len()), column);
}

/// Take the columns the server answered with, keeping a move it may not have seen yet.
pub(crate) fn accept_columns(
    model: &mut crate::native::model::Model,
    mut columns: Vec<BoardColumn>,
) {
    if let Some(pending) = &model.board.pending_column_place {
        let landed = columns
            .iter()
            .position(|column| column.id == pending.column_id)
            .is_some_and(|at| at == pending.index.min(columns.len().saturating_sub(1)));
        if landed {
            model.board.pending_column_place = None;
        } else {
            let (column_id, index) = (pending.column_id.clone(), pending.index);
            place_column_in(&mut columns, &column_id, index);
        }
    }
    model.board.columns = columns;
}

/// One column's heading: what it is called, how many cards are in it, and everything that can
/// be done to the column itself.
///
/// The name is the handle the column is dragged by, the way a card's title is the handle the
/// card is dragged by - and for the same reason, so the marks beside it stay clickable.
pub(super) fn draw_heading(
    app: &mut App,
    ui: &mut Ui,
    column: &BoardColumn,
    cards: usize,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let editing = app
        .model
        .board
        .renaming_column
        .as_ref()
        .is_some_and(|rename| rename.column_id == column.id);
    let pending_delete = app.model.board.pending_column_delete.as_ref() == Some(&column.id);

    ui.horizontal(|ui| {
        if editing {
            draw_heading_editor(app, ui, column, actions);
        } else {
            draw_heading_handle(app, ui, column, palette, actions);
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            if pending_delete {
                match widgets::confirm(
                    ui,
                    palette,
                    "[really remove]",
                    "this takes the column off the board; the cards it held are already gone \
                     from it",
                ) {
                    widgets::Confirmed::Yes => {
                        app.model.board.pending_column_delete = None;
                        actions.push(BoardAction::DeleteColumn(column.id.clone()));
                    }
                    widgets::Confirmed::No => app.model.board.pending_column_delete = None,
                    widgets::Confirmed::Waiting => {}
                }
            } else {
                // A column holding cards is the only record of where those cards are, so the
                // mark is there and says why it will not do anything rather than being absent
                // and leaving someone hunting for it.
                let mark = close_mark(ui, palette, cards == 0);
                let mark = if cards == 0 {
                    mark.on_hover_text("Remove this column from the board")
                } else {
                    mark.on_hover_text("Move this column's cards elsewhere before removing it")
                };
                if mark.clicked() && cards == 0 {
                    app.model.board.pending_column_delete = Some(column.id.clone());
                }

                ui.label(
                    RichText::new(cards.to_string())
                        .size(SMALL_SIZE - 1.0)
                        .color(palette.muted),
                );
            }
        });
    });
}

/// The heading as it usually reads: what the column is dragged by, what a double click opens
/// for renaming, and what a right click offers about the column itself.
fn draw_heading_handle(
    app: &mut App,
    ui: &mut Ui,
    column: &BoardColumn,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let laid_out = ui
        .add(
            egui::Label::new(
                RichText::new(&column.label)
                    .size(SMALL_SIZE - 1.0)
                    .color(palette.muted)
                    .strong(),
            )
            .selectable(false),
        )
        .rect;

    let handle = ui
        .interact(
            laid_out,
            column_drag_id(&column.id),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move this column, double click to rename it");

    if handle.double_clicked() {
        app.model.board.renaming_column = Some(ColumnRename {
            column_id: column.id.clone(),
            label: column.label.clone(),
            focus: true,
        });
    }

    draw_heading_menu(&handle, column, actions);
}

/// What a column can be told about itself: where cards moved into it land. A menu rather than
/// a mark on the heading, because it is set once for a column and then left alone.
fn draw_heading_menu(
    handle: &egui::Response,
    column: &BoardColumn,
    actions: &mut Vec<BoardAction>,
) {
    egui::Popup::context_menu(handle)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(
                RichText::new("a card moved in from another column goes").size(SMALL_SIZE - 1.0),
            );
            // Written out rather than derived: these are the three answers, and each says
            // plainly what the column will do with what is dropped on it.
            for (label, hover, arrivals) in [
                (
                    "where it is dropped",
                    "the drop decides, the way an ordinary column behaves",
                    None,
                ),
                (
                    "to the top",
                    "what arrived last is what the column shows first - what DONE wants",
                    Some(ColumnEnd::Top),
                ),
                (
                    "to the bottom",
                    "the column is a queue, and an arrival joins the back of it",
                    Some(ColumnEnd::Bottom),
                ),
            ] {
                let chosen = column.arrivals == arrivals;
                if widgets::clickable(ui.selectable_label(chosen, label))
                    .on_hover_text(hover)
                    .clicked()
                {
                    actions.push(BoardAction::SetColumnArrivals(column.id.clone(), arrivals));
                    ui.close();
                }
            }
        });
}

/// The heading being renamed. Enter and clicking away keep it, Escape throws it away - the
/// same shape a card's title has.
fn draw_heading_editor(
    app: &mut App,
    ui: &mut Ui,
    column: &BoardColumn,
    actions: &mut Vec<BoardAction>,
) {
    let Some(rename) = &mut app.model.board.renaming_column else {
        return;
    };
    let entry = ui.add_sized(
        vec2(NEW_COLUMN_WIDTH, ui.spacing().interact_size.y),
        egui::TextEdit::singleline(&mut rename.label).hint_text("Column name"),
    );
    if std::mem::take(&mut rename.focus) {
        entry.request_focus();
    }

    let label = rename.label.clone();
    let abandon = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let keep = entry.lost_focus() && !abandon;

    if keep && !label.trim().is_empty() && label != column.label {
        actions.push(BoardAction::RenameColumn(column.id.clone(), label));
    } else if abandon || keep {
        actions.push(BoardAction::CancelColumnRename);
    }
}

/// Draw a column where the layout puts it, sliding there when the order has changed, and carry
/// it to the cursor while it is the one being dragged.
///
/// The whole column goes with the heading - the cards are drawn inside it - so a column moves
/// as the thing it is rather than as a heading that leaves its cards behind.
pub(super) fn with_column_drag(
    app: &mut App,
    ui: &mut Ui,
    column: &BoardColumn,
    origin: f32,
    draw: impl FnOnce(&mut App, &mut Ui) -> egui::Rect,
) -> egui::Rect {
    let drag_id = column_drag_id(&column.id);
    if !ui.ctx().is_being_dragged(drag_id) {
        return slide_into_place(ui, Axis::Horizontal, drag_id, origin, |ui| draw(app, ui));
    }

    egui::DragAndDrop::set_payload(ui.ctx(), DraggedColumn(column.id.clone()));
    stamp_place(ui, Axis::Horizontal, drag_id, origin);

    let layer_id = egui::LayerId::new(egui::Order::Tooltip, drag_id);
    let laid_out = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
            // A ghost of the column, so the gap it is being held over shows through it.
            ui.set_opacity(DRAGGED_CARD_OPACITY);
            draw(app, ui)
        })
        .inner;

    // Onto the cursor, and only sideways: a column is as tall as the board, and one that
    // followed the cursor up and down would leave the row it belongs to rather than look like
    // it was being moved along it.
    //
    // Carried by the cursor rather than by how far the cursor has come, which is what a card
    // does too - and for a reason beyond matching. The columns reorder underneath the drag as
    // it goes, so where this one was laid out changes while it is in the air; a ghost placed
    // relative to that would chase its own tail, and one placed on the cursor cannot.
    if let Some(pointer) = ui.ctx().pointer_interact_pos() {
        ui.ctx().transform_layer_shapes(
            layer_id,
            egui::emath::TSTransform::from_translation(vec2(pointer.x - laid_out.center().x, 0.0)),
        );
    }
    laid_out
}

/// Where a dragged column would land: how many of the others it has been carried past.
///
/// The dragged column is carried on the cursor, so the cursor is its middle - and this is
/// middle against middle, against the columns as they are drawn now with the dragged one
/// already out of the reckoning, so carrying it over a column cannot bounce between two
/// answers.
pub(super) fn landing_for(dragged_centre: f32, headings: &[(ColumnId, f32)]) -> usize {
    headings
        .iter()
        .filter(|(_, centre)| *centre < dragged_centre)
        .count()
}

/// The box a new column is named in, at the right-hand end of the board.
pub(super) fn draw_new_column(
    app: &mut App,
    ui: &mut Ui,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    if !app.model.board.column_composer_open {
        if plus_button(ui, palette)
            .on_hover_text("New column")
            .clicked()
        {
            app.model.board.column_composer_open = true;
            app.model.board.column_composer_focus = true;
        }
        return;
    }

    egui::Frame::new()
        .fill(palette.composer_bg)
        .stroke(egui::Stroke::new(1.0, palette.accent))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 7))
        .show(ui, |ui| {
            ui.set_width(NEW_COLUMN_WIDTH);
            let entry = ui.add(
                egui::TextEdit::singleline(&mut app.model.board.new_column_label)
                    .hint_text("Column name")
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut app.model.board.column_composer_focus) {
                entry.request_focus();
            }

            let submitted =
                entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let ready = !app.model.board.new_column_label.trim().is_empty();

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::clickable(ui.add_enabled(ready, egui::Button::new("add")))
                    .on_hover_text("Add this column to the board")
                    .clicked()
                    || (submitted && ready)
                {
                    actions.push(BoardAction::AddColumn(
                        app.model.board.new_column_label.trim().to_string(),
                    ));
                }
                if close_button(ui, palette)
                    .on_hover_text("Discard this column")
                    .clicked()
                    || ui.input(|input| input.key_pressed(egui::Key::Escape))
                {
                    actions.push(BoardAction::CloseColumnComposer);
                }
            });
        });
}
