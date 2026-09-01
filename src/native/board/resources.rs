//! The runs and files a task has, as they are listed under it.
//!
//! The list is drawn twice - down a card, and down the task's own pane - and it is the same
//! list in both, from here: a row is a way back to what it names, and the marks that stop it,
//! resume it, or take it off the task.

use egui::{Align, CornerRadius, Layout as UiLayout, Rect, Response, RichText, Sense, Ui, UiBuilder, vec2};

use crate::{
    api::AgentKind,
    moontasks::{TaskResourceKind, TaskResourceView, TaskView},
    native::{
        app::App,
        board::{BoardAction, close_button, file_mark, running_dot},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// Every run and file the task has, each on its row.
pub(crate) fn draw_list(
    app: &App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    // Read once for the whole list: the mark that takes a run off the task asks first, and the
    // row that asked is the one drawn holding the question.
    let removing = app.model.board.pending_resource_delete.clone();
    for resource in &task.resources {
        let pending = removing.as_deref() == Some(resource.id.as_str());
        draw_resource(ui, task, resource, pending, palette, actions);
    }
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
    pending_delete: bool,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    if resource.kind == TaskResourceKind::File {
        draw_file_resource(ui, task, resource, palette, actions);
        return;
    }
    // A running shell is the way back to its tab; a run that has ended opens nothing, and
    // its row says so by staying unlit.
    let opens = resource.running && resource.terminal_id.is_some();
    let row = draw_row(ui, palette, resource, opens);
    draw_in_row(ui, row.rect, |ui| {
        running_dot(ui, resource.running, palette);

        match (&resource.terminal_id, resource.running) {
            (Some(terminal_id), true) => {
                if widgets::quiet_button(ui, &resource.label)
                    .on_hover_text("Open this shell in a tab")
                    .clicked()
                    || row.clicked()
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

/// How far a row's fill reaches past its contents on either side.
const ROW_INSET: f32 = 3.0;

/// The row a run or a file is listed on, taken before its contents are drawn: lit while the
/// pointer is over it, when a click on it would open something, so what the click will do is
/// plain before it is made. A row with nothing to open stays as it is. The row is a click of
/// its own, on everything the marks at its right do not cover - the marks are drawn after it,
/// so a click on one of them is the mark's and not the row's.
fn draw_row(ui: &mut Ui, palette: &Palette, resource: &TaskResourceView, opens: bool) -> Response {
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
            .on_hover_text(match resource.kind {
                TaskResourceKind::File => "Open this file in a pane",
                TaskResourceKind::Shell | TaskResourceKind::Agent => "Open this shell in a tab",
            })
    } else {
        row
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
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(file_path) = resource.file_path.as_deref() else {
        panic!("linked file {} has no file path", resource.id);
    };
    let row = draw_row(ui, palette, resource, true);
    draw_in_row(ui, row.rect, |ui| {
        file_mark(ui, palette);

        if widgets::quiet_button(ui, &widgets::elide_path(file_path, FILE_PATH_CHARS))
            .on_hover_text(format!("Open {file_path} in a pane"))
            .clicked()
            || row.clicked()
        {
            actions.push(BoardAction::OpenFile {
                task_id: task.id.clone(),
                file_path: file_path.to_string(),
            });
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            if close_button(ui, palette)
                .on_hover_text("Take this file off the task")
                .clicked()
            {
                actions.push(BoardAction::DeleteResource(
                    task.id.clone(),
                    resource.id.clone(),
                ));
            }
        });
    });
}
