//! The moontasks board: the repo's `.moontasks` folder, drawn as columns of cards.
//!
//! The pane holds no state of its own. What it draws comes from the last answer the server
//! gave, and everything it does goes back through the backend, so the same board works
//! against a repo on this machine and one on another.

pub(crate) mod actions;
pub(crate) mod attach;
pub(crate) mod card_menu;
pub(crate) mod cards;
pub(crate) mod column;
pub(crate) mod columns;
pub(crate) mod day_lines;
pub(crate) mod filter;
pub(crate) mod gesture;
pub(crate) mod marks;
pub(crate) mod motion;
pub(crate) mod resources;
pub(crate) mod selection;
pub(crate) mod start;
pub(crate) mod tags;
pub(crate) mod work_on_marked;

pub(super) use actions::BoardAction;
use actions::apply;
use column::draw_column;
use motion::{hold_the_off_axis, scroll_under_the_finger};

use egui::{Align, Layout as UiLayout, RichText, ScrollArea, Ui, vec2};

use crate::{
    api::AgentKind,
    moontasks::ColumnId,
    native::{
        app::App,
        model::{CardMenu, PendingColumnPlace},
        panes::{Pane, PaneKind},
        theme::Palette,
    },
};

/// How wide one column of the board is. Cards are titles and a handful of small buttons, so
/// this is about what a title needs rather than what the window has.
const COLUMN_WIDTH: f32 = 286.0;

pub(super) use crate::native::widgets::CLOSE_MARK_SIZE;

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();
    let mut actions = Vec::new();

    // The board sits off the pane's edges rather than against them: a column hard against the
    // left of the pane reads as cut off. A margin rather than an indent, which would draw the
    // rule down the side that goes with a nested list.
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 12,
            right: 6,
            top: 8,
            bottom: 0,
        })
        .show(ui, |ui| draw_board(app, ui, &palette, &mut actions));

    attach::draw(app, ui.ctx(), &palette, &mut actions);
    // Over the whole board rather than inside the card it belongs to: the cards are laid out
    // and moved every frame, and the menu stands where the click was made.
    card_menu::draw(app, ui.ctx(), &mut actions);
    settle_gesture(app, ui, &mut actions);

    for action in actions {
        apply(app, action);
    }
}

/// What the press on the board turned out to be, once the button comes back up - and the cards
/// it picks up on the way there.
///
/// Read after the columns have drawn, so a card has had its chance to claim the press and the
/// column under the pointer has had its chance to take a drop.
fn settle_gesture(app: &mut App, ui: &Ui, actions: &mut Vec<BoardAction>) {
    // Carrying begins once the press has carried far enough to be a card being picked up. What
    // it carries is settled then and there, from the keys that were held when it went down.
    if app.model.board.carrying.is_none()
        && let Some((task_id, modifiers)) = gesture::grabbed(&app.model.board)
    {
        let task_id = task_id.to_string();
        app.model.board.carrying = Some(selection::carried_by(
            &mut app.model.board,
            &task_id,
            modifiers,
        ));
    }
    if app.model.board.carrying.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    // A finger resting on a card picks it up once it has rested long enough, which is a frame
    // that has to be drawn whether or not anything moves.
    if app
        .model
        .board
        .press
        .as_ref()
        .is_some_and(|press| !press.picks_up && !press.travelled)
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(gesture::HOLD_TO_PICK_UP));
    }

    let Some(ended) = ui.input(|input| gesture::settle(&mut app.model.board, input)) else {
        return;
    };
    let (on, on_title, on_a_button, modifiers) = match ended {
        gesture::Ended::Click {
            on,
            on_title,
            on_a_button,
            modifiers,
        } => (on, on_title, on_a_button, modifiers),
        // A finger held on a card and lifted: the card's menu, where a right click would have
        // put it - see `card_menu`.
        gesture::Ended::Held { on, at } => {
            app.model.board.carrying = None;
            app.model.board.card_menu = Some(CardMenu { task_id: on, at });
            return;
        }
        gesture::Ended::Dropped => {
            // The column it was let go of over has already made the move.
            app.model.board.carrying = None;
            app.model.board.landing = None;
            return;
        }
        // The board has already scrolled under the finger as it went, and carries on at the
        // speed it was flicked at.
        gesture::Ended::Swiped => {
            app.model.board.flung = ui.input(|input| input.pointer.velocity());
            return;
        }
    };

    // A press that went down on one of the card's own buttons and stayed there is that
    // button's: it has already done whatever it does.
    if on_a_button {
        return;
    }

    // A second click on a title opens the box that renames it, rather than opening the task
    // again - the first of the two has already opened it.
    let renaming = on_title
        && ui.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
        });
    if renaming && let Some(task_id) = on {
        open_rename(app, &task_id);
        return;
    }

    if let Some(task_id) = selection::clicked(&mut app.model.board, on.as_deref(), modifiers) {
        let title = app
            .model
            .board
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.title.clone())
            .unwrap_or_default();
        actions.push(BoardAction::OpenStart {
            task_id,
            title,
            opens_on: actions::TaskPaneBox::Neither,
        });
    }
}

/// Put away the pages of the cards a click has let go of.
///
/// A card marked is a task to read and its page is what reads it, so the two keep each other:
/// opening a page marks the card, and letting the card go - Escape, a click on the board beside
/// the cards, another card marked instead - puts the page away. Only the page: a shell started
/// in the task, or a file opened off its card, is a tab of yours and stays until you close it.
///
/// Called after the window has drawn, because a pane is never closed while the tree that holds
/// it is being drawn.
pub(crate) fn close_pages_let_go_of(app: &mut App) {
    for task_id in std::mem::take(&mut app.model.board.pages_to_close) {
        let page = app
            .model
            .layout
            .find_pane(|pane| matches!(pane, Pane::Start { task_id: on, .. } if *on == task_id))
            .map(|(pane, _)| pane);
        if let Some(page) = page {
            app.close_pane(page);
        }
    }
}

/// Put away the pane a new task was being written on, when the cross on the empty card
/// standing for it says no to the task.
///
/// Closing the pane is the whole of it: the draft it was written on and the empty card holding
/// its place both go with it - see [`crate::native::app::App::close_pane`].
pub(crate) fn close_the_new_task_page(app: &mut App) {
    if !std::mem::take(&mut app.model.board.new_task_let_go_of) {
        return;
    }
    let page = app
        .model
        .layout
        .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
        .map(|(page, _)| page);
    if let Some(page) = page {
        app.close_pane(page);
    }
}

/// Open the box that renames a card, on the second click of a double one.
fn open_rename(app: &mut App, task_id: &str) {
    let Some(task) = app
        .model
        .board
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .cloned()
    else {
        return;
    };
    // The first of the two clicks opened the task's tab and promised it the keyboard. The box
    // being opened here is what the keyboard was reached for, so the promise is taken back - a
    // shell that is still attaching would otherwise take it frames later, out of a box that has
    // been typed into by then.
    app.pane_taking_keyboard = None;
    app.model.board.renaming = Some(crate::native::model::TaskRename {
        task_id: task.id,
        title: task.title,
        focus: true,
        title_rect: egui::Rect::NOTHING,
    });
}

fn draw_board(app: &mut App, ui: &mut Ui, palette: &Palette, actions: &mut Vec<BoardAction>) {
    if let Some(error) = app.model.board.error.clone() {
        ui.label(RichText::new(error).color(palette.warn));
        ui.add_space(4.0);
    }
    if !app.model.board.loaded {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("reading .moontasks…").color(palette.muted));
        });
        return;
    }

    // Escape lets the marks go, for a hand already on the keyboard. Only while there are marks
    // to let go of, so the key is still the filter box's and the palette's the rest of the
    // time, and not while a box is being typed into, where Escape means what that box says.
    if !app.model.board.marked.is_empty()
        && !ui.ctx().text_edit_focused()
        && !app.model.palette.open
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        selection::let_go_of_all(&mut app.model.board);
    }

    // Over the columns rather than inside one: the query is asked of the whole board, and
    // every column answers it.
    filter::draw(app, ui, palette, actions);

    // A press beside the columns is a press on the board too. Claimed after the columns have
    // drawn, at the foot of this function, so a card or a column has first refusal.
    let board_rect = ui.available_rect_before_wrap();
    // And nothing outside this is the board's at all, however far a column's cards are laid
    // out past it.
    app.model.board.showing = Some(board_rect);

    // The columns are as tall as the pane, and the board scrolls sideways to reach the ones
    // that do not fit - measured before the scroll area, which has no height of its own.
    let height = ui.available_height();
    // One wheel or trackpad gesture moves the board sideways or a column up and down, never
    // both at once: a nested pair of scroll areas would otherwise take a component each and
    // send the board off diagonally. Held for the length of the gesture, so a flick that
    // starts across does not swap axis halfway through as the fingers drift.
    scroll_under_the_finger(app, ui);
    let held_back = hold_the_off_axis(app, ui);
    let columns = ScrollArea::horizontal()
        .id_salt("moontasks-columns")
        // Dragging is how a column is moved, so it must not also mean "scroll the board".
        .scroll_source(egui::containers::scroll_area::ScrollSource {
            drag: egui::containers::scroll_area::DragScroll::Never,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.horizontal_top(|ui| draw_column_row(app, ui, height, palette, actions));
            ui.min_rect()
        })
        .inner;
    held_back.give_it_back(ui);

    // Beside or below the columns, where nothing of the board's is drawn. A press in a column
    // that no card wanted has already been claimed by the column itself; this is the rest of
    // the board, and a press on the board is the marks being let go of.
    let beside_the_columns = egui::Rect::from_min_max(
        egui::pos2(board_rect.left().max(columns.right()), board_rect.top()),
        board_rect.max,
    );
    let under_the_columns = egui::Rect::from_min_max(
        egui::pos2(board_rect.left(), board_rect.top().max(columns.bottom())),
        board_rect.max,
    );
    for empty in [beside_the_columns, under_the_columns] {
        gesture::claim(
            &mut app.model.board,
            ui,
            empty,
            None,
            egui::Rect::NOTHING,
            false,
        );
    }
}

/// The place in the row the new-column box is standing in, at the width a column has.
fn draw_new_column_slot(
    app: &mut App,
    ui: &mut Ui,
    height: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    ui.allocate_ui_with_layout(
        vec2(COLUMN_WIDTH, height),
        UiLayout::top_down(Align::Min),
        |ui| columns::draw_new_column(app, ui, palette, actions),
    );
    ui.add_space(6.0);
}

/// The row of columns, and the drag that reorders it.
fn draw_column_row(
    app: &mut App,
    ui: &mut Ui,
    height: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let carried = egui::DragAndDrop::payload::<columns::DraggedColumn>(ui.ctx());
    let dragged = carried.as_deref().map(|carried| carried.0.clone());
    // What a column's place is measured against, so scrolling the board sideways is not read
    // as every column having moved.
    let origin = ui.min_rect().left();
    let order = columns::ordered_columns(app, dragged.as_ref());

    // Where each column ended up, for working out what a dragged one is being held over. The
    // dragged one is left out: it is on the cursor rather than where it was laid out.
    let mut headings: Vec<(ColumnId, f32)> = Vec::new();
    for (at, column) in order.iter().enumerate() {
        // The box for a column being named stands where that column will go, so the gap it
        // leaves is the gap the column will fill. Out of the row while one is being dragged,
        // for the same reason the `+` at the end is.
        if dragged.is_none() && columns::composer_stands_at(app, at) {
            draw_new_column_slot(app, ui, height, palette, actions);
        }
        let rect = columns::with_column_drag(app, ui, column, origin, |app, ui| {
            draw_column(app, ui, column, height, palette, actions)
        });
        if Some(&column.id) != dragged.as_ref() {
            headings.push((column.id.clone(), rect.center().x));
        }
        ui.add_space(6.0);
    }

    // At the right-hand end, where a new column would go - and out of the way while one is
    // being dragged, so it is never the thing a column is dropped onto.
    if dragged.is_none() {
        // The box is here when it was opened here, and also when the place it was opened at is
        // no longer on the board - a column it stood beside removed while it was open. It has
        // to be drawn somewhere: a box nobody can see is a box nobody can close.
        if columns::composer_is_past(app, order.len()) {
            draw_new_column_slot(app, ui, height, palette, actions);
        } else if !app.model.board.column_composer_open {
            ui.allocate_ui_with_layout(
                vec2(COLUMN_WIDTH, height),
                UiLayout::top_down(Align::Min),
                |ui| columns::draw_new_column_plus(ui, palette, actions),
            );
        }
    }

    let Some(dragged) = dragged else {
        app.model.board.column_landing = None;
        return;
    };
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };

    let at = columns::landing_for(pointer.x, &headings);
    // Read by the next frame, which draws the column in this place rather than the one it was
    // picked up from.
    app.model.board.column_landing = Some(at);

    if ui.input(|input| input.pointer.any_released())
        && egui::DragAndDrop::take_payload::<columns::DraggedColumn>(ui.ctx()).is_some()
    {
        app.model.board.column_landing = None;
        app.model.board.pending_column_place = Some(PendingColumnPlace {
            column_id: dragged.clone(),
            index: at,
        });
        columns::place_column_in(&mut app.model.board.columns, &dragged, at);
        actions.push(BoardAction::PlaceColumn(dragged, at));
    }
}

// The board grew these marks first and the rest of the window took them up, so they live in
// [`widgets`] now and keep their old names here.
pub(super) use crate::native::widgets::{close_button, close_mark};

/// The agents this machine has, with "None" first for a task started without one.
pub(crate) fn available_agents(app: &App) -> Vec<AgentKind> {
    let mut agents = vec![AgentKind::None];
    let session_id = app.model.root_session_id.clone();
    if let Some(payload) = app
        .model
        .review_ref(&session_id)
        .and_then(|review| review.payload.as_ref())
    {
        agents.extend(
            payload
                .available_agents
                .iter()
                .filter(|option| option.available && option.kind != AgentKind::None)
                .map(|option| option.kind),
        );
    }
    agents
}

pub(super) fn agent_label(agent: AgentKind) -> String {
    match agent {
        AgentKind::None => "no agent".to_string(),
        other => other.label().to_lowercase(),
    }
}

/// Whether the board is open, which is what decides if it is worth polling.
pub(crate) fn is_open(app: &App) -> bool {
    // A task's own pane is drawn from the same answer, so it counts as the board being open:
    // it says what the task has running, and a pane that is never read again would go on
    // saying whatever was true when it opened.
    app.model
        .layout
        .find_pane(|pane| matches!(pane.kind(), PaneKind::Tasks | PaneKind::Start))
        .is_some()
}
