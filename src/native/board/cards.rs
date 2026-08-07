//! The cards on the board: what one shows, and the drag that moves it between columns.
//!
//! A card is picked up by its title and drawn into a layer of its own while it is in flight,
//! so the buttons it carries stay clickable and the slot it is being held over shows through
//! it. Where it would land is worked out from the cards it would be put among rather than
//! from what the pointer is over, which is what keeps the answer from bouncing.

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, Ui, vec2};

use crate::{
    api::AgentKind,
    moontasks::{ColumnId, StartResourceRequest, TaskResourceKind, TaskResourceView, TaskView},
    native::{
        app::App,
        board::{
            Axis, BoardAction, CLOSE_MARK_SIZE, agent_label, available_agents, close_button,
            running_dot, slide_into_place, stamp_place,
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

/// How many lines of a card's title are shown before the rest is cut. Enough for a sentence
/// of a task name, short enough that one long title does not push every card down the column.
const TITLE_ROWS: usize = 3;

/// The cards of one column, in the order they are drawn.
///
/// A card being dragged is one of them from the moment it is over the column, and no longer
/// one of the column it came from: the board makes the move as it is being made rather than
/// once it is over, so nothing jumps when the card is let go of.
pub(super) fn column_cards(app: &App, status: &ColumnId, dragged_id: Option<&str>) -> Vec<TaskView> {
    let landing = app.model.board.landing.clone();
    // Until the pointer has been over a column there is nowhere for the card to be but where
    // it came from, and taking it out of the board for that first frame reads as a flicker.
    let taken = landing.is_some();

    let mut tasks: Vec<TaskView> = app
        .model
        .board
        .tasks
        .iter()
        .filter(|task| {
            task.status == *status && !(taken && Some(task.id.as_str()) == dragged_id)
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

/// The id a card is dragged by, which is also the layer its ghost is drawn into.
pub(super) fn card_drag_id(task_id: &str) -> egui::Id {
    egui::Id::new(("moontask-card", task_id))
}

/// Take a board the server has answered with, with a drop that it may not have seen yet.
///
/// A read that was already on its way when a card was dropped answers with the card where it
/// was, so the drop is made again on top of it — until an answer comes back with the card
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
            let (task_id, status, index) =
                (pending.task_id.clone(), pending.status.clone(), pending.index);
            place_in(&mut tasks, &task_id, &status, index);
        }
    }
    model.board.tasks = tasks;
}

/// Make the move on the board being drawn, ahead of the server being told about it.
///
/// What the board draws is the last answer the server gave, and the next one is a worker
/// thread and a poll away. Without this the dropped card springs back to where it came from
/// for those few frames and then moves again — which reads as the drop having failed.
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
/// leaves the buttons underneath clickable — anything sensing a drag claims everything under
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
    // remembers where cards are is concerned — the drawing is at the cursor, but the place is
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

/// A card's border, which is the ordinary one except for a moment after the card was dropped.
///
/// A card let go of among a column of others is easy to lose track of, so the one that just
/// landed is marked and fades back over [`DROP_FLASH`]. It is a fade rather than a mark that
/// is cleared: nothing has to remember to put it back.
fn dropped_stroke(app: &App, ui: &Ui, task: &TaskView, palette: &Palette) -> egui::Stroke {
    let plain = egui::Stroke::new(1.0, palette.line);
    let Some(dropped) = &app.model.board.dropped else {
        return plain;
    };
    if dropped.task_id != task.id {
        return plain;
    }
    let left = (DROP_FLASH - (ui.input(|input| input.time) - dropped.at) as f32) / DROP_FLASH;
    if left <= 0.0 {
        return plain;
    }
    // The fade is drawn frame by frame, so it needs frames to be drawn in.
    ui.ctx().request_repaint();
    egui::Stroke::new(
        1.0 + left,
        palette.line.lerp_to_gamma(palette.warn, left),
    )
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
        .stroke(dropped_stroke(app, ui, task, palette))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            draw_card_title(app, ui, task, drag_id, palette, actions);
            ui.add_space(3.0);

            let removing = app.model.board.pending_resource_delete.clone();
            for resource in &task.resources {
                let pending = removing.as_deref() == Some(resource.id.as_str());
                draw_resource(ui, task, resource, pending, palette, actions);
            }
            if !task.resources.is_empty() {
                ui.add_space(3.0);
            }

            draw_card_actions(app, ui, task, actions);
        })
        .response
        .rect
}

/// The card's title — the handle it is dragged by, the box it is renamed in, and the mark
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
    let editing = app.model.board.renaming.as_ref().is_some_and(|rename| rename.task_id == task.id);
    let handle_width = ui.available_width() - CLOSE_MARK_SIZE - ui.spacing().item_spacing.x;

    ui.horizontal(|ui| {
        if editing {
            draw_title_editor(app, ui, task, handle_width, actions);
        } else {
            draw_title_handle(app, ui, task, drag_id, handle_width, palette, actions);
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            // The folder and everything an agent left in it goes, so the cross asks first —
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

/// The title as it usually reads: what the card is dragged by, and what a double click opens
/// for renaming.
fn draw_title_handle(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    handle_width: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let _ = actions;
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
        .on_hover_text(format!("{}\n\n{}", task.title, task.dir_path));

    if handle.double_clicked() {
        app.model.board.renaming = Some(crate::native::model::TaskRename {
            task_id: task.id.clone(),
            title: task.title.clone(),
            focus: true,
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

    let title = rename.title.clone();
    let abandon = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let keep = entry.lost_focus() && !abandon;

    if keep && !title.trim().is_empty() && title != task.title {
        actions.push(BoardAction::Rename(task.id.clone(), title));
    } else if abandon || keep {
        actions.push(BoardAction::CancelRename);
    }
}

/// One shell or agent run of a task: what it is, whether it is still going, and the way back
/// to it.
fn draw_resource(
    ui: &mut Ui,
    task: &TaskView,
    resource: &TaskResourceView,
    pending_delete: bool,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    ui.horizontal(|ui| {
        running_dot(ui, resource.running, palette);

        match (&resource.terminal_id, resource.running) {
            (Some(terminal_id), true) => {
                if widgets::quiet_button(ui, &resource.label)
                    .on_hover_text("Open this shell in a tab")
                    .clicked()
                {
                    actions.push(BoardAction::OpenShell {
                        terminal_id: terminal_id.clone(),
                        command: (resource.agent != AgentKind::None).then_some(resource.agent),
                        task_id: task.id.clone(),
                    });
                }
            }
            _ => {
                ui.label(
                    RichText::new(&resource.label)
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
            }
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            // Furthest right, so the two that keep the run are never the one you mean to
            // press and miss. Removing a run is not undoable either, so it asks first.
            // A shell has nothing to keep — closing it is the end of it — so it is offered the
            // close mark alone, while an agent run can be stopped and come back to.
            let is_shell = resource.kind == TaskResourceKind::Shell;

            if pending_delete {
                match widgets::confirm(
                    ui,
                    palette,
                    "[really close]",
                    if is_shell {
                        "this ends the shell, and its scrollback goes with it"
                    } else {
                        "this ends the run and takes it off the task for good"
                    },
                ) {
                    widgets::Confirmed::Yes => actions.push(BoardAction::DeleteResource(
                        task.id.clone(),
                        resource.id.clone(),
                    )),
                    widgets::Confirmed::No => actions.push(BoardAction::CancelResourceDelete),
                    widgets::Confirmed::Waiting => {}
                }
                return;
            }
            if close_button(ui, palette)
                .on_hover_text(match (is_shell, resource.running) {
                    (true, _) => "Close this shell",
                    (false, true) => "End this run and take it off the task",
                    (false, false) => "Take this run off the task",
                })
                .clicked()
            {
                actions.push(BoardAction::ArmResourceDelete(resource.id.clone()));
            }
            if resource.running && !is_shell {
                if widgets::quiet_button_colored(ui, "stop", palette.muted)
                    .on_hover_text("End this shell, keeping the run to come back to")
                    .clicked()
                {
                    actions.push(BoardAction::Stop(task.id.clone(), resource.id.clone()));
                }
            } else if resource.resumable
                && widgets::quiet_button_colored(ui, "resume", palette.accent)
                    .on_hover_text("Start this agent again where it left off")
                    .clicked()
            {
                actions.push(BoardAction::Resume(task.id.clone(), resource.id.clone()));
            }
        });
    });
}

fn draw_card_actions(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    actions: &mut Vec<BoardAction>,
) {
    let agents: Vec<AgentKind> = available_agents(app)
        .into_iter()
        .filter(|agent| *agent != AgentKind::None)
        .collect();

    ui.horizontal(|ui| {
        if widgets::quiet_button(ui, "[start review]")
            .on_hover_text("Open the review of this repo in a tab")
            .clicked()
        {
            actions.push(BoardAction::OpenReview(
                task.repo_path.clone(),
                task.title.clone(),
            ));
        }

        if widgets::quiet_button(ui, "[launch shell]")
            .on_hover_text("Open a shell in this task")
            .clicked()
        {
            actions.push(BoardAction::Start(
                task.id.clone(),
                StartResourceRequest {
                    kind: TaskResourceKind::Shell,
                    agent: AgentKind::None,
                },
            ));
        }

        // The same bracketed action as the other two, opening onto which agent to run: the
        // menu is built from the button rather than the other way round, so it can be one.
        if !agents.is_empty() {
            egui::containers::menu::MenuButton::from_button(
                egui::Button::new("[new agent]").frame(false),
            )
            .ui(ui, |ui| {
                for agent in agents {
                    if widgets::clickable(ui.button(agent_label(agent))).clicked() {
                        actions.push(BoardAction::Start(
                            task.id.clone(),
                            StartResourceRequest {
                                kind: TaskResourceKind::Agent,
                                agent,
                            },
                        ));
                        ui.close();
                    }
                }
            });
        }
    });
}
