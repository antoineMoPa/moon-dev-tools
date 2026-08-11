//! The moontasks board: the repo's `.moontasks` folder, drawn as columns of cards.
//!
//! The pane holds no state of its own. What it draws comes from the last answer the server
//! gave, and everything it does goes back through the backend, so the same board works
//! against a repo on this machine and one on another.

pub(crate) mod attach;
pub(crate) mod cards;
pub(crate) mod columns;

use cards::{
    CARD_SPACING, DraggedTask, card_drag_id, column_cards, draw_card, draw_empty_slot, place_in,
};

use egui::{Align, CornerRadius, Layout as UiLayout, RichText, ScrollArea, Ui, vec2};

use crate::{
    api::AgentKind,
    moontasks::{BoardColumn, ColumnId, CreateTaskRequest, StartResourceRequest},
    native::{
        app::App,
        model::{OpenedShell, PendingColumnPlace, PendingPlace, TaskDropped, TaskLanding},
        palette::CommandAction,
        panes::{OpenPaneRequest, PaneKind},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide one column of the board is. Cards are titles and a handful of small buttons, so
/// this is about what a title needs rather than what the window has.
const COLUMN_WIDTH: f32 = 286.0;

pub(super) use crate::native::widgets::CLOSE_MARK_SIZE;

/// What a click on the board asked for. Collected while drawing and acted on afterwards, so
/// nothing changes the pane tree or the task list while either is being read.
pub(super) enum BoardAction {
    /// Open the new-task box in this column.
    OpenComposer(ColumnId),
    CloseComposer,
    /// Create the typed task in this column — the one the composer is open in — and start
    /// the picked agent on it.
    Create(ColumnId, AgentKind),
    /// A card let go of in a column, at the place among its cards it was dropped.
    Place(String, ColumnId, usize),
    /// A column let go of on the board, at the place among the others it was dropped.
    PlaceColumn(ColumnId, usize),
    AddColumn(String),
    RenameColumn(ColumnId, String),
    CancelColumnRename,
    DeleteColumn(ColumnId),
    CloseColumnComposer,
    Delete(String),
    Rename(String, String),
    CancelRename,
    Start(String, StartResourceRequest),
    Resume(String, String),
    /// Open the modal that lists the agents' own sessions, for this task.
    OpenAttachPicker {
        task_id: String,
        task_title: String,
    },
    CloseAttachPicker,
    /// Put the picked session on the task and open a shell resumed on it.
    Attach {
        task_id: String,
        agent: AgentKind,
        agent_session_id: String,
    },
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

    attach::draw(app, ui.ctx(), &palette, &mut actions);

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
        // Dragging is how a column is moved, so it must not also mean "scroll the board".
        .scroll_source(egui::containers::scroll_area::ScrollSource {
            drag: egui::containers::scroll_area::DragScroll::Never,
            ..Default::default()
        })
        .show(ui, |ui| {
            ui.horizontal_top(|ui| draw_column_row(app, ui, height, palette, actions));
        });
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
    for column in &order {
        let rect = columns::with_column_drag(app, ui, column, origin, |app, ui| {
            draw_column(app, ui, column, height, palette, actions)
        });
        if Some(&column.id) != dragged.as_ref() {
            headings.push((column.id.clone(), rect.center().x));
        }
        ui.add_space(6.0);
    }

    // At the right-hand end, where a new column would go — and out of the way while one is
    // being dragged, so it is never the thing a column is dropped onto.
    if dragged.is_none() {
        ui.allocate_ui_with_layout(
            vec2(COLUMN_WIDTH, height),
            UiLayout::top_down(Align::Min),
            |ui| columns::draw_new_column(app, ui, palette, actions),
        );
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

/// The new-task box, which the `+` on a column's heading opens.
///
/// It is a card in the column it will add to, in the place the new card will appear, rather
/// than a row over the whole board: what is being written is a card.
fn draw_composer(
    app: &mut App,
    ui: &mut Ui,
    column: &ColumnId,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let available = available_agents(app);
    // What the box offers first: the choice already made in this box, else the agent the
    // column's last task was created with (the board's own file remembers it), else the
    // machine-wide one the review's selector holds. An agent that has since left this
    // machine would silently start nothing.
    let remembered = app
        .model
        .board
        .columns
        .iter()
        .find(|candidate| candidate.id == *column)
        .and_then(|candidate| candidate.default_agent);
    let mut agent = app
        .model
        .board
        .composer_agent
        .or(remembered)
        .unwrap_or_else(|| app.selected_agent());
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
                // The pick belongs to this box: the column it creates into remembers it once
                // the task is created, and the review's own selector is left alone.
                if picked != agent {
                    app.model.board.composer_agent = Some(picked);
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
                        actions.push(BoardAction::Create(column.clone(), agent));
                    }
                });
            });

            if submitted && ready {
                actions.push(BoardAction::Create(column.clone(), agent));
            }
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                actions.push(BoardAction::CloseComposer);
            }
        });
    ui.add_space(5.0);
}

/// How long a card takes to walk to a new place in its column. Long enough to be followed by
/// eye, short enough that the board is never waiting on it.
const CARD_SLIDE: f32 = 0.12;

/// Which way a run of things is laid out, and so which way one of them slides to a new place.
///
/// Cards stack down a column and columns run across the board, and both make room for one
/// being dragged in exactly the same way — so the animation is written once and told which
/// axis it is on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    /// Where the next thing will be laid out, along this axis.
    fn cursor_start(self, ui: &Ui) -> f32 {
        match self {
            Self::Vertical => ui.cursor().top(),
            Self::Horizontal => ui.cursor().left(),
        }
    }

    fn offset(self, along: f32) -> egui::Vec2 {
        match self {
            Self::Vertical => vec2(0.0, along),
            Self::Horizontal => vec2(along, 0.0),
        }
    }
}

// The board grew these marks first and the rest of the window took them up, so they live in
// [`widgets`] now and keep their old names here.
pub(super) use crate::native::widgets::{close_button, close_mark};

/// Whether a resource is still going: a filled dot for running, a hollow one for ended.
///
/// Drawn rather than typeset, because the bundled fonts have no circle glyph — the system
/// font that a shell's output borrows is not there to fall back on in a snapshot.
pub(super) fn running_dot(ui: &mut Ui, running: bool, palette: &Palette) {
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
pub(super) fn plus_button(ui: &mut Ui, palette: &Palette) -> egui::Response {
    const DIAMETER: f32 = 15.0;

    let (rect, response) = ui.allocate_exact_size(vec2(DIAMETER, DIAMETER), egui::Sense::click());
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
    column: &BoardColumn,
    height: f32,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) -> egui::Rect {
    let status = column.id.clone();
    let carried = egui::DragAndDrop::payload::<DraggedTask>(ui.ctx());
    let dragged_id = carried.as_deref().map(|carried| carried.0.clone());
    let tasks = column_cards(app, &status, dragged_id.as_deref());

    // A column stacks its cards, whatever layout the row of columns is in.
    ui.allocate_ui_with_layout(
        vec2(COLUMN_WIDTH, height),
        UiLayout::top_down(Align::Min),
        |ui| {
            ui.set_width(COLUMN_WIDTH);
            columns::draw_heading(app, ui, column, tasks.len(), palette, actions);
            ui.add_space(3.0);

            let composing = app.model.board.composer_in.as_ref() == Some(&column.id);

            // Where a drop would land, counted against the cards it would be put among — so the
            // slot the dragged card is standing in is taken back out of the reckoning, and moving
            // the pointer over a card cannot bounce between two answers.
            let mut cards: Vec<egui::Rect> = Vec::new();
            let mut slot = 0.0;
            let mut zone = egui::Frame::new()
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(4))
                .begin(ui);
            {
                let ui = &mut zone.content_ui;
                ScrollArea::vertical()
                    .id_salt(format!("moontasks-column-{status}"))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // What a card's place is measured against, so scrolling the column is not
                        // read as every card in it having moved.
                        let origin = ui.min_rect().top();
                        if composing {
                            draw_composer(app, ui, &status, palette, actions);
                        }
                        for task in &tasks {
                            let card = draw_card(app, ui, task, origin, palette, actions);
                            if Some(task.id.as_str()) == dragged_id.as_deref() {
                                draw_empty_slot(ui, card, palette);
                                slot = card.height() + CARD_SPACING;
                            } else {
                                cards.push(card.translate(vec2(0.0, -slot)));
                            }
                            ui.add_space(CARD_SPACING);
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

            // Which column the pointer is in, worked out from the pointer rather than taken from
            // the response: egui hit-tests a frame behind, on the widgets the last frame drew, and
            // a column that has just taken the dragged card in is not the column it hit-tested.
            let ghost = dragged_id
                .as_deref()
                .map(|task_id| egui::LayerId::new(egui::Order::Tooltip, card_drag_id(task_id)));
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

            if let Some(at) = landing {
                // Read by the next frame, which draws the card in this slot rather than in the
                // one it was picked up from.
                app.model.board.landing = Some(TaskLanding {
                    status: status.clone(),
                    index: at,
                });

                if ui.input(|input| input.pointer.any_released())
                    && let Some(dragged) = egui::DragAndDrop::take_payload::<DraggedTask>(ui.ctx())
                {
                    // The card is already drawn where it landed; this marks it there for a
                    // moment, because one let go of between two others is hard to pick back out.
                    app.model.board.dropped = Some(TaskDropped {
                        task_id: dragged.0.clone(),
                        at: ui.input(|input| input.time),
                    });
                    app.model.board.landing = None;
                    // The board is redrawn from the server's answer, which is a worker thread and
                    // a poll away: the move is made here as well, and held over every answer
                    // until one of them agrees, so the card stays where it was put rather than
                    // springing back and landing a second time.
                    app.model.board.pending_place = Some(PendingPlace {
                        task_id: dragged.0.clone(),
                        status: status.clone(),
                        index: at,
                    });
                    place_in(&mut app.model.board.tasks, &dragged.0, &status, at);
                    actions.push(BoardAction::Place(dragged.0.clone(), status.clone(), at));
                }
            }
        },
    )
    .response
    .rect
}

/// Whether the pointer is inside a column with nothing but the dragged card's own ghost over
/// it — that one follows the cursor, so it is over every column the cursor could be over and
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

/// Draw something at the place the layout gives it, moving there from wherever it was drawn
/// last rather than appearing there.
///
/// This is what makes the things a dragged one is being put between move out of its way
/// instead of jumping: the layout answers where each belongs, and this walks it there over
/// [`CARD_SLIDE`]. One that has not moved is drawn where it is with no work done, and one
/// drawn for the first time starts where it belongs rather than sliding in from the edge.
///
/// `origin` is what the place is measured from — the top of the column's contents for a card,
/// the left of the row for a column — so that scrolling, which moves everything at once, is
/// not read as everything having moved.
pub(super) fn slide_into_place(
    ui: &mut Ui,
    axis: Axis,
    id: egui::Id,
    origin: f32,
    draw: impl FnOnce(&mut Ui) -> egui::Rect,
) -> egui::Rect {
    let belongs_at = axis.cursor_start(ui) - origin;
    let drawn_at = ui
        .ctx()
        .animate_value_with_time(id.with("slide"), belongs_at, CARD_SLIDE);
    let offset = axis.offset(drawn_at - belongs_at);
    if offset.length() < 0.5 {
        return draw(ui);
    }

    // Drawn into a layer of its own so the shapes can be moved once they are made — the same
    // way the dragged card is. Its clip is moved the other way first, so one on its way between
    // two places is still cut off at the pane it is in rather than drawn over what is beside it.
    let layer_id = egui::LayerId::new(egui::Order::Middle, id.with("sliding"));
    let clip = ui.clip_rect();
    let rect = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
            ui.set_clip_rect(clip.translate(-offset));
            draw(ui)
        })
        .inner;
    ui.ctx()
        .transform_layer_shapes(layer_id, egui::emath::TSTransform::from_translation(offset));
    rect
}

/// Say that something is at the place the layout gives it right now, without walking there.
///
/// One that is somewhere for a reason of its own — carried by the cursor — is still at a
/// place, and the next thing to draw it has to know that place is where it already is.
pub(super) fn stamp_place(ui: &Ui, axis: Axis, id: egui::Id, origin: f32) {
    ui.ctx()
        .animate_value_with_time(id.with("slide"), axis.cursor_start(ui) - origin, 0.0);
}

/// The agents this machine has, with "None" first for a task started without one.
pub(super) fn available_agents(app: &App) -> Vec<AgentKind> {
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

fn apply(app: &mut App, action: BoardAction) {
    let session_id = app.model.root_session_id.clone();

    match action {
        BoardAction::OpenComposer(column_id) => {
            app.model.board.composer_in = Some(column_id);
            app.model.board.composer_focus = true;
            // Each column's box starts from that column's own remembered agent.
            app.model.board.composer_agent = None;
        }
        BoardAction::CloseComposer => {
            app.model.board.composer_in = None;
            app.model.board.new_title.clear();
            app.model.board.composer_agent = None;
        }
        BoardAction::Create(column_id, agent) => {
            let request = CreateTaskRequest {
                title: app.model.board.new_title.trim().to_string(),
                agent,
                status: column_id,
            };
            if request.title.is_empty() {
                return;
            }
            // The box closes on the way out: the card it was standing in for is on its way.
            app.model.board.new_title.clear();
            app.model.board.composer_in = None;
            app.model.board.composer_agent = None;
            act(app, "could not create the task", move |backend| {
                backend.create_task(&session_id, &request).map(|_| ())
            });
        }
        BoardAction::Place(task_id, status, position) => {
            app.tasks.spawn(
                move |backend| backend.place_task(&session_id, &task_id, status, position),
                |model, result| {
                    // A move the server would not make is not one to keep drawing.
                    if result.is_err() {
                        model.board.pending_place = None;
                    }
                    model.report(result, "could not move the task");
                    model.board.refresh_requested = true;
                },
            );
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
        BoardAction::OpenAttachPicker {
            task_id,
            task_title,
        } => {
            app.model.board.attach_picker = Some(crate::native::model::AttachPicker {
                task_id,
                task_title,
                sessions: None,
                error: None,
                manual_id: String::new(),
                manual_agent: None,
            });
            // Keyed, so holding the menu item down cannot queue a listing per frame. The
            // listing is the repo's rather than the task's, so whichever picker is open when
            // it lands is the one it answers — including one reopened on another card while
            // an earlier read was still on its way, whose own read the key swallowed.
            app.tasks.spawn_keyed(
                Some("agent-sessions".to_string()),
                move |backend| backend.list_agent_sessions(&session_id),
                move |model, result| {
                    let Some(picker) = model.board.attach_picker.as_mut() else {
                        return;
                    };
                    match result {
                        Ok(sessions) => picker.sessions = Some(sessions),
                        Err(error) => picker.error = Some(error.to_string()),
                    }
                },
            );
        }
        BoardAction::CloseAttachPicker => app.model.board.attach_picker = None,
        BoardAction::Attach {
            task_id,
            agent,
            agent_session_id,
        } => {
            app.model.board.attach_picker = None;
            let for_pane = task_id.clone();
            let request = crate::moontasks::AttachResourceRequest {
                agent,
                agent_session_id,
            };
            app.tasks.spawn(
                move |backend| backend.attach_task_resource(&session_id, &task_id, &request),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command: Some(agent),
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not attach it: {error}")),
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
        BoardAction::AddColumn(label) => {
            // The box closes on the way out: the column it was standing in for is on its way.
            app.model.board.new_column_label.clear();
            app.model.board.column_composer_open = false;
            act(app, "could not add the column", move |backend| {
                backend.add_column(&session_id, &label).map(|_| ())
            });
        }
        BoardAction::CloseColumnComposer => {
            app.model.board.column_composer_open = false;
            app.model.board.new_column_label.clear();
        }
        BoardAction::RenameColumn(column_id, label) => {
            app.model.board.renaming_column = None;
            act(app, "could not rename the column", move |backend| {
                backend.rename_column(&session_id, &column_id, &label)
            });
        }
        BoardAction::CancelColumnRename => app.model.board.renaming_column = None,
        BoardAction::DeleteColumn(column_id) => {
            app.model.board.pending_column_delete = None;
            act(app, "could not remove the column", move |backend| {
                backend.delete_column(&session_id, &column_id)
            });
        }
        BoardAction::PlaceColumn(column_id, position) => {
            app.tasks.spawn(
                move |backend| backend.place_column(&session_id, &column_id, position),
                |model, result| {
                    // A move the server would not make is not one to keep drawing.
                    if result.is_err() {
                        model.board.pending_column_place = None;
                    }
                    model.report(result, "could not move the column");
                    model.board.refresh_requested = true;
                },
            );
        }
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
