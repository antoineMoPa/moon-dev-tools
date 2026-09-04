//! The runs and files a task has, as they are listed under it.
//!
//! The list is drawn twice - down a card, and down the task's own pane - and it is the same
//! list in both, from here: a row is a way back to what it names, and the marks that stop it,
//! resume it, or take it off the task.

use egui::{Align, CornerRadius, Layout as UiLayout, Rect, Response, RichText, Sense, Ui, UiBuilder, vec2};

use crate::{
    api::AgentKind,
    moontasks::{
        ReviewRequestView, TaskResourceKind, TaskResourceView, TaskView, review_request::Amend,
    },
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
        draw_review_request(ui, request, card, palette, actions);
    }
}

/// How far the line under a pending review is indented, so it starts under the words rather
/// than under the dot.
const BRANCH_LINE_INDENT: f32 = 12.0;
/// How much of a branch name that line shows. A branch made for a task can run to a slug and a
/// uuid, which would set the width of every card on the board; the front of it is what says
/// which branch it is, and the whole of it is on the row's hover. As much as fits a card at the
/// small size, which is most real branch names whole.
const BRANCH_NAME_CHARS: usize = 38;

/// One repo a task's `request_for_review.txt` asks to have looked at.
///
/// Whether it is still pending is not written down anywhere: it is the repo having changed files,
/// which the submodule hub's answer already says. So the list ticks itself off as the repos are
/// committed, and there is nothing on the row to press to say it is done - only the menu that
/// takes the line out of the file, for a review that turned out not to be wanted.
///
/// A branch goes on a line of its own under the name. Three things - what to review, which
/// branch, how much has changed - do not fit across a card, and a branch name is the longest and
/// the least often there, so it is the one that moves down. The row is drawn first and measured
/// after, so it is exactly as tall as what is in it and the card grows by the same amount.
fn draw_review_request(
    ui: &mut Ui,
    request: &ReviewRequestView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    // Three ways to be finished with: the repo has nothing left to commit, which the board can
    // see for itself; someone crossed the line off - work that is committed and pushed and wants
    // no more looking at, which it cannot; or the card was moved to the column that finishes a
    // task, which finishes everything it was still asking for at once.
    let pending = !request.done && !request.task_finished && request.changed_files > 0;

    // Kept back so the fill can be painted behind contents that have not been drawn yet: how
    // tall the row is is only known once they have been.
    let fill = ui.painter().add(egui::Shape::Noop);
    let mut opens = false;

    let drawn = ui.scope_builder(
        UiBuilder::new().layout(UiLayout::top_down(Align::Min)),
        |ui| {
            ui.horizontal(|ui| {
                ui.add_space(ROW_INSET);
                ui.set_min_height(ui.spacing().interact_size.y);
                running_dot(ui, pending, palette);

                let name = match pending {
                    true => widgets::quiet_button(ui, &format!("pending {} review", request.name)),
                    false => widgets::quiet_button_colored(
                        ui,
                        &format!("{} reviewed", request.name),
                        palette.muted,
                    ),
                };
                // What the agent wrote for the repo says what the review is about, which is more
                // than the row has room for.
                let name = match &request.suggestion {
                    Some(suggestion) => name.on_hover_text(&suggestion.subject),
                    None => name,
                };
                opens |= card.pressed(&name);

                ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                    ui.add_space(ROW_INSET);
                    ui.label(
                        RichText::new(changes_label(request.changed_files))
                            .size(SMALL_SIZE)
                            .color(palette.muted),
                    );
                });
            });

            if let Some(branch) = &request.branch {
                ui.horizontal(|ui| {
                    ui.add_space(ROW_INSET + BRANCH_LINE_INDENT);
                    ui.label(
                        RichText::new(format!(
                            "#{}",
                            widgets::elide_end(branch, BRANCH_NAME_CHARS)
                        ))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                    )
                    .on_hover_text(format!(
                        "#{branch}\nthe branch this commit belongs on - the review opens \
                         wherever it is checked out"
                    ));
                });
            }
        },
    );

    // The whole of what was drawn is one target, taken after it so it covers both lines. The
    // marks inside it were interacted with as they were drawn, so a click on one is theirs.
    let rect = drawn.response.rect;
    let row = ui.interact(
        rect,
        ui.make_persistent_id(("review-request", &request.task_id, request.index)),
        Sense::click(),
    );
    if row.hovered() && ui.is_rect_visible(rect) {
        ui.painter().set(
            fill,
            egui::epaint::RectShape::filled(rect, CornerRadius::same(3), palette.row_hover_bg),
        );
    }
    let row = row
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Open the review of this repo");
    opens |= card.pressed(&row);

    // Taking a line out of the file is the one thing done to a request, and it is not something
    // to press by accident on a row whose whole job is to be clicked - so it lives on the menu.
    egui::Popup::context_menu(&row)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut amend = |ui: &mut Ui, label: &str, hover: &str, amend: Amend| {
                if widgets::clickable(ui.button(label))
                    .on_hover_text(hover)
                    .clicked()
                {
                    actions.push(BoardAction::AmendReviewRequest {
                        task_id: request.task_id.clone(),
                        index: request.index,
                        amend,
                    });
                    ui.close();
                }
            };

            // Crossing off keeps the line, because it stays true that this repo was part of the
            // work; dismissing says the line should not have been written, so it goes.
            match request.done {
                false => amend(
                    ui,
                    "mark as completed",
                    "Cross this line off - the work is committed and wants no more looking at",
                    Amend::Done(true),
                ),
                true => amend(
                    ui,
                    "mark as pending",
                    "Put this line back on the list",
                    Amend::Done(false),
                ),
            }
            ui.separator();
            amend(
                ui,
                "dismiss",
                "Take this line out of the task's request_for_review.txt",
                Amend::Dismiss,
            );
        });

    if opens {
        actions.push(BoardAction::OpenReview(
            request.repo_path.clone(),
            request.name.clone(),
        ));
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
