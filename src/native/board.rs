//! The moontasks board: the repo's `.moontasks` folder, drawn as columns of cards.
//!
//! The pane holds no state of its own. What it draws comes from the last answer the server
//! gave, and everything it does goes back through the backend, so the same board works
//! against a repo on this machine and one on another.

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, ScrollArea, Ui, vec2};

use crate::{
    api::AgentKind,
    moontasks::{
        CreateTaskRequest, StartResourceRequest, TaskResourceKind, TaskResourceView, TaskStatus,
        TaskView, store::BOARD_COLUMNS,
    },
    native::{
        app::App,
        model::OpenedShell,
        palette::CommandAction,
        panes::{OpenPaneRequest, PaneKind},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide one column of the board is. Cards are titles and a handful of small buttons, so
/// this is about what a title needs rather than what the window has.
const COLUMN_WIDTH: f32 = 286.0;

/// The task a drag is carrying. A type of its own rather than the bare id, so a column only
/// lights up for a card and never for whatever else the window might one day let go of.
#[derive(Clone)]
struct DraggedTask(String);

/// The close mark's box, matching the one on a tab.
const CLOSE_MARK_SIZE: f32 = 12.0;

/// How many lines of a card's title are shown before the rest is cut. Enough for a sentence
/// of a task name, short enough that one long title does not push every card down the column.
const TITLE_ROWS: usize = 3;

/// What a click on the board asked for. Collected while drawing and acted on afterwards, so
/// nothing changes the pane tree or the task list while either is being read.
enum BoardAction {
    OpenComposer,
    CloseComposer,
    Create,
    /// Point the review — and so the next task — at another agent.
    SelectAgent(AgentKind),
    SetStatus(String, TaskStatus),
    Delete(String),
    Rename(String, String),
    CancelRename,
    Start(String, StartResourceRequest),
    Resume(String, String),
    Stop(String, String),
    /// Take a run off the task for good, rather than leaving it to be resumed.
    DeleteResource(String, String),
    ArmResourceDelete(String),
    CancelResourceDelete,
    /// Bring a task's shell on screen, in a tab of its own.
    OpenShell {
        terminal_id: String,
        command: Option<AgentKind>,
        task_id: String,
    },
    /// Open the review of what the task has changed.
    OpenReview(String, String),
}

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

    for action in actions {
        apply(app, action);
    }
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

    // The columns are as tall as the pane, and the board scrolls sideways to reach the ones
    // that do not fit — measured before the scroll area, which has no height of its own.
    let height = ui.available_height();
    ScrollArea::horizontal()
        .id_salt("moontasks-columns")
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for status in BOARD_COLUMNS {
                    draw_column(app, ui, *status, height, palette, actions);
                }
            });
        });
}

/// The new-task box, which the `+` on the TODO column opens.
///
/// It is a card in the column it will add to, in the place the new card will appear, rather
/// than a row over the whole board: what is being written is a card.
fn draw_composer(
    app: &mut App,
    ui: &mut Ui,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let available = available_agents(app);
    // The agent the review's selector is on, which is the one this machine last used. An
    // agent that has since left it would silently start nothing.
    let mut agent = app.selected_agent();
    if !available.contains(&agent) {
        agent = AgentKind::None;
    }

    egui::Frame::new()
        .fill(palette.composer_bg)
        .stroke(egui::Stroke::new(1.0, palette.accent))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            let entry = ui.add(
                egui::TextEdit::singleline(&mut app.model.board.new_title)
                    .hint_text("Task title")
                    .desired_width(f32::INFINITY),
            );
            // The box opened because the user asked to type in it, so it takes the keyboard.
            if std::mem::take(&mut app.model.board.composer_focus) {
                entry.request_focus();
            }
            let submitted =
                entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            ui.add_space(4.0);

            let ready = !app.model.board.new_title.trim().is_empty();
            ui.horizontal(|ui| {
                let mut picked = agent;
                egui::ComboBox::from_id_salt("moontasks-new-agent")
                    .selected_text(agent_label(agent))
                    .width(104.0)
                    .show_ui(ui, |ui| {
                        for option in &available {
                            ui.selectable_value(&mut picked, *option, agent_label(*option));
                        }
                    });
                // Picking here is picking for the review too — one choice, remembered in one
                // place, rather than a second agent living beside it.
                if picked != agent {
                    actions.push(BoardAction::SelectAgent(picked));
                }

                ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                    if close_button(ui, palette)
                        .on_hover_text("Discard this task")
                        .clicked()
                    {
                        actions.push(BoardAction::CloseComposer);
                    }
                    if widgets::clickable(ui.add_enabled(ready, egui::Button::new("create")))
                        .on_hover_text("Create the task and start the agent on it")
                        .clicked()
                    {
                        actions.push(BoardAction::Create);
                    }
                });
            });

            if submitted && ready {
                actions.push(BoardAction::Create);
            }
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                actions.push(BoardAction::CloseComposer);
            }
        });
    ui.add_space(5.0);
}

/// The thin `✕` a tab carries to close it, as a button of its own.
///
/// Drawn rather than typeset for the reason `egui_frames` gives: a font's close glyph is a
/// heavy emoji ✖, and this wants the hairline cross a browser tab draws.
fn close_button(ui: &mut Ui, palette: &Palette) -> egui::Response {
    const SIZE: f32 = CLOSE_MARK_SIZE;

    let (rect, response) = ui.allocate_exact_size(vec2(SIZE, SIZE), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let ink = if response.hovered() {
            palette.warn
        } else {
            palette.muted
        };
        let reach = SIZE * 0.27;
        let stroke = egui::Stroke::new(1.0, ink);
        let center = rect.center();
        ui.painter().line_segment(
            [center + vec2(-reach, -reach), center + vec2(reach, reach)],
            stroke,
        );
        ui.painter().line_segment(
            [center + vec2(reach, -reach), center + vec2(-reach, reach)],
            stroke,
        );
    }
    widgets::clickable(response)
}

/// Whether a resource is still going: a filled dot for running, a hollow one for ended.
///
/// Drawn rather than typeset, because the bundled fonts have no circle glyph — the system
/// font that a shell's output borrows is not there to fall back on in a snapshot.
fn running_dot(ui: &mut Ui, running: bool, palette: &Palette) {
    const DIAMETER: f32 = 7.0;

    let (rect, _) = ui.allocate_exact_size(vec2(DIAMETER, DIAMETER), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let center = rect.center();
    if running {
        ui.painter()
            .circle_filled(center, DIAMETER / 2.0, palette.added);
    } else {
        ui.painter().circle_stroke(
            center,
            DIAMETER / 2.0 - 0.5,
            egui::Stroke::new(1.0, palette.muted),
        );
    }
}

/// A `+` on a filled disc, the same button the tab strips carry for a new tab.
///
/// It is drawn rather than taken from `egui_frames`, which only offers it as part of a tab
/// strip — but it is the same shape, because it means the same thing.
fn plus_button(ui: &mut Ui, palette: &Palette) -> egui::Response {
    const DIAMETER: f32 = 15.0;

    let (rect, response) =
        ui.allocate_exact_size(vec2(DIAMETER, DIAMETER), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, ink) = if response.hovered() {
            (palette.control_active_bg, palette.ink)
        } else {
            (palette.control_bg, palette.muted)
        };
        ui.painter()
            .circle_filled(rect.center(), DIAMETER / 2.0, fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(DIAMETER * 0.72),
            ink,
        );
    }
    widgets::clickable(response)
}

fn draw_column(
    app: &mut App,
    ui: &mut Ui,
    status: TaskStatus,
    height: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let tasks: Vec<TaskView> = app
        .model
        .board
        .tasks
        .iter()
        .filter(|task| task.status == status)
        .cloned()
        .collect();

    // A column stacks its cards, whatever layout the row of columns is in.
    ui.allocate_ui_with_layout(
        vec2(COLUMN_WIDTH, height),
        UiLayout::top_down(Align::Min),
        |ui| {
        ui.set_width(COLUMN_WIDTH);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(status.label())
                    .size(SMALL_SIZE - 1.0)
                    .color(palette.muted)
                    .strong(),
            );
            ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                // New tasks start in TODO, so that is the column that offers one.
                if status == TaskStatus::Todo
                    && plus_button(ui, palette)
                        .on_hover_text("New task")
                        .clicked()
                {
                    actions.push(BoardAction::OpenComposer);
                }
                ui.label(
                    RichText::new(tasks.len().to_string())
                        .size(SMALL_SIZE - 1.0)
                        .color(palette.muted),
                );
            });
        });
        ui.add_space(3.0);

        let composing = status == TaskStatus::Todo && app.model.board.composer_open;

        // The whole column below its heading is where a card can be dropped, so the target is
        // the column the eye reads rather than the gap between two cards.
        let carrying = egui::DragAndDrop::has_payload_of_type::<DraggedTask>(ui.ctx());
        let mut zone = egui::Frame::new()
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(4))
            .begin(ui);
        {
            let ui = &mut zone.content_ui;
            ScrollArea::vertical()
                .id_salt(format!("moontasks-column-{}", status.wire_name()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if composing {
                        draw_composer(app, ui, palette, actions);
                    }
                    for task in &tasks {
                        draw_card(app, ui, task, palette, actions);
                        ui.add_space(5.0);
                    }
                    if tasks.is_empty() && !composing {
                        ui.label(
                            RichText::new("nothing here")
                                .size(SMALL_SIZE)
                                .color(palette.muted),
                        );
                    }
                });
        }
        let response = zone.allocate_space(ui);

        // `contains_pointer` rather than `hovered`, which is false while something else is
        // being dragged — and something else being dragged is the whole situation here.
        let over = carrying && response.contains_pointer();
        if over {
            zone.frame.fill = palette.control_active_bg;
            zone.frame.stroke = egui::Stroke::new(1.0, palette.accent);
        }
        zone.paint(ui);

        if over && ui.input(|input| input.pointer.any_released())
            && let Some(dragged) = egui::DragAndDrop::take_payload::<DraggedTask>(ui.ctx())
        {
            actions.push(BoardAction::SetStatus(dragged.0.clone(), status));
        }
        },
    );
    ui.add_space(6.0);
}

/// One card, and the drag that carries it between columns.
///
/// The card is picked up by its title, but what moves is the whole box: while a drag is in
/// flight the card is drawn into a layer of its own and that layer is moved to the cursor,
/// which is how `egui`'s own drag sources work. Sensing the drag on the title alone is what
/// leaves the buttons underneath clickable — anything sensing a drag claims everything under
/// it.
fn draw_card(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let drag_id = egui::Id::new(("moontask-card", &task.id));
    if !ui.ctx().is_being_dragged(drag_id) {
        draw_card_body(app, ui, task, drag_id, palette, actions);
        return;
    }

    egui::DragAndDrop::set_payload(ui.ctx(), DraggedTask(task.id.clone()));

    let layer_id = egui::LayerId::new(egui::Order::Tooltip, drag_id);
    let card = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
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
}

fn draw_card_body(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    drag_id: egui::Id,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    egui::Frame::new()
        .fill(palette.panel)
        .stroke(egui::Stroke::new(1.0, palette.line))
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
        });
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
            if pending_delete {
                match widgets::confirm(
                    ui,
                    palette,
                    "[really remove]",
                    "this ends the run and takes it off the task for good",
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
                .on_hover_text(if resource.running {
                    "End this run and take it off the task"
                } else {
                    "Take this run off the task"
                })
                .clicked()
            {
                actions.push(BoardAction::ArmResourceDelete(resource.id.clone()));
            }
            if resource.running {
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

/// The agents this machine has, with "None" first for a task started without one.
fn available_agents(app: &App) -> Vec<AgentKind> {
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
                .filter(|option| option.available)
                .map(|option| option.kind),
        );
    }
    agents
}

fn agent_label(agent: AgentKind) -> String {
    match agent {
        AgentKind::None => "no agent".to_string(),
        other => other.label().to_lowercase(),
    }
}

fn apply(app: &mut App, action: BoardAction) {
    let session_id = app.model.root_session_id.clone();

    match action {
        BoardAction::OpenComposer => {
            app.model.board.composer_open = true;
            app.model.board.composer_focus = true;
        }
        BoardAction::CloseComposer => {
            app.model.board.composer_open = false;
            app.model.board.new_title.clear();
        }
        BoardAction::Create => {
            let request = CreateTaskRequest {
                title: app.model.board.new_title.trim().to_string(),
                agent: app.selected_agent(),
            };
            if request.title.is_empty() {
                return;
            }
            // The box closes on the way out: the card it was standing in for is on its way.
            app.model.board.new_title.clear();
            app.model.board.composer_open = false;
            act(app, "could not create the task", move |backend| {
                backend.create_task(&session_id, &request).map(|_| ())
            });
        }
        BoardAction::SelectAgent(agent) => {
            let root = session_id.clone();
            app.tasks
                .act(&session_id, "could not switch agent", move |backend| {
                    backend.set_agent(&root, agent)
                });
        }
        BoardAction::SetStatus(task_id, status) => {
            act(app, "could not move the task", move |backend| {
                backend.set_task_status(&session_id, &task_id, status)
            });
        }
        BoardAction::Delete(task_id) => {
            app.model.board.pending_delete = None;
            act(app, "could not delete the task", move |backend| {
                backend.delete_task(&session_id, &task_id)
            });
        }
        BoardAction::Start(task_id, request) => {
            // The shell it starts in is what the user wants to look at, so it opens with it.
            let for_pane = task_id.clone();
            let command = (request.agent != AgentKind::None).then_some(request.agent);
            app.tasks.spawn(
                move |backend| backend.start_task_resource(&session_id, &task_id, request),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command,
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not start it: {error}")),
                    }
                },
            );
        }
        BoardAction::Resume(task_id, resource_id) => {
            let for_pane = task_id.clone();
            let command = app
                .model
                .board
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .and_then(|task| {
                    task.resources
                        .iter()
                        .find(|resource| resource.id == resource_id)
                })
                .map(|resource| resource.agent);
            app.tasks.spawn(
                move |backend| backend.resume_task_resource(&session_id, &task_id, &resource_id),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command,
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not resume it: {error}")),
                    }
                },
            );
        }
        BoardAction::Stop(task_id, resource_id) => {
            act(app, "could not stop it", move |backend| {
                backend.stop_task_resource(&session_id, &task_id, &resource_id)
            });
        }
        BoardAction::ArmResourceDelete(resource_id) => {
            app.model.board.pending_resource_delete = Some(resource_id);
        }
        BoardAction::CancelResourceDelete => app.model.board.pending_resource_delete = None,
        BoardAction::DeleteResource(task_id, resource_id) => {
            app.model.board.pending_resource_delete = None;
            act(app, "could not remove it", move |backend| {
                backend.delete_task_resource(&session_id, &task_id, &resource_id)
            });
        }
        BoardAction::Rename(task_id, title) => {
            app.model.board.renaming = None;
            act(app, "could not rename the task", move |backend| {
                backend.rename_task(&session_id, &task_id, &title)
            });
        }
        BoardAction::CancelRename => app.model.board.renaming = None,
        BoardAction::OpenShell {
            terminal_id,
            command,
            task_id,
        } => {
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::AttachTerminal {
                terminal_id,
                command,
                task_id: Some(task_id),
            }));
        }
        BoardAction::OpenReview(repo_path, title) => {
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::ReviewRepo {
                repo_path,
                title,
            }));
        }
    }
}

/// Run a board action, and read the board again once it is done.
fn act<W>(app: &App, context: &'static str, work: W)
where
    W: FnOnce(&dyn crate::backend::Backend) -> anyhow::Result<()> + Send + 'static,
{
    app.tasks.spawn(work, move |model, result| {
        model.report(result, context);
        model.board.refresh_requested = true;
    });
}

/// Whether the board is open, which is what decides if it is worth polling.
pub(crate) fn is_open(app: &App) -> bool {
    app.model
        .layout
        .find_pane(|pane| pane.kind() == PaneKind::Tasks)
        .is_some()
}
