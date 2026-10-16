//! The cards on the board: what one shows, and the drag that moves it between columns.
//!
//! A card is picked up by its title and drawn into a layer of its own while it is in flight,
//! so the buttons it carries stay clickable and the slot it is being held over shows through
//! it. Where it would land is worked out from the cards it would be put among rather than
//! from what the pointer is over, which is what keeps the answer from bouncing.

use egui::{Align, CornerRadius, Layout as UiLayout, Ui, vec2};

use crate::{
    moontasks::{BoardColumn, ColumnEnd, ColumnId, ColumnSort, TaskView, column_sort, sub_tasks},
    native::{
        app::App,
        board::{
            BoardAction, CLOSE_MARK_SIZE,
            actions::TaskPaneBox,
            close_button,
            filter::Filter,
            gesture,
            motion::{Axis, slide_into_place, stamp_place},
            resources, selection, start, tags, tracker_link,
        },
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
        // Under the card they are held over, already: that is the drop being shown.
        let carried = app
            .model
            .board
            .tasks
            .iter()
            .filter(|task| carrying.carries(&task.id))
            .map(|task| TaskView {
                parent: sub_tasks::parent_after_move(
                    task.parent.as_deref(),
                    landing.parent.as_deref(),
                    &carrying.task_ids,
                ),
                ..task.clone()
            });
        let at = landing.index.min(tasks.len());
        for (offset, task) in carried.enumerate() {
            tasks.insert(at + offset, task);
        }
    }
    // A sorted column keeps its order whatever is held over it: what is being carried is
    // shown where the order will put it rather than where the pointer is.
    if let Some(sort) = sort_of(&app.model.board.columns, status) {
        column_sort::sort_cards(sort, &mut tasks);
    }
    sub_tasks::under_their_parents(&mut tasks);
    tasks
}

/// The order a column keeps by itself, if it keeps one.
pub(super) fn sort_of(columns: &[BoardColumn], status: &ColumnId) -> Option<ColumnSort> {
    columns.iter().find(|column| column.id == *status)?.sort
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
/// once it is made. What is drawn in it is what has been typed on the pane so far, the way a
/// made card draws its title and notes, so the card fills in as the task is written.
pub(super) fn draw_pending_card(
    ui: &mut Ui,
    palette: &Palette,
    controls: &mut gesture::Controls,
    actions: &mut Vec<BoardAction>,
    column: &ColumnId,
    joins: ColumnEnd,
    (title, notes): &(String, String),
) {
    let slot = draw_written_slot(ui, palette, title, notes);

    // The empty card is the task as it stands, so clicking it brings the pane it is being
    // written on back in front - the same offer a made card's title makes. It is worth having
    // because the board and the writing are two different tabs: from a shell the card is what
    // is in front of you, and the half-named task it stands for is not.
    //
    // Registered before the cross so that the cross, drawn over it, keeps its own clicks.
    let body = ui
        .interact(slot, pending_card_id(column), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Back to naming this task");
    if controls.pressed(&body) {
        // The same thing the `+` that opened it does: one new-task pane at a time, and asking
        // for it again brings the open one forward with what is already written on it.
        actions.push(BoardAction::OpenNewTask(column.clone(), joins));
    }

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

/// The outlined slot of the card being written, with its title and the start of its notes in
/// it. It is [`PENDING_CARD_HEIGHT`] tall at the least and grows with what is written, cut the
/// way a made card cuts them. Answers with where it stands.
fn draw_written_slot(ui: &mut Ui, palette: &Palette, title: &str, notes: &str) -> egui::Rect {
    let width = ui.available_width();
    let inner_width = width
        - f32::from(CARD_MARGIN.left)
        - f32::from(CARD_MARGIN.right)
        - CLOSE_MARK_SIZE
        - ui.spacing().item_spacing.x;
    let card = egui::Frame::new()
        .corner_radius(CornerRadius::same(6))
        .fill(palette.control_bg)
        .stroke(egui::Stroke::new(1.0, palette.accent))
        .inner_margin(CARD_MARGIN)
        .show(ui, |ui| {
            ui.set_width(width - f32::from(CARD_MARGIN.left) - f32::from(CARD_MARGIN.right));
            ui.set_min_height(
                PENDING_CARD_HEIGHT - f32::from(CARD_MARGIN.top) - f32::from(CARD_MARGIN.bottom),
            );
            if !title.trim().is_empty() {
                let title = widgets::cut_to_fit(
                    ui,
                    title.trim(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    palette.ink,
                    inner_width,
                    TITLE_ROWS,
                );
                ui.add(egui::Label::new(title).selectable(false));
            }
            if !notes.trim().is_empty() {
                let preview = widgets::cut_to_fit(
                    ui,
                    notes.trim(),
                    egui::FontId::proportional(SMALL_SIZE),
                    palette.muted,
                    ui.available_width(),
                    NOTES_ROWS,
                );
                ui.add(egui::Label::new(preview).selectable(false));
            }
        });
    card.response.rect
}

/// The id the empty card is interacted by - named the way a card's and a column's handles are,
/// so it can be read back by whatever needs to know where it was drawn.
pub(crate) fn pending_card_id(column: &ColumnId) -> egui::Id {
    egui::Id::new(("moontask-pending-card", column.as_str()))
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
            // The task's issue in the tracker it is kept in, under what the task is about.
            tracker_link::draw_on_card(ui, task, &mut card, palette);

            resources::draw_list(
                app,
                ui,
                task.runs(),
                &mut card,
                palette,
                actions,
                resources::Rows::Down,
            );
            if !task.resources.is_empty() {
                ui.add_space(3.0);
            }
            // The documents the task lists, under what runs in it.
            super::attachments::draw_list(app, ui, task, &mut card, palette, actions, false);
            if !task.attachments.is_empty() {
                ui.add_space(3.0);
            }
            // At the foot of the card, over the row that opens their box: what a card is
            // marked with is read after what it is about.
            tags::draw_on_card(app, ui, task, palette, &mut card, actions);

            draw_card_actions(app, ui, task, &mut card, drag_id, showing, actions);
        })
        .response
        .rect;
    tags::shut_on_press_elsewhere(app, ui, task, rect);

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
            // The folder is kept under `.moontasks/.deleted`, but the task's shells and agents
            // are stopped for good, so the cross asks first - the same two-press shape
            // discarding a hunk has.
            if pending_delete {
                match widgets::confirm(
                    ui,
                    palette,
                    "[really delete]",
                    "this stops the task's shells and agents, and moves its folder to .moontasks/.deleted",
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
/// itself. All it senses is the pointer being over it, which turns the pointer into a hand
/// and shows nothing else: a title can run to several lines, and showing the whole of it
/// covered the board each time the pointer crossed a card.
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
        .on_hover_cursor(egui::CursorIcon::PointingHand);
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
/// [`start::draw_button`], the same ones the start window shows - and beside it the `[tags]`
/// that opens the box the card is marked in.
///
/// They come up with the notes offer above them and go the same way, so a card at rest is its
/// title and its description and nothing else. They sit at the bottom right, out of the way of
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
        let start_up = start::draw_button(app, ui, task.runs(), card, actions);
        let tags_up = tags::draw_button(app, ui, task, card);
        // Told to the card, which keeps its offers out for as long as the menu or the tag box
        // is up.
        ui.data_mut(|data| data.insert_temp(menu_up_id(drag_id), start_up || tags_up));
    });
}
