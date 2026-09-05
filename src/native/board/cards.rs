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
            Axis, BoardAction, CLOSE_MARK_SIZE, actions::TaskPaneBox, close_button, filter::Filter,
            gesture, resources, selection, slide_into_place, stamp_place, start,
        },
        model::Model,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How solid the card under the cursor is while it is being dragged. Enough of it to read,
/// little enough that the slot it is being held over shows through it.
pub(super) const DRAGGED_CARD_OPACITY: f32 = 0.5;

/// The gap between two cards in a column.
pub(super) const CARD_SPACING: f32 = 5.0;

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
pub(super) fn column_cards(app: &App, status: &ColumnId) -> Vec<TaskView> {
    let landing = app.model.board.landing.clone();
    let carrying = app.model.board.carrying.as_ref();
    // Until the pointer has been over a column there is nowhere for the cards to be but where
    // they came from, and taking them out of the board for that first frame reads as a flicker.
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
                && !(taken && carrying.is_some_and(|carrying| carrying.carries(&task.id)))
        })
        .cloned()
        .collect();

    // The cards being carried land as a run, in the order the board holds them, so the column
    // they are over shows them that way while they are being held over it.
    if let Some(landing) = landing.filter(|landing| landing.status == *status)
        && let Some(carrying) = carrying
    {
        let carried = app
            .model
            .board
            .tasks
            .iter()
            .filter(|task| carrying.carries(&task.id))
            .cloned();
        let at = landing.index.min(tasks.len());
        for (offset, task) in carried.enumerate() {
            tasks.insert(at + offset, task);
        }
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
/// The cards being dragged are left out of the reckoning, because [`place_in`] takes them out
/// of the column before it counts places in it.
pub(super) fn column_index_of(
    tasks: &[TaskView],
    filter: &Filter,
    status: &ColumnId,
    dragged_ids: &[String],
    showing_index: usize,
) -> usize {
    if !filter.is_on() {
        return showing_index;
    }

    let column: Vec<&TaskView> = tasks
        .iter()
        .filter(|task| task.status == *status && !dragged_ids.contains(&task.id))
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
    close_the_pages_of_deleted_tasks(model, &tasks);
    let Some(pending) = &model.board.pending_place else {
        model.board.tasks = tasks;
        return;
    };
    let column: Vec<&TaskView> = tasks
        .iter()
        .filter(|task| task.status == pending.status)
        .collect();
    // Landed once the run of dropped cards is where it was dropped and in the order it was
    // dropped in: the last of them can be no further down than the end of the column.
    let first = pending
        .index
        .min(column.len().saturating_sub(pending.task_ids.len()));
    let landed = pending
        .task_ids
        .iter()
        .enumerate()
        .all(|(offset, task_id)| {
            column
                .get(first + offset)
                .is_some_and(|task| task.id == *task_id)
        });

    if landed {
        model.board.pending_place = None;
    } else {
        let (task_ids, status, index) = (
            pending.task_ids.clone(),
            pending.status.clone(),
            pending.index,
        );
        place_in(&mut tasks, &task_ids, &status, index);
    }
    model.board.tasks = tasks;
}

/// A task the board had and no longer has has been deleted - here, or in `.moontasks` by hand -
/// and its page goes with it: a tab standing there saying the task is gone is a tab you have to
/// close yourself.
///
/// Read against the answer before it rather than against the tab: a task created a moment ago
/// is in no answer yet, and a page opened on it must not be closed for the read that was
/// already on its way when it was made.
fn close_the_pages_of_deleted_tasks(model: &mut Model, tasks: &[TaskView]) {
    let deleted: Vec<String> = model
        .board
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .filter(|task_id| !tasks.iter().any(|task| task.id == *task_id))
        .collect();
    // Queued rather than closed here, the way a card let go of queues its page: the answer is
    // taken while the window is being drawn, and a pane is never closed while the tree that
    // holds it is being drawn.
    model.board.pages_to_close.extend(deleted);
}

/// Make the move on the board being drawn, ahead of the server being told about it.
///
/// What the board draws is the last answer the server gave, and the next one is a worker
/// thread and a poll away. Without this the dropped card springs back to where it came from
/// for those few frames and then moves again - which reads as the drop having failed.
pub(super) fn place_in(
    tasks: &mut Vec<TaskView>,
    task_ids: &[String],
    status: &ColumnId,
    index: usize,
) {
    // Taken out in the order the board had them, which is the order they go back in: a drag
    // moves a run of cards without reordering it.
    let mut moving: Vec<TaskView> = Vec::new();
    tasks.retain(|task| {
        if !task_ids.contains(&task.id) {
            return true;
        }
        moving.push(TaskView {
            status: status.clone(),
            ..task.clone()
        });
        false
    });

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
    for (offset, moved) in moving.into_iter().enumerate() {
        tasks.insert(into + offset, moved);
    }
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

/// The card being written on the new-task pane, in the place it will take once `[create]` is
/// pressed: an empty card, outlined the way the hole a dragged card leaves is, because it means
/// the same thing - a card is going here.
///
/// It stands at whichever end of the column the `+` that opened the pane was, so a task is
/// written with its place on the board already in front of you rather than appearing somewhere
/// once it is made. Nothing is drawn in it: what would be in it is being typed on the pane.
pub(super) fn draw_pending_card(
    ui: &mut Ui,
    palette: &Palette,
    controls: &mut gesture::Controls,
    actions: &mut Vec<BoardAction>,
) {
    let (_, slot) = ui.allocate_space(vec2(ui.available_width(), PENDING_CARD_HEIGHT));
    draw_empty_slot(ui, slot, palette);

    // The cross a made card carries, in the place it sits there: the empty card is the task
    // as it stands, and this is how it is said no to without going looking for the tab it is
    // being written on. Nothing has been made yet, so it asks nothing first - what goes is
    // the writing on the pane.
    let mark = egui::Rect::from_min_size(
        slot.right_top()
            + vec2(
                -CLOSE_MARK_SIZE - f32::from(CARD_MARGIN.right),
                f32::from(CARD_MARGIN.top),
            ),
        vec2(CLOSE_MARK_SIZE, CLOSE_MARK_SIZE),
    );
    let mut mark_ui = ui.new_child(egui::UiBuilder::new().max_rect(mark));
    let cross = widgets::close_button(&mut mark_ui, palette).on_hover_text("Discard this task");
    if controls.pressed(&cross) {
        actions.push(BoardAction::CancelNewTask);
    }
}

/// How tall the empty card is: about what a card with a title of one line and nothing else on
/// it comes out at, so the space it holds is the space the card will want.
pub(crate) const PENDING_CARD_HEIGHT: f32 = 78.0;

/// The inside margin a card's frame keeps. The empty card has no frame of its own, and places
/// its cross by this so the mark stands where a made card's does.
const CARD_MARGIN: egui::Margin = egui::Margin::symmetric(8, 7);

/// One card: what it shows, the press it claims, and the drag that carries it.
///
/// A card claims a press that lands on it and on none of its own buttons - that is the whole
/// of the interaction, and [`gesture`] works out afterwards whether it was a click or the card
/// being carried somewhere. While it is being carried it is drawn into a layer of its own and
/// that layer is moved to the cursor, the way `egui`'s own drag sources do it.
///
/// Answers with the place the card was laid out in, which is what a drop is measured against:
/// a card on the cursor keeps its place in the column.
pub(super) fn draw_card(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    origin: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    let drag_id = card_drag_id(&task.id);
    let carrying = app.model.board.carrying.clone();
    let on_the_cursor = carrying
        .as_ref()
        .is_some_and(|carrying| carrying.primary == task.id);

    if !on_the_cursor {
        // One of the others being carried is not on the cursor, but it is on its way with the
        // one that is, so it is drawn where it is going and as faint as the ghost leading it
        // there.
        let carried = carrying.is_some_and(|carrying| carrying.carries(&task.id));
        return slide_into_place(ui, Axis::Vertical, drag_id, origin, |ui| {
            if carried {
                ui.multiply_opacity(DRAGGED_CARD_OPACITY);
            }
            draw_card_body(app, ui, task, drag_id, palette, actions)
        });
    }

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

    let carried = app
        .model
        .board
        .carrying
        .as_ref()
        .map_or(1, |carrying| carrying.task_ids.len());
    if carried > 1 {
        draw_carried_count(ui, layer_id, card.rect, carried, palette);
    }

    // The card is laid out where it belongs and then moved: a widget has to have a place
    // before it can be drawn, and nothing in a card on the cursor is interactive anyway.
    if let Some(pointer) = ui.ctx().pointer_interact_pos() {
        ui.ctx().transform_layer_shapes(
            layer_id,
            egui::emath::TSTransform::from_translation(pointer - card.rect.center()),
        );
    }
    card.rect
}

/// How many cards the drag is carrying, on the corner of the one drawn at the cursor - the
/// others are down where they will land, and without this a hand holding three cards looks
/// exactly like one holding a single card.
///
/// Painted into the ghost's own layer so it travels with it, and at full strength rather than
/// the ghost's: it is the one thing on the card that is not a copy of what is already on the
/// board.
fn draw_carried_count(
    ui: &Ui,
    layer_id: egui::LayerId,
    card: egui::Rect,
    count: usize,
    palette: &Palette,
) {
    const RADIUS: f32 = 10.0;

    let center = card.right_top() + vec2(-RADIUS * 0.4, RADIUS * 0.4);
    let painter = ui.ctx().layer_painter(layer_id);
    painter.circle_filled(center, RADIUS, palette.accent);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        count.to_string(),
        egui::FontId::proportional(SMALL_SIZE),
        palette.panel,
    );
}

/// A card's border: the ordinary one, unless the card is marked.
fn card_stroke(app: &App, task: &TaskView, palette: &Palette) -> egui::Stroke {
    let color = if selection::is_marked(&app.model.board, &task.id) {
        palette.accent
    } else {
        palette.line
    };
    egui::Stroke::new(CARD_BORDER_WIDTH, color)
}

/// How much of the accent color a marked card's face is washed with. Enough to pick a run of
/// them out of a column at a glance, little enough that the words on them read exactly as well
/// as they did.
const MARKED_WASH: f32 = 0.14;

/// How heavy a card's border is, marked or not. A marked card is told apart by the color of its
/// edge and the wash on its face: a heavier edge would take its width out of the card's inside
/// and walk the whole column along by a pixel every time a card was marked.
const CARD_BORDER_WIDTH: f32 = 1.0;

fn draw_card_body(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    // A marked card is washed with the accent color it is outlined in, so a run of them reads
    // as one thing rather than as cards that happen to share an outline.
    let fill = if selection::is_marked(&app.model.board, &task.id) {
        palette.panel.lerp_to_gamma(palette.accent, MARKED_WASH)
    } else {
        palette.panel
    };
    let mut card = gesture::Controls::new(ui);
    let mut title_rect = egui::Rect::NOTHING;

    let rect = egui::Frame::new()
        .fill(fill)
        .stroke(card_stroke(app, task, palette))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(CARD_MARGIN)
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
            let showing =
                ui.ctx()
                    .animate_bool_with_time(drag_id.with("offers"), pointed_at, OFFER_FADE);
            title_rect = draw_card_title(app, ui, task, &mut card, palette, actions);
            ui.add_space(3.0);
            draw_notes_box(ui, task, &mut card, palette, showing, actions);
            ui.add_space(3.0);

            resources::draw_list(app, ui, task, &mut card, palette, actions);
            if !task.resources.is_empty() {
                ui.add_space(3.0);
            }

            draw_card_actions(app, ui, task, &mut card, drag_id, showing, actions);
        })
        .response
        .rect;

    // The press this card claims: one that landed on it and on none of its own buttons. What
    // it turns out to have been - a click, or the card being carried somewhere - is worked out
    // when the button comes back up, in [`super::settle_gesture`].
    //
    // A card whose title is open for renaming claims nothing: the box is what is being clicked
    // in, and it is the only thing on the card the pointer is there for.
    let renaming = matches!(&app.model.board.renaming, Some(rename) if rename.task_id == task.id);
    if !renaming {
        gesture::claim(
            &mut app.model.board,
            ui,
            rect,
            Some(task.id.clone()),
            title_rect,
            card.took_the_press(),
        );
    }
    rect
}

/// The card's title, the box it is renamed in, and the mark that deletes it, which sits up
/// here the way a tab's close mark does.
///
/// Answers with where the title was drawn, which is what a double click opens the rename box
/// from.
fn draw_card_title(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut gesture::Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    let pending_delete = app.model.board.pending_delete.as_deref() == Some(task.id.as_str());
    let editing = app
        .model
        .board
        .renaming
        .as_ref()
        .is_some_and(|rename| rename.task_id == task.id);
    let title_width = ui.available_width() - CLOSE_MARK_SIZE - ui.spacing().item_spacing.x;

    let mut title_rect = egui::Rect::NOTHING;
    ui.horizontal(|ui| {
        if editing {
            draw_title_editor(app, ui, task, card, title_width, actions);
        } else {
            title_rect = draw_title(ui, task, title_width, palette);
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
            } else if card.pressed(&close_button(ui, palette).on_hover_text("Delete this task")) {
                app.model.board.pending_delete = Some(task.id.clone());
            }
        });
    });
    title_rect
}

/// The title as it usually reads.
///
/// A label and nothing more: what a click on it does - open the task, mark the card, pick the
/// card up - is the card's business rather than the title's, and is settled from the press
/// itself. All it senses is the pointer being over it, which is what draws the whole title,
/// since the card may only have room for the start of it.
fn draw_title(ui: &mut Ui, task: &TaskView, width: f32, palette: &Palette) -> egui::Rect {
    let width = width.max(0.0);
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
            // The whole width up to the close mark, so the card reads as one thing rather
            // than as a line of text with space beside it.
            ui.set_min_width(width);
            ui.add(egui::Label::new(title).selectable(false));
        })
        .response;

    // Hover only: a label that sensed clicks would take the press the card is claiming. It
    // carries the card's own id, so the title is what anything looking for the card finds.
    ui.interact(laid_out.rect, card_drag_id(&task.id), egui::Sense::hover())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{}\n\n{}\n\nClick to open this task, double click to rename it",
            task.title, task.dir_path
        ));
    laid_out.rect
}

/// The title being renamed. Enter and clicking away keep it, Escape throws it away.
fn draw_title_editor(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut gesture::Controls,
    title_width: f32,
    actions: &mut Vec<BoardAction>,
) {
    let Some(rename) = &mut app.model.board.renaming else {
        return;
    };
    let entry = ui.add_sized(
        vec2(title_width.max(40.0), ui.spacing().interact_size.y),
        egui::TextEdit::singleline(&mut rename.title).hint_text("Task title"),
    );
    // A press in the box is the box's, so the card leaves it alone - the third click of a
    // triple lands in here, and a card that took it would open the task instead.
    card.pressed(&entry);
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
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| entry.rect.contains(pos) || rename.title_rect.contains(pos))
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
    card: &mut gesture::Controls,
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
            let offer = widgets::quiet_button_colored(ui, "[add notes]", palette.muted)
                .on_hover_text("Write this task's notes.md, shared with its agents");
            if card.pressed(&offer) {
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
    let notes = ui
        .add(
            egui::Label::new(preview)
                .selectable(false)
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open this task, with its notes ready to write");
    if card.pressed(&notes) {
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
    card: &mut gesture::Controls,
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
        let menu_up = start::draw_button(app, ui, task, card, actions);
        // Told to the card, which keeps its offers out for as long as this is up.
        ui.data_mut(|data| data.insert_temp(menu_up_id(drag_id), menu_up));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one card an ordinary drag carries, as the run every drag is read as.
    fn dragging(task_id: &str) -> Vec<String> {
        vec![task_id.to_string()]
    }

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
                column_index_of(&tasks, &nothing, &status, &dragging("two-1111"), at),
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
            column_index_of(&tasks, &filter, &status, &dragging(dragged), 0),
            1,
            "above the first card showing is above that card, not the top of the column"
        );
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging(dragged), 1),
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
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging("sing-1111"), 0),
            1
        );
        assert_eq!(
            column_index_of(&tasks, &filter, &status, &dragging("sing-1111"), 1),
            2,
            "and past it is the end of what the column has left"
        );
    }

    /// A drag made with several cards marked puts them down as one run, in the order the board
    /// had them - whichever of them the drop names first.
    #[test]
    fn cards_dragged_together_land_as_a_run_in_the_order_the_board_had_them() {
        let mut tasks = column(&["one", "two", "three", "four"]);
        let done = ColumnId::new("done");

        place_in(
            &mut tasks,
            &["three-1111".to_string(), "one-1111".to_string()],
            &done,
            0,
        );

        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["two", "four", "one", "three"]);
        assert_eq!(tasks[2].status, done);
        assert_eq!(tasks[3].status, done);
        assert_eq!(tasks[0].status, ColumnId::new("todo"));
    }

    #[test]
    fn cards_dragged_into_a_column_go_in_at_the_place_they_were_dropped() {
        let mut tasks = column(&["one", "two", "three", "four"]);

        place_in(
            &mut tasks,
            &["one-1111".to_string(), "two-1111".to_string()],
            &ColumnId::new("todo"),
            1,
        );

        let titles: Vec<&str> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["three", "one", "two", "four"]);
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
