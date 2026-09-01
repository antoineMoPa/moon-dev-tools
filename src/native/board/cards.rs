//! The cards on the board: what one shows, and the drag that moves it between columns.
//!
//! A card is picked up by its title and drawn into a layer of its own while it is in flight,
//! so the buttons it carries stay clickable and the slot it is being held over shows through
//! it. Where it would land is worked out from the cards it would be put among rather than
//! from what the pointer is over, which is what keeps the answer from bouncing.

use egui::{Align, CornerRadius, Layout as UiLayout, Ui, vec2};

use crate::{
    moontasks::{ColumnId, TaskView},
    native::{
        app::App,
        board::{
            Axis, BoardAction, CLOSE_MARK_SIZE, actions::TaskPaneBox, close_button,
            filter::Filter, resources, slide_into_place, stamp_place, start,
        },
        model::Model,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// The task a drag is carrying. A type of its own rather than the bare id, so a column only
/// lights up for a card and never for whatever else the window might one day let go of.
#[derive(Clone)]
pub(super) struct DraggedTask(pub(super) String);

/// How solid the card under the cursor is while it is being dragged. Enough of it to read,
/// little enough that the slot it is being held over shows through it.
pub(super) const DRAGGED_CARD_OPACITY: f32 = 0.5;

/// The gap between two cards in a column.
pub(super) const CARD_SPACING: f32 = 5.0;

/// How long a card that has just been dropped stays marked, in seconds.
const DROP_FLASH: f32 = 1.2;

/// How long a card's offers take to come up under the pointer and go again, in seconds.
const OFFER_FADE: f32 = 0.15;

/// Where a card's actions row records whether its menu is up, for the card to read next frame.
fn menu_up_id(drag_id: egui::Id) -> egui::Id {
    drag_id.with("menu-up")
}

/// How many lines of a card's title are shown before the rest is cut. Enough for a sentence
/// of a task name, short enough that one long title does not push every card down the column.
const TITLE_ROWS: usize = 3;

/// The cards of one column, in the order they are drawn.
///
/// A card being dragged is one of them from the moment it is over the column, and no longer
/// one of the column it came from: the board makes the move as it is being made rather than
/// once it is over, so nothing jumps when the card is let go of.
///
/// The board's filter is applied here, so a column shows the cards that match it and keeps
/// them in the order it holds them in.
pub(super) fn column_cards(
    app: &App,
    status: &ColumnId,
    dragged_id: Option<&str>,
) -> Vec<TaskView> {
    let landing = app.model.board.landing.clone();
    // Until the pointer has been over a column there is nowhere for the card to be but where
    // it came from, and taking it out of the board for that first frame reads as a flicker.
    let taken = landing.is_some();
    let filter = Filter::of(&app.model.board.filter);

    let mut tasks: Vec<TaskView> = app
        .model
        .board
        .tasks
        .iter()
        .filter(|task| {
            task.status == *status
                && filter.matches(task)
                && !(taken && Some(task.id.as_str()) == dragged_id)
        })
        .cloned()
        .collect();

    if let Some(landing) = landing.filter(|landing| landing.status == *status)
        && let Some(dragged) = dragged_id.and_then(|id| {
            app.model
                .board
                .tasks
                .iter()
                .find(|task| task.id == id)
                .cloned()
        })
    {
        tasks.insert(landing.index.min(tasks.len()), dragged);
    }
    tasks
}

/// Where a card let go of among the cards a filter is showing belongs in the column itself.
///
/// A drop is read against what is on screen: let go of above the third card showing means
/// above that card, whatever the filter is hiding between it and the one before. Let go of
/// below the last card showing means the end of the column, the same as it does with no filter
/// on - and with no filter on the two indexes are the same, so nothing is translated at all.
///
/// The dragged card is left out of the reckoning, because [`place_in`] takes it out of the
/// column before it counts places in it.
pub(super) fn column_index_of(
    tasks: &[TaskView],
    filter: &Filter,
    status: &ColumnId,
    dragged_id: &str,
    showing_index: usize,
) -> usize {
    if !filter.is_on() {
        return showing_index;
    }

    let column: Vec<&TaskView> = tasks
        .iter()
        .filter(|task| task.status == *status && task.id != dragged_id)
        .collect();

    let mut showing = 0;
    for (at, task) in column.iter().enumerate() {
        if !filter.matches(task) {
            continue;
        }
        if showing == showing_index {
            return at;
        }
        showing += 1;
    }
    column.len()
}

/// How many cards a column holds, filter or no filter - what the board would show if the query
/// were emptied. The heading's delete mark goes by this: a column whose cards are only hidden
/// is still the record of where they are.
pub(super) fn column_size(tasks: &[TaskView], status: &ColumnId) -> usize {
    tasks.iter().filter(|task| task.status == *status).count()
}

/// The id a card is dragged by, which is also the layer its ghost is drawn into.
pub(crate) fn card_drag_id(task_id: &str) -> egui::Id {
    egui::Id::new(("moontask-card", task_id))
}

/// Take a board the server has answered with, with a drop that it may not have seen yet.
///
/// A read that was already on its way when a card was dropped answers with the card where it
/// was, so the drop is made again on top of it - until an answer comes back with the card
/// where it was put, which is the server having caught up.
pub(crate) fn accept_board(model: &mut Model, mut tasks: Vec<TaskView>) {
    if let Some(pending) = &model.board.pending_place {
        let column: Vec<&TaskView> = tasks
            .iter()
            .filter(|task| task.status == pending.status)
            .collect();
        let landed = column
            .iter()
            .position(|task| task.id == pending.task_id)
            .is_some_and(|at| at == pending.index.min(column.len().saturating_sub(1)));
        if landed {
            model.board.pending_place = None;
        } else {
            let (task_id, status, index) = (
                pending.task_id.clone(),
                pending.status.clone(),
                pending.index,
            );
            place_in(&mut tasks, &task_id, &status, index);
        }
    }
    model.board.tasks = tasks;
}

/// Make the move on the board being drawn, ahead of the server being told about it.
///
/// What the board draws is the last answer the server gave, and the next one is a worker
/// thread and a poll away. Without this the dropped card springs back to where it came from
/// for those few frames and then moves again - which reads as the drop having failed.
pub(super) fn place_in(tasks: &mut Vec<TaskView>, task_id: &str, status: &ColumnId, index: usize) {
    let Some(at) = tasks.iter().position(|task| task.id == task_id) else {
        return;
    };
    let mut moved = tasks.remove(at);
    moved.status = status.clone();

    // Where that place is in the one list the board keeps every column's cards in.
    let column: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.status == *status)
        .map(|(at, _)| at)
        .collect();
    let into = match column.get(index) {
        Some(&at) => at,
        None => column.last().map_or(tasks.len(), |&at| at + 1),
    };
    tasks.insert(into, moved);
}

/// The hole a dragged card leaves where it is going to land: the card itself is drawn at the
/// cursor, and this is the shape of the space being held for it.
pub(super) fn draw_empty_slot(ui: &Ui, slot: egui::Rect, palette: &Palette) {
    ui.painter().rect(
        slot,
        CornerRadius::same(6),
        palette.control_bg,
        egui::Stroke::new(1.0, palette.accent),
        egui::StrokeKind::Inside,
    );
}

/// One card, and the drag that carries it between columns.
///
/// The card is picked up by its title, but what moves is the whole box: while a drag is in
/// flight the card is drawn into a layer of its own and that layer is moved to the cursor,
/// which is how `egui`'s own drag sources work. Sensing the drag on the title alone is what
/// leaves the buttons underneath clickable - anything sensing a drag claims everything under
/// it.
///
/// Answers with the place the card was laid out in, which is what a drop is measured against:
/// a dragged card is drawn at the cursor but keeps its place in the column.
pub(super) fn draw_card(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    origin: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    let drag_id = card_drag_id(&task.id);
    if !ui.ctx().is_being_dragged(drag_id) {
        return slide_into_place(ui, Axis::Vertical, drag_id, origin, |ui| {
            draw_card_body(app, ui, task, drag_id, palette, actions)
        });
    }

    egui::DragAndDrop::set_payload(ui.ctx(), DraggedTask(task.id.clone()));

    // A card being carried is at the slot it is being held over, as far as anything that
    // remembers where cards are is concerned - the drawing is at the cursor, but the place is
    // the slot. Kept up to date rather than left at the slot the card was picked up from,
    // which is where the card would otherwise be seen to slide back from when it is let go of.
    stamp_place(ui, Axis::Vertical, drag_id, origin);

    let layer_id = egui::LayerId::new(egui::Order::Tooltip, drag_id);
    let card = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
            // A ghost of the card rather than the card: it is drawn at the cursor, which is
            // exactly where the line saying where it will land is, and one of the two has to
            // be seen through.
            ui.set_opacity(DRAGGED_CARD_OPACITY);
            draw_card_body(app, ui, task, drag_id, palette, actions);
        })
        .response;

    // The card is laid out where it belongs and then moved: a widget has to have a place
    // before it can be drawn, and nothing in a dragged card is interactive anyway.
    if let Some(pointer) = ui.ctx().pointer_interact_pos() {
        ui.ctx().transform_layer_shapes(
            layer_id,
            egui::emath::TSTransform::from_translation(pointer - card.rect.center()),
        );
    }
    card.rect
}

/// A card's border: the ordinary one, unless the card is the task being worked in, or was
/// dropped a moment ago.
///
/// The drop flash comes first of the two because it is the shorter-lived: a card dropped onto
/// the task you are working in is being told two things at once, and the one that only has a
/// second to say it goes first.
fn card_stroke(app: &App, ui: &Ui, task: &TaskView, palette: &Palette) -> egui::Stroke {
    if let Some(dropped) = dropped_stroke(app, ui, task, palette) {
        return dropped;
    }
    if app.worked_in_task() == Some(task.id.as_str()) {
        return egui::Stroke::new(CARD_BORDER_WIDTH, palette.accent);
    }
    egui::Stroke::new(CARD_BORDER_WIDTH, palette.line)
}

/// How heavy a card's border is, marked or not. The task being worked in is told apart by the
/// color of its edge alone: a heavier one would take its width out of the card's inside and
/// walk the whole column along by a pixel every time the tab in front changed.
const CARD_BORDER_WIDTH: f32 = 1.0;

/// The border of a card that was dropped a moment ago, if it was.
///
/// A card let go of among a column of others is easy to lose track of, so the one that just
/// landed is marked and fades back over [`DROP_FLASH`]. It is a fade rather than a mark that
/// is cleared: nothing has to remember to put it back.
fn dropped_stroke(app: &App, ui: &Ui, task: &TaskView, palette: &Palette) -> Option<egui::Stroke> {
    let dropped = app.model.board.dropped.as_ref()?;
    if dropped.task_id != task.id {
        return None;
    }
    let left = (DROP_FLASH - (ui.input(|input| input.time) - dropped.at) as f32) / DROP_FLASH;
    if left <= 0.0 {
        return None;
    }
    // The fade is drawn frame by frame, so it needs frames to be drawn in.
    ui.ctx().request_repaint();
    Some(egui::Stroke::new(
        CARD_BORDER_WIDTH,
        palette.line.lerp_to_gamma(palette.warn, left),
    ))
}

fn draw_card_body(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    egui::Frame::new()
        .fill(palette.panel)
        .stroke(card_stroke(app, ui, task, palette))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // What a card offers to start is only drawn while the pointer is on the card,
            // and comes up and goes over [`OFFER_FADE`] rather than at once. Read from the
            // card's own background, which the buttons drawn over it do not take the pointer
            // away from, so pointing at one of them still counts as being on the card - and
            // so does having its menu up, which hangs below the card and would otherwise
            // fade the card out from under the hand reaching into it. That one is read from
            // the frame before, because whether the menu is up is only known once the row
            // that opens it has been drawn.
            let holding = ui
                .data(|data| data.get_temp::<bool>(menu_up_id(drag_id)))
                .unwrap_or(false);
            let pointed_at = holding || ui.response().contains_pointer();
            let showing = ui
                .ctx()
                .animate_bool_with_time(drag_id.with("offers"), pointed_at, OFFER_FADE);
            draw_card_title(app, ui, task, drag_id, palette, actions);
            ui.add_space(3.0);
            draw_notes_box(ui, task, palette, showing, actions);
            ui.add_space(3.0);

            resources::draw_list(app, ui, task, palette, actions);
            if !task.resources.is_empty() {
                ui.add_space(3.0);
            }

            draw_card_actions(app, ui, task, drag_id, showing, actions);
        })
        .response
        .rect
}

/// The card's title - the handle it is dragged by, the box it is renamed in, and the mark
/// that deletes it, which sits up here the way a tab's close mark does.
fn draw_card_title(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let pending_delete = app.model.board.pending_delete.as_deref() == Some(task.id.as_str());
    let editing = app
        .model
        .board
        .renaming
        .as_ref()
        .is_some_and(|rename| rename.task_id == task.id);
    let handle_width = ui.available_width() - CLOSE_MARK_SIZE - ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        if editing {
            draw_title_editor(app, ui, task, handle_width, actions);
        } else {
            draw_title_handle(app, ui, task, drag_id, handle_width, palette, actions);
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            // The folder and everything an agent left in it goes, so the cross asks first -
            // the same two-press shape discarding a hunk has.
            if pending_delete {
                match widgets::confirm(
                    ui,
                    palette,
                    "[really delete]",
                    "this deletes the task folder and everything in it, and cannot be undone",
                ) {
                    widgets::Confirmed::Yes => {
                        app.model.board.pending_delete = None;
                        actions.push(BoardAction::Delete(task.id.clone()));
                    }
                    widgets::Confirmed::No => app.model.board.pending_delete = None,
                    widgets::Confirmed::Waiting => {}
                }
            } else if close_button(ui, palette)
                .on_hover_text("Delete this task and its folder")
                .clicked()
            {
                app.model.board.pending_delete = Some(task.id.clone());
            }
        });
    });
}

/// The title as it usually reads: what the card is dragged by, what a click goes to the task
/// by, and what a double click opens for renaming.
fn draw_title_handle(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    handle_width: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let width = handle_width.max(0.0);
    // Cut rather than wrapped without end: a card sits in a column of a fixed width, and a
    // title long enough to need a fourth line used to widen the whole column to fit it.
    let title = widgets::cut_to_fit(
        ui,
        &task.title,
        egui::TextStyle::Body.resolve(ui.style()),
        palette.ink,
        width,
        TITLE_ROWS,
    );
    let laid_out = ui
        .scope(|ui| {
            // The whole width up to the close mark, so the card is easy to grab rather than
            // only grabbable on the letters of its title.
            ui.set_min_width(width);
            ui.add(egui::Label::new(title).selectable(false));
        })
        .response;

    let handle = ui
        .interact(laid_out.rect, drag_id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        // The title in full, since the card may only have room for the start of it.
        .on_hover_text(format!(
            "{}\n\n{}\n\nClick to open this task",
            task.title, task.dir_path
        ));

    // Acted on the moment it lands, rather than held to see whether a second click is coming:
    // a wait would be felt on every click for the sake of the few that turn out to be renames.
    //
    // The pane it opens is the whole answer, whatever the task has running - not the agent,
    // even when there is one. A click that sometimes landed in a terminal instead would be one
    // you had to know the task's state to predict; the runs are listed on the card, each its
    // own way back to its own shell.
    if handle.clicked() {
        actions.push(BoardAction::OpenStart {
            task_id: task.id.clone(),
            title: task.title.clone(),
            opens_on: TaskPaneBox::Neither,
        });
    }

    if handle.double_clicked() {
        // The first of the two clicks opened the task's tab and promised it the keyboard. The
        // box being opened here is what the keyboard was reached for, so the promise is taken
        // back - a shell that is still attaching would otherwise take it frames later, out of
        // a box that has been typed into by then.
        app.pane_taking_keyboard = None;
        app.model.board.renaming = Some(crate::native::model::TaskRename {
            task_id: task.id.clone(),
            title: task.title.clone(),
            focus: true,
            title_rect: laid_out.rect,
        });
    }
}

/// The title being renamed. Enter and clicking away keep it, Escape throws it away.
fn draw_title_editor(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    handle_width: f32,
    actions: &mut Vec<BoardAction>,
) {
    let Some(rename) = &mut app.model.board.renaming else {
        return;
    };
    let entry = ui.add_sized(
        vec2(handle_width.max(40.0), ui.spacing().interact_size.y),
        egui::TextEdit::singleline(&mut rename.title).hint_text("Task title"),
    );
    if std::mem::take(&mut rename.focus) {
        entry.request_focus();
    }

    // The third click of the triple that opened this box selects the whole title. That click
    // is routed against the frame before the box existed - the double click's frame, where the
    // title was still a label - so the box's own response never hears it and it is read off
    // the pointer itself instead.
    let tripled = ui.input(|input| {
        input
            .pointer
            .button_triple_clicked(egui::PointerButton::Primary)
            && input.pointer.interact_pos().is_some_and(|pos| {
                entry.rect.contains(pos) || rename.title_rect.contains(pos)
            })
    });
    if tripled {
        let mut state =
            egui::text_edit::TextEditState::load(ui.ctx(), entry.id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(rename.title.chars().count()),
            )));
        // Stored after the box's own end-of-frame store, so this range is the one it loads
        // next frame - and focused, since a box without the keyboard drops its selection as
        // it loads.
        state.store(ui.ctx(), entry.id);
        entry.request_focus();
    }

    let title = rename.title.clone();
    let abandon = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let keep = entry.lost_focus() && !abandon;

    if keep && !title.trim().is_empty() && title != task.title {
        actions.push(BoardAction::Rename(task.id.clone(), title));
    } else if abandon || keep {
        actions.push(BoardAction::CancelRename);
    }
}

/// How many lines of a task's notes the card shows before the rest is cut. The card is the
/// description at a glance, not the whole file - that is what the notes pane is for.
const NOTES_ROWS: usize = 3;

/// The first lines of the task's `notes.md` under the title - its description. A task with
/// none offers the link that starts them. Either way a click opens the task's own pane with
/// the keyboard in its notes box, rather than the file beside it: the pane is where the notes
/// are written now, and a file open on the same words is a second place for them to be typed.
///
/// The offer is worth its row only while the pointer is on the card, but a card that dropped
/// the row when it is not would change height under the pointer as it crossed the column. So
/// the button keeps its place either way, and what fades over [`OFFER_FADE`] is how much of
/// it is drawn - `showing` is 0 on a card at rest and 1 on the one under the pointer.
fn draw_notes_box(
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    showing: f32,
    actions: &mut Vec<BoardAction>,
) {
    let opens_the_notes = || BoardAction::OpenStart {
        task_id: task.id.clone(),
        title: task.title.clone(),
        opens_on: TaskPaneBox::Notes,
    };

    let notes = task.notes.trim();
    if notes.is_empty() {
        ui.scope(|ui| {
            // Multiplied rather than set, so the ghost of a card being dragged stays a ghost.
            ui.multiply_opacity(showing);
            if widgets::quiet_button_colored(ui, "[add notes]", palette.muted)
                .on_hover_text("Write this task's notes.md, shared with its agents")
                .clicked()
            {
                actions.push(opens_the_notes());
            }
        });
        return;
    }

    let preview = widgets::cut_to_fit(
        ui,
        notes,
        egui::FontId::proportional(SMALL_SIZE),
        palette.muted,
        ui.available_width(),
        NOTES_ROWS,
    );
    if ui
        .add(
            egui::Label::new(preview)
                .selectable(false)
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open this task, with its notes ready to write")
        .clicked()
    {
        actions.push(opens_the_notes());
    }
}

/// Everything a card starts, on the one menu - the button and what is under it are
/// [`start::draw_button`], the same ones the start window shows.
///
/// It comes up with the notes offer above it and goes the same way, so a card at rest is its
/// title and its description and nothing else. It sits at the bottom right, out of the way of
/// the description the card is read by, and under the mark that deletes the card - the two
/// ends of the card are what it is acted on from.
fn draw_card_actions(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    showing: f32,
    actions: &mut Vec<BoardAction>,
) {
    // A row of its own, the width of the card and no taller than the button, and the button
    // laid out from the right-hand end of it. The row has to be allocated rather than laid
    // out into what is left: a right-to-left layout given the rest of the column takes the
    // rest of the column, and the button would come to rest at the foot of it.
    let row = vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, UiLayout::right_to_left(Align::Center), |ui| {
        // Multiplied rather than set, so the ghost of a card being dragged stays a ghost.
        ui.multiply_opacity(showing);
        let menu_up = start::draw_button(app, ui, task, actions);
        // Told to the card, which keeps its offers out for as long as this is up.
        ui.data_mut(|data| data.insert_temp(menu_up_id(drag_id), menu_up));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column of cards, each named after what a query would find it by.
    fn column(titles: &[&str]) -> Vec<TaskView> {
        titles
            .iter()
            .map(|title| TaskView {
                id: format!("{title}-1111"),
                title: title.to_string(),
                status: ColumnId::new("todo"),
                created_at_unix: 1700000000,
                dir_path: String::new(),
                repo_path: String::new(),
                notes: String::new(),
                resources: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn with_no_filter_on_a_drop_lands_where_it_was_let_go_of() {
        let tasks = column(&["one", "two", "three"]);
        let status = ColumnId::new("todo");
        let nothing = Filter::of("");

        for at in 0..3 {
            assert_eq!(
                column_index_of(&tasks, &nothing, &status, "two-1111", at),
                at
            );
        }
    }

    /// The cards a filter hides are still in the column, and a drop is read against the ones
    /// on screen: above the card showing that was dropped above.
    #[test]
    fn a_drop_among_filtered_cards_lands_above_the_card_it_was_dropped_above() {
        // Showing: `sing` at column place 1, and `song` at column place 3.
        let tasks = column(&["hidden", "sing", "also hidden", "song"]);
        let status = ColumnId::new("todo");
        let filter = Filter::of("s\u{69}ng");
        let dragged = "song-1111";

        assert_eq!(
            column_index_of(&tasks, &filter, &status, dragged, 0),
            1,
            "above the first card showing is above that card, not the top of the column"
        );
        assert_eq!(
            column_index_of(&tasks, &filter, &status, dragged, 1),
            3,
            "below the last card showing is the end of the column, hidden cards and all"
        );
    }

    /// The card being dragged is not one of the places it can be dropped into: `place_in`
    /// takes it out of the column before it counts places in it.
    #[test]
    fn the_dragged_card_is_left_out_of_the_places_it_could_land_in() {
        let tasks = column(&["sing", "hidden", "song"]);
        let status = ColumnId::new("todo");
        let filter = Filter::of("s");

        // With `sing` in the air, the only card showing is `song`, at place 1 of the two the
        // column has left - so the first place a drop can take is that one, not `sing`'s.
        assert_eq!(column_index_of(&tasks, &filter, &status, "sing-1111", 0), 1);
        assert_eq!(
            column_index_of(&tasks, &filter, &status, "sing-1111", 1),
            2,
            "and past it is the end of what the column has left"
        );
    }

    #[test]
    fn a_columns_size_counts_the_cards_a_filter_is_hiding_too() {
        let mut tasks = column(&["one", "two"]);
        tasks.push(TaskView {
            status: ColumnId::new("done"),
            ..column(&["three"]).remove(0)
        });

        assert_eq!(column_size(&tasks, &ColumnId::new("todo")), 2);
        assert_eq!(column_size(&tasks, &ColumnId::new("done")), 1);
    }
}
