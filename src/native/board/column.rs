//! One column of the board: its header, its cards, and the drop that lands cards in it.

use super::cards::{
    CARD_SPACING, card_drag_id, column_cards, draw_card, draw_empty_slot, draw_pending_card,
    place_in,
};

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, ScrollArea, Ui, vec2};

use crate::{
    moontasks::{BoardColumn, ColumnEnd, ColumnId},
    native::{
        app::App,
        model::{PendingPlace, TaskLanding},
        theme::{Palette, SMALL_SIZE},
    },
};

use super::{BoardAction, COLUMN_WIDTH, cards, columns, day_lines, filter, gesture};

use super::marks::plus_button;

pub(super) fn draw_column(
    app: &mut App,
    ui: &mut Ui,
    column: &BoardColumn,
    height: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    let status = column.id.clone();
    let carrying = app.model.board.carrying.clone();
    let tasks = column_cards(app, &status);
    // The dated lines a column of finished work draws between its days - see `day_lines`.
    let mut lines_between_days =
        day_lines::drawn_in(column).then(day_lines::DayLines::starting_now);

    // A column stacks its cards, whatever layout the row of columns is in.
    ui.allocate_ui_with_layout(
        vec2(COLUMN_WIDTH, height),
        UiLayout::top_down(Align::Min),
        |ui| {
            ui.set_width(COLUMN_WIDTH);
            columns::draw_heading(
                app,
                ui,
                column,
                cards::column_size(&app.model.board.tasks, &status),
                palette,
                actions,
            );
            ui.add_space(3.0);

            // Which end of this column the card being written on the new-task pane will join,
            // if it is this column it is joining at all.
            let pending = app
                .model
                .board
                .card_being_written
                .as_ref()
                .filter(|pending| pending.column == status)
                .map(|pending| pending.joins);

            // Where a drop would land, counted against the cards it would be put among - so the
            // slot the dragged card is standing in is taken back out of the reckoning, and moving
            // the pointer over a card cannot bounce between two answers.
            let mut cards: Vec<egui::Rect> = Vec::new();
            let mut slot = 0.0;
            // The column's own buttons - the `+` at either end of it. A press on one of them
            // is theirs, the way a card's buttons are the card's.
            let mut controls = gesture::Controls::new(ui);
            let mut zone = egui::Frame::new()
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(4))
                .begin(ui);
            {
                let ui = &mut zone.content_ui;
                ScrollArea::vertical()
                    .id_salt(format!("moontasks-column-{status}"))
                    // Dragging is how a card is moved, so it must not also mean "scroll the
                    // column" - the same reason the board's own scroll area says it.
                    .scroll_source(egui::containers::scroll_area::ScrollSource {
                        drag: egui::containers::scroll_area::DragScroll::Never,
                        ..Default::default()
                    })
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // What a card's place is measured against, so scrolling the column is not
                        // read as every card in it having moved.
                        let origin = ui.min_rect().top();
                        // Over the first card, where a card added here will appear - the twin
                        // of the `+` under the last one, drawn the same way so the two ends of
                        // the column read as the same offer.
                        ui.vertical_centered(|ui| {
                            let plus =
                                plus_button(ui, palette).on_hover_text("New task at the top");
                            if controls.pressed(&plus) {
                                actions
                                    .push(BoardAction::OpenNewTask(status.clone(), ColumnEnd::Top));
                            }
                        });
                        ui.add_space(CARD_SPACING);
                        if pending == Some(ColumnEnd::Top) {
                            draw_pending_card(
                                ui,
                                palette,
                                &mut controls,
                                actions,
                                &status,
                                ColumnEnd::Top,
                            );
                            ui.add_space(CARD_SPACING);
                        }
                        for task in &tasks {
                            if let Some(lines) = &mut lines_between_days
                                && let Some(label) = lines.line_above(
                                    task.entered_column_at_unix.map(day_lines::LocalDay::of),
                                )
                            {
                                day_lines::draw(ui, palette, &label);
                                ui.add_space(CARD_SPACING);
                            }
                            let card = draw_card(app, ui, task, origin, palette, actions);
                            // A card being carried is the space being held for the drop rather
                            // than a place the drop could be aimed at, so it is counted out of
                            // the slots and its height taken back off the cards below it. The
                            // one on the cursor leaves a hole here; the others are drawn faint
                            // where they are going.
                            match carrying.as_ref().filter(|c| c.carries(&task.id)) {
                                Some(carrying) => {
                                    if carrying.primary == task.id {
                                        draw_empty_slot(ui, card, palette);
                                    }
                                    slot += card.height() + CARD_SPACING;
                                }
                                None => cards.push(card.translate(vec2(0.0, -slot))),
                            }
                            ui.add_space(CARD_SPACING);
                        }
                        if pending == Some(ColumnEnd::Bottom) {
                            draw_pending_card(
                                ui,
                                palette,
                                &mut controls,
                                actions,
                                &status,
                                ColumnEnd::Bottom,
                            );
                            ui.add_space(CARD_SPACING);
                        }
                        if !tasks.is_empty() {
                            // Under the last card, where a card added here will appear. Only
                            // once there are cards: in an empty column the `+` above would be
                            // the same offer twice over.
                            ui.vertical_centered(|ui| {
                                let plus = plus_button(ui, palette)
                                    .on_hover_text("New task at the bottom");
                                if controls.pressed(&plus) {
                                    actions.push(BoardAction::OpenNewTask(
                                        status.clone(),
                                        ColumnEnd::Bottom,
                                    ));
                                }
                            });
                        }
                        if tasks.is_empty() && pending.is_none() {
                            // A column emptied by the filter still holds its cards, so it says
                            // that rather than "nothing here", which would read as a column
                            // with nothing in it.
                            let empty = if filter::Filter::of(&app.model.board.filter).is_on() {
                                "nothing matching"
                            } else {
                                "nothing here"
                            };
                            ui.label(RichText::new(empty).size(SMALL_SIZE).color(palette.muted));
                        }
                    });
            }
            let response = zone.allocate_space(ui);
            // A press in the column that no card claimed - the space under the last card, the
            // gap between two of them - is a press on the board, and what a press on the board
            // does is let the marks go.
            if !controls.took_the_press() {
                gesture::claim(
                    &mut app.model.board,
                    ui,
                    response.rect,
                    None,
                    egui::Rect::NOTHING,
                    false,
                );
            }

            // Which column the pointer is in, worked out from the pointer rather than taken from
            // the response: egui hit-tests a frame behind, on the widgets the last frame drew, and
            // a column that has just taken the carried cards in is not the column it hit-tested.
            let ghost = carrying.as_ref().map(|carrying| {
                egui::LayerId::new(egui::Order::Tooltip, card_drag_id(&carrying.primary))
            });
            let over = ghost.is_some() && pointer_over(ui, &response, ghost);
            let landing = over
                .then(|| ui.ctx().pointer_interact_pos())
                .flatten()
                .map(|pointer| {
                    cards
                        .iter()
                        .filter(|card| card.center().y < pointer.y)
                        .count()
                });
            if over {
                zone.frame.fill = palette.control_active_bg;
                zone.frame.stroke = egui::Stroke::new(1.0, palette.accent);
            }
            zone.paint(ui);

            let (Some(at), Some(carrying)) = (landing, carrying) else {
                return;
            };
            // A column that takes arrivals to one end takes them there while they are still in
            // the air: the gap opens at that end however far down the column they are held, so
            // what is shown is where they are going rather than somewhere they will not stay.
            let at = match arrivals_end(app, &status, &carrying.task_ids) {
                Some(ColumnEnd::Top) => 0,
                Some(ColumnEnd::Bottom) => cards.len(),
                None => at,
            };
            // Read by the next frame, which draws the cards in this slot rather than in the
            // ones they were picked up from.
            app.model.board.landing = Some(TaskLanding {
                status: status.clone(),
                index: at,
            });
            if !ui.input(|input| input.pointer.any_released()) {
                return;
            }

            // The cards are already drawn where they landed, and they are the marked ones -
            // which is how a run let go of between two others is picked back out.
            app.model.board.landing = None;
            if cards::sort_of(&app.model.board.columns, &status).is_some() {
                arrive_in_column(app, &status, &carrying.task_ids, actions);
                return;
            }
            // `at` counts the cards the filter is showing; the place they are going to is a
            // place in the column itself.
            let into = cards::column_index_of(
                &app.model.board.tasks,
                &filter::Filter::of(&app.model.board.filter),
                &status,
                &carrying.task_ids,
                at,
            );
            // A column may say which end cards arriving from elsewhere go to, and it is the
            // server that will put them there - so the board works out the same place now.
            // Without this the drop would be drawn where it landed and held there for ever,
            // because the answer it is waiting to agree with never comes.
            let into = arrivals_place(app, &status, &carrying.task_ids).unwrap_or(into);
            // The board is redrawn from the server's answer, which is a worker thread and a
            // poll away: the move is made here as well, and held over every answer until one of
            // them agrees, so the cards stay where they were put rather than springing back and
            // landing a second time.
            app.model.board.pending_place = Some(PendingPlace {
                task_ids: carrying.task_ids.clone(),
                status: status.clone(),
                index: into,
            });
            place_in(
                &mut app.model.board.tasks,
                &carrying.task_ids,
                &status,
                into,
            );
            actions.push(BoardAction::Place(carrying.task_ids, status.clone(), into));
        },
    )
    .response
    .rect
}

/// Cards sent to a column with no place in it said: let go of over a column that keeps an
/// order of its own, where the place they were let go of at says nothing - see
/// [`crate::moontasks::column_sort`] - or moved there from a card's menu, where there was no
/// drop at all.
///
/// Cards shuffled about within the column stay where they are, so nothing is sent: a place
/// counted against the sorted cards on screen would scramble the order the column remembers
/// for when it is no longer sorted. Cards arriving from another column are moved into it, at
/// the end its arrivals go to - the bottom when it names none - and drawn there at once, the
/// same way a drop is.
pub(super) fn arrive_in_column(
    app: &mut App,
    status: &ColumnId,
    task_ids: &[String],
    actions: &mut Vec<BoardAction>,
) {
    let arriving = app
        .model
        .board
        .tasks
        .iter()
        .any(|task| task.status != *status && task_ids.contains(&task.id));
    if !arriving {
        return;
    }
    let into = arrivals_place(app, status, task_ids).unwrap_or_else(|| {
        app.model
            .board
            .tasks
            .iter()
            .filter(|task| task.status == *status && !task_ids.contains(&task.id))
            .count()
    });
    app.model.board.pending_place = Some(PendingPlace {
        task_ids: task_ids.to_vec(),
        status: status.clone(),
        index: into,
    });
    place_in(&mut app.model.board.tasks, task_ids, status, into);
    crate::moontasks::column_sort::arrange(&app.model.board.columns, &mut app.model.board.tasks);
    actions.push(BoardAction::Place(task_ids.to_vec(), status.clone(), into));
}

/// Which end a column insists cards go to, if it insists at all and if these cards are
/// arriving from another column rather than being shuffled about within this one.
///
/// See [`crate::moontasks::store::BoardColumn::arrivals`]. Asked both while a drag is over the
/// column, so the gap opens at the end the cards will actually go to, and when they are let go
/// of, so the board draws the drop where the server's answer will put it.
fn arrivals_end(app: &App, status: &ColumnId, task_ids: &[String]) -> Option<ColumnEnd> {
    let arriving = app
        .model
        .board
        .tasks
        .iter()
        .any(|task| task.status != *status && task_ids.contains(&task.id));
    if !arriving {
        return None;
    }
    app.model
        .board
        .columns
        .iter()
        .find(|column| column.id == *status)?
        .arrivals
}

/// Where in the column itself those cards will land, counted the way the server counts it.
fn arrivals_place(app: &App, status: &ColumnId, task_ids: &[String]) -> Option<usize> {
    Some(match arrivals_end(app, status, task_ids)? {
        ColumnEnd::Top => 0,
        ColumnEnd::Bottom => app
            .model
            .board
            .tasks
            .iter()
            .filter(|task| task.status == *status && !task_ids.contains(&task.id))
            .count(),
    })
}

/// Whether the pointer is inside a column with nothing but the dragged card's own ghost over
/// it - that one follows the cursor, so it is over every column the cursor could be over and
/// would otherwise be the answer to every question about what is under the pointer.
fn pointer_over(ui: &Ui, zone: &egui::Response, ghost: Option<egui::LayerId>) -> bool {
    let Some(pointer) = ui.ctx().input(|input| input.pointer.interact_pos()) else {
        return false;
    };
    if !zone.rect.contains(pointer) {
        return false;
    }
    let over = ui.ctx().layer_id_at(pointer);
    over == Some(zone.layer_id) || (over.is_some() && over == ghost)
}
