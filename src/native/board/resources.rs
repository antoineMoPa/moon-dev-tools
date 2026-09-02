//! The runs and files a task has, as they are listed under it.
//!
//! The list is drawn twice - down a card, and down the task's own pane - and it is the same
//! list in both, from here: a row is a way back to what it names, and the marks that stop it,
//! resume it, or take it off the task.

use egui::{Align, CornerRadius, Layout as UiLayout, Rect, Response, RichText, Sense, Ui, UiBuilder, vec2};

use crate::{
    api::AgentKind,
    moontasks::{ReviewRequestView, TaskResourceKind, TaskResourceView, TaskView},
    native::{
        app::App,
        board::{BoardAction, close_button, file_mark, gesture::Controls, running_dot},
        submodules::changes_label,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// Every run and file the task has, each on its row.
pub(crate) fn draw_list(
    app: &App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    // Read once for the whole list: the mark that takes a run off the task asks first, and the
    // row that asked is the one drawn holding the question.
    let removing = app.model.board.pending_resource_delete.clone();
    for resource in &task.resources {
        let pending = removing.as_deref() == Some(resource.id.as_str());
        draw_resource(ui, task, resource, card, pending, palette, actions);
    }

    // Under the runs and the files, because a review is asked for once the rest has happened -
    // and in the order the task's file lists them, which is the order they are to be deployed.
    for request in app
        .model
        .review_requests
        .iter()
        .filter(|request| request.task_id == task.id)
    {
        draw_review_request(app, ui, request, card, palette, actions);
    }
}

/// One repo a task's `request_for_review.txt` asks to have looked at.
///
/// Whether it is still pending is not written down anywhere: it is the repo having changed files,
/// which the submodule hub's answer already says. So the list ticks itself off as the repos are
/// committed, and there is nothing on the row to press to say it is done.
fn draw_review_request(
    app: &App,
    ui: &mut Ui,
    request: &ReviewRequestView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let status = app.model.repo_status(&request.repo_path);
    // A repo the hub has said nothing about yet is taken to be pending: the row is what the
    // agent asked for, and it should not read as dealt with because a poll has not landed.
    let pending = status.is_none_or(|repo| repo.changed_files > 0);

    let row = draw_row(ui, palette, true, "Open the review of this repo");
    let row_pressed = card.pressed(&row);
    draw_in_row(ui, row.rect, |ui| {
        running_dot(ui, pending, palette);

        let name = match pending {
            true => widgets::quiet_button(ui, &format!("pending {} review", request.name)),
            false => widgets::quiet_button_colored(
                ui,
                &format!("{} reviewed", request.name),
                palette.muted,
            ),
        };
        // What the agent wrote for the repo says what the review is about, which is more than
        // the row has room for.
        let name = match &request.suggestion {
            Some(suggestion) => name.on_hover_text(&suggestion.subject),
            None => name,
        };
        if card.pressed(&name) || row_pressed {
            actions.push(BoardAction::OpenReview(
                request.repo_path.clone(),
                request.name.clone(),
            ));
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            if let Some(status) = status {
                ui.label(
                    RichText::new(changes_label(status))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
            }
            if let Some(branch) = &request.branch {
                ui.label(
                    RichText::new(format!("#{branch}"))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                )
                .on_hover_text("The branch the agent means this commit to be made on");
            }
        });
    });
}

/// How many characters of a linked file's path the card shows before the middle is cut out.
/// The end of a path is what tells files apart, so that is the part that is kept.
const FILE_PATH_CHARS: usize = 34;

/// One shell, agent run or linked file of a task: what it is, whether it is still going, and
/// the way back to it.
fn draw_resource(
    ui: &mut Ui,
    task: &TaskView,
    resource: &TaskResourceView,
    card: &mut Controls,
    pending_delete: bool,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    if resource.kind == TaskResourceKind::File {
        draw_file_resource(ui, task, resource, card, palette, actions);
        return;
    }
    // A running shell is the way back to its tab; a run that has ended opens nothing, and
    // its row says so by staying unlit.
    let opens = resource.running && resource.terminal_id.is_some();
    let row = draw_row(ui, palette, opens, hover_of(resource.kind));
    let row_pressed = card.pressed(&row);
    draw_in_row(ui, row.rect, |ui| {
        running_dot(ui, resource.running, palette);

        match (&resource.terminal_id, resource.running) {
            (Some(terminal_id), true) => {
                let name = widgets::quiet_button(ui, &resource.label)
                    .on_hover_text("Open this shell in a tab");
                if card.pressed(&name) || row_pressed
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
            // A shell has nothing to keep - closing it is the end of it - so it is offered the
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
            let close = close_button(ui, palette).on_hover_text(match (is_shell, resource.running) {
                (true, _) => "Close this shell",
                (false, true) => "End this run and take it off the task",
                (false, false) => "Take this run off the task",
            });
            if card.pressed(&close) {
                actions.push(BoardAction::ArmResourceDelete(resource.id.clone()));
            }
            if resource.running && !is_shell {
                let stop = widgets::quiet_button_colored(ui, "stop", palette.muted)
                    .on_hover_text("End this shell, keeping the run to come back to");
                if card.pressed(&stop) {
                    actions.push(BoardAction::Stop(task.id.clone(), resource.id.clone()));
                }
            } else if resource.resumable {
                let resume = widgets::quiet_button_colored(ui, "resume", palette.accent)
                    .on_hover_text("Start this agent again where it left off");
                if card.pressed(&resume) {
                    actions.push(BoardAction::Resume(task.id.clone(), resource.id.clone()));
                }
            }
        });
    });
}

/// How far a row's fill reaches past its contents on either side.
const ROW_INSET: f32 = 3.0;

/// The row a run, a file or a pending review is listed on, taken before its contents are drawn:
/// lit while the pointer is over it, when a click on it would open something, so what the click
/// will do is plain before it is made. A row with nothing to open stays as it is. The row is a
/// click of its own, on everything the marks at its right do not cover - the marks are drawn
/// after it, so a click on one of them is the mark's and not the row's.
fn draw_row(ui: &mut Ui, palette: &Palette, opens: bool, hover: &str) -> Response {
    let (rect, row) = ui.allocate_exact_size(
        vec2(ui.available_width(), ui.spacing().interact_size.y),
        if opens {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if opens && row.hovered() && ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), palette.row_hover_bg);
    }
    if opens {
        row.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hover)
    } else {
        row
    }
}

/// What a run's or a file's row says when the pointer rests on it.
fn hover_of(kind: TaskResourceKind) -> &'static str {
    match kind {
        TaskResourceKind::File => "Open this file in a pane",
        TaskResourceKind::Shell | TaskResourceKind::Agent => "Open this shell in a tab",
    }
}

/// Draw a row's contents inside the space [`draw_row`] took for it, inset from its fill.
fn draw_in_row(ui: &mut Ui, rect: Rect, contents: impl FnOnce(&mut Ui)) {
    let inside = rect.shrink2(vec2(ROW_INSET, 0.0));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(inside)
            .layout(UiLayout::left_to_right(Align::Center)),
        contents,
    );
}

/// A file linked to the task: its path, which opens it, and the mark that takes it off the
/// card again.
///
/// Nothing runs here, so there is nothing to stop or resume, and taking the file off the
/// card loses nothing - the file stays where it is - so unlike a run it goes without asking.
fn draw_file_resource(
    ui: &mut Ui,
    task: &TaskView,
    resource: &TaskResourceView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(file_path) = resource.file_path.as_deref() else {
        panic!("linked file {} has no file path", resource.id);
    };
    let row = draw_row(ui, palette, true, hover_of(resource.kind));
    let row_pressed = card.pressed(&row);
    draw_in_row(ui, row.rect, |ui| {
        file_mark(ui, palette);

        let path = widgets::quiet_button(ui, &widgets::elide_path(file_path, FILE_PATH_CHARS))
            .on_hover_text(format!("Open {file_path} in a pane"));
        if card.pressed(&path) || row_pressed {
            actions.push(BoardAction::OpenFile {
                task_id: task.id.clone(),
                file_path: file_path.to_string(),
            });
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            let unlink =
                close_button(ui, palette).on_hover_text("Take this file off the task");
            if card.pressed(&unlink) {
                actions.push(BoardAction::DeleteResource(
                    task.id.clone(),
                    resource.id.clone(),
                ));
            }
        });
    });
}
