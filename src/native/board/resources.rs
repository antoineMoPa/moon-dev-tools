//! The runs and files a task has, as they are listed under it.
//!
//! The list is drawn twice - down a card, and down the task's own pane - and it is the same
//! list in both, from here: a row is a way back to what it names, and the marks that stop it,
//! resume it, or take it off the task.

use egui::{
    Align, CornerRadius, Layout as UiLayout, Rect, Response, RichText, Sense, Ui, UiBuilder, vec2,
};

use crate::{
    api::AgentKind,
    commit_suggestion::CommitSuggestion,
    moontasks::{
        ReviewRequestView, RunsOf, TaskResourceKind, TaskResourceView, review_request::Amend,
    },
    native::{
        app::App,
        board::{
            BoardAction, close_button,
            gesture::Controls,
            marks::{Activity, activity_dot, chart_mark, file_mark, running_dot},
        },
        submodules::changes_label,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How a task's rows are laid out.
#[derive(Clone, Copy)]
pub(crate) enum Rows {
    /// One under the other, each as wide as what they are listed in: a card, or a task's pane.
    Down,
    /// Side by side, each this wide, in a `Ui` that wraps: the board task, over the columns.
    Across { width: f32 },
}

impl Rows {
    /// Give one row its place, and draw it there.
    fn place(self, ui: &mut Ui, row: impl FnOnce(&mut Ui)) {
        match self {
            Rows::Down => row(ui),
            Rows::Across { width } => {
                ui.allocate_ui(vec2(width, ui.spacing().interact_size.y), row);
            }
        }
    }
}

/// Every run and file the task has, each on its row.
pub(crate) fn draw_list(
    app: &App,
    ui: &mut Ui,
    task: RunsOf<'_>,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
    rows: Rows,
) {
    // Read once for the whole list: the mark that takes a run off the task asks first, and the
    // row that asked is the one drawn holding the question.
    let removing = app.model.board.pending_resource_delete.clone();
    for resource in task.resources {
        let pending = removing.as_deref() == Some(resource.id.as_str());
        rows.place(ui, |ui| {
            draw_resource(ui, task, resource, card, pending, palette, actions)
        });
    }

    // Under the runs and the files, because a review is asked for once the rest has happened -
    // and in the order the task's file lists them, which is the order they are to be deployed.
    for request in app
        .model
        .review_requests
        .iter()
        .filter(|request| request.task_id == task.id)
    {
        rows.place(ui, |ui| {
            draw_review_request(ui, request, card, palette, actions)
        });
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
    // Where the branch was drawn under the name, for a line that named one: the pointer is told
    // about the branch there, and about the commit everywhere else on the row.
    let mut branch_line: Option<Rect> = None;

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
                let line = ui.horizontal(|ui| {
                    ui.add_space(ROW_INSET + BRANCH_LINE_INDENT);
                    ui.label(
                        RichText::new(format!(
                            "#{}",
                            widgets::elide_end(branch, BRANCH_NAME_CHARS)
                        ))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                    );
                });
                branch_line = Some(line.response.rect);
            }
        },
    );

    // The whole of what was drawn is one target, taken after it so it covers both lines - which
    // puts it over the name as well, so the pointer on the name is on the row.
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
    // So the hover is the row's to say, all of it: one put on the name is under the row, and
    // `egui` shows the row's there and never the name's. Which of the two it says goes by
    // whether the pointer is on the branch.
    let on_branch_line = branch_line
        .zip(row.hover_pos())
        .is_some_and(|(line, pointer)| line.contains(pointer));
    let hover = match (&request.branch, &request.suggestion) {
        (Some(branch), _) if on_branch_line => format!(
            "#{branch}\nthe branch this commit belongs on - the review opens wherever it is \
             checked out"
        ),
        // What the agent wrote for the repo says what the review is about, which is more than
        // the row has room for. Whatever branch the repo is on: which commit the message is put
        // in is the commit pane's to decide, and this only says what was asked for.
        (_, Some(suggestion)) => commit_written_for(suggestion),
        (_, None) => "Open the review of this repo".to_string(),
    };
    let row = row
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(hover);
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

/// The commit a line of `request_for_review.txt` wrote for its repo, as the row's hover says it:
/// the subject, and under a blank line the paragraph, for a line that had one indented under it.
fn commit_written_for(suggestion: &CommitSuggestion) -> String {
    match suggestion.paragraph.is_empty() {
        true => suggestion.subject.clone(),
        false => suggestion.as_message(),
    }
}

/// How many characters of a linked file's path the card shows before the middle is cut out.
/// The end of a path is what tells files apart, so that is the part that is kept.
const FILE_PATH_CHARS: usize = 34;

/// How long a running agent has to have printed nothing before its dot turns amber. An agent
/// at work redraws its spinner many times a second and a long tool call still ticks; one
/// that has stopped is waiting - on a question it asked, or on the person - or is stuck.
/// Long enough that a pause between two tool calls does not flicker the dot.
const AGENT_QUIET_AFTER_SECS: u64 = 10;

/// What a run's dot says - see [`Activity`]. A shell asking for a person says so over
/// anything else; only an agent run reads as quiet, and only once it has been so for
/// [`AGENT_QUIET_AFTER_SECS`].
fn activity_of(resource: &TaskResourceView) -> Activity {
    // Going in a process that is no shell of this moon: that it is going is all there is to
    // say of it from here.
    if resource.going_elsewhere_in.is_some() {
        return Activity::Running;
    }
    if !resource.running {
        return Activity::Ended;
    }
    if resource.attention.is_some() {
        return Activity::Attention;
    }
    match resource.quiet_for_secs {
        Some(quiet) if quiet >= AGENT_QUIET_AFTER_SECS => Activity::Quiet,
        _ => Activity::Running,
    }
}

/// `2m 05s` for a hover, `45s` under a minute.
fn quiet_text(secs: u64) -> String {
    match (secs / 3600, (secs % 3600) / 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s:02}s"),
        (h, m, _) => format!("{h}h {m:02}m"),
    }
}

/// One shell, agent run or linked file of a task: what it is, whether it is still going, and
/// the way back to it.
fn draw_resource(
    ui: &mut Ui,
    task: RunsOf<'_>,
    resource: &TaskResourceView,
    card: &mut Controls,
    pending_delete: bool,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    match resource.kind {
        TaskResourceKind::File => {
            draw_file_resource(ui, task, resource, card, palette, actions);
            return;
        }
        TaskResourceKind::Visualization => {
            draw_visualization_resource(ui, task, resource, card, palette, actions);
            return;
        }
        TaskResourceKind::Shell | TaskResourceKind::Agent => {}
    }
    // A running shell is the way back to its tab; a run that has ended opens nothing, and
    // its row says so by staying unlit.
    let opens = resource.running && resource.terminal_id.is_some();
    let row = draw_row(ui, palette, opens, hover_of(resource.kind));
    let row_pressed = card.pressed(&row);
    draw_in_row(ui, row.rect, |ui| {
        let activity = activity_of(resource);
        activity_dot(ui, activity, palette);

        match (&resource.terminal_id, resource.running) {
            (Some(terminal_id), true) => {
                let hover = match (activity, resource.quiet_for_secs, &resource.attention) {
                    (Activity::Attention, _, Some(asking)) => {
                        format!("Open this shell in a tab\n{}", asking.asked.message())
                    }
                    (Activity::Quiet, Some(quiet), _) => format!(
                        "Open this shell in a tab\nnothing printed for {} - waiting on you?",
                        quiet_text(quiet)
                    ),
                    _ => "Open this shell in a tab".to_string(),
                };
                let name = widgets::quiet_button(ui, &resource.label).on_hover_text(hover);
                if card.pressed(&name) || row_pressed {
                    actions.push(BoardAction::OpenShell {
                        terminal_id: terminal_id.clone(),
                        command: (resource.agent != AgentKind::None).then_some(resource.agent),
                        task_id: task.id.to_string(),
                    });
                }
            }
            _ => {
                let name = ui.label(
                    RichText::new(&resource.label)
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                if let Some(pid) = resource.going_elsewhere_in {
                    name.on_hover_text(format!(
                        "Going in process {pid}, which is no shell of this window: another \
                         moon has it, or its agent was started outside one. It is opened \
                         where it was started"
                    ));
                }
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
                    match (is_shell, resource.going_elsewhere_in) {
                        (true, _) => "this ends the shell, and its scrollback goes with it",
                        (false, None) => "this ends the run and takes it off the task for good",
                        (false, Some(_)) => {
                            "this takes the run off the task, and leaves it going where it is"
                        }
                    },
                ) {
                    widgets::Confirmed::Yes => actions.push(BoardAction::DeleteResource(
                        task.id.to_string(),
                        resource.id.clone(),
                    )),
                    widgets::Confirmed::No => actions.push(BoardAction::CancelResourceDelete),
                    widgets::Confirmed::Waiting => {}
                }
                return;
            }
            let close = close_button(ui, palette).on_hover_text(
                match (is_shell, resource.running, resource.going_elsewhere_in) {
                    (true, ..) => "Close this shell",
                    (false, _, Some(_)) => "Take this run off the task, where it goes on going",
                    (false, true, None) => "End this run and take it off the task",
                    (false, false, None) => "Take this run off the task",
                },
            );
            if card.pressed(&close) {
                actions.push(BoardAction::ArmResourceDelete(resource.id.clone()));
            }
            if resource.running && !is_shell {
                let stop = widgets::quiet_button_colored(ui, "stop", palette.muted)
                    .on_hover_text("End this shell, keeping the run to come back to");
                if card.pressed(&stop) {
                    actions.push(BoardAction::Stop(task.id.to_string(), resource.id.clone()));
                }
            } else if resource.resumable {
                let resume = widgets::quiet_button_colored(ui, "resume", palette.accent)
                    .on_hover_text("Start this agent again where it left off");
                if card.pressed(&resume) {
                    actions.push(BoardAction::Resume(
                        task.id.to_string(),
                        resource.id.clone(),
                    ));
                }
            }
            // Last, in whatever room the name and the marks have left between them - and
            // never while the row holds the question of closing it, which returned above.
            draw_started_by(ui, resource, palette);
        });
    });
}

/// The least room a run's row has to have left for it to say whose the run is: under this
/// there is the cut-off mark and nothing of the login before it.
const ROOM_TO_SAY_WHOSE: f32 = 36.0;

/// Whose a run is and the work tree it went in, as its row says them: the login, and the
/// folder's own name when the run did not go in the checkout the board is in. `None` on a run
/// that says neither, which is every run of a server where nobody has a Unix user.
fn started_by_mark(resource: &TaskResourceView) -> Option<String> {
    let whose = resource
        .started_by
        .as_deref()
        .map(|login| format!("@{login}"));
    let where_it_went = resource.work_tree.as_deref().map(|work_tree| {
        let folder = std::path::Path::new(work_tree)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| work_tree.to_string());
        format!("in {folder}")
    });
    let said: Vec<String> = whose.into_iter().chain(where_it_went).collect();
    (!said.is_empty()).then(|| said.join(" "))
}

/// What a run's row says between its name and its marks, in the quiet ink: who started it,
/// and the work tree it went in. Cut off to the room the row has left, with the whole of it
/// on the hover; a row with no room says it on the hover of nothing, and the task's pane has
/// the room.
fn draw_started_by(ui: &mut Ui, resource: &TaskResourceView, palette: &Palette) {
    let Some(mark) = started_by_mark(resource) else {
        return;
    };
    if ui.available_width() < ROOM_TO_SAY_WHOSE {
        return;
    }
    let mut hover = Vec::new();
    if let Some(started_by) = resource.started_by.as_deref() {
        hover.push(format!(
            "Started by {started_by}: it runs as them, and resumes as them whoever asks"
        ));
    }
    if let Some(work_tree) = resource.work_tree.as_deref() {
        hover.push(format!("It ran in the work tree {work_tree}"));
    }
    ui.add(
        egui::Label::new(RichText::new(mark).size(SMALL_SIZE).color(palette.muted)).truncate(),
    )
    .on_hover_text(hover.join("\n"));
}

/// How far a row's fill reaches past its contents on either side.
const ROW_INSET: f32 = 3.0;

/// The row a run, a file or a pending review is listed on, taken before its contents are drawn:
/// lit while the pointer is over it, when a click on it would open something, so what the click
/// will do is plain before it is made. A row with nothing to open stays as it is. The row is a
/// click of its own, on everything the marks at its right do not cover - the marks are drawn
/// after it, so a click on one of them is the mark's and not the row's.
pub(super) fn draw_row(ui: &mut Ui, palette: &Palette, opens: bool, hover: &str) -> Response {
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
        TaskResourceKind::Visualization => "Open this visualization in a pane",
        TaskResourceKind::Shell | TaskResourceKind::Agent => "Open this shell in a tab",
    }
}

/// Draw a row's contents inside the space [`draw_row`] took for it, inset from its fill.
pub(super) fn draw_in_row(ui: &mut Ui, rect: Rect, contents: impl FnOnce(&mut Ui)) {
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
    task: RunsOf<'_>,
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
                task_id: task.id.to_string(),
                file_path: file_path.to_string(),
            });
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            let unlink = close_button(ui, palette).on_hover_text("Take this file off the task");
            if card.pressed(&unlink) {
                actions.push(BoardAction::DeleteResource(
                    task.id.to_string(),
                    resource.id.clone(),
                ));
            }
        });
    });
}

/// A visualization a run of the task showed: its name, which opens it again, and the mark that
/// takes it off the card.
///
/// Like a linked file, taking it off loses nothing and so goes without asking: its copy stays in
/// the task's folder. An agent still running that rewrites it puts it back.
fn draw_visualization_resource(
    ui: &mut Ui,
    task: RunsOf<'_>,
    resource: &TaskResourceView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    let Some(file_path) = resource.file_path.as_deref() else {
        panic!("visualization {} has no file path", resource.id);
    };
    let row = draw_row(ui, palette, true, hover_of(resource.kind));
    let row_pressed = card.pressed(&row);
    draw_in_row(ui, row.rect, |ui| {
        chart_mark(ui, palette);

        let name = widgets::quiet_button(ui, &resource.label)
            .on_hover_text(format!("Open {file_path} in a pane"));
        if card.pressed(&name) || row_pressed {
            actions.push(BoardAction::OpenVisualization {
                fragment_path: std::path::Path::new(&task.repo_path)
                    .join(file_path)
                    .display()
                    .to_string(),
            });
        }

        ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
            let unlink =
                close_button(ui, palette).on_hover_text("Take this visualization off the task");
            if card.pressed(&unlink) {
                actions.push(BoardAction::DeleteResource(
                    task.id.to_string(),
                    resource.id.clone(),
                ));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(running: bool, quiet_for_secs: Option<u64>) -> TaskResourceView {
        TaskResourceView {
            id: "run".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            label: "claude - 1".to_string(),
            file_path: None,
            terminal_id: running.then(|| "terminal-1".to_string()),
            running,
            going_elsewhere_in: None,
            quiet_for_secs,
            attention: None,
            resumable: true,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }
    }

    /// The dot turns amber only for a run that is going and has been quiet long enough: a
    /// run that ended is hollow however long ago it last printed, and one that only just went
    /// quiet is still green.
    #[test]
    fn a_run_reads_as_quiet_once_it_has_printed_nothing_for_long_enough() {
        assert_eq!(activity_of(&run(true, None)), Activity::Running);
        assert_eq!(activity_of(&run(true, Some(3))), Activity::Running);
        assert_eq!(
            activity_of(&run(true, Some(AGENT_QUIET_AFTER_SECS))),
            Activity::Quiet
        );
        assert_eq!(activity_of(&run(false, Some(600))), Activity::Ended);
        // Asking for a person outranks being quiet, which it usually also is.
        let mut asking = run(true, Some(600));
        asking.attention = Some(crate::api::TerminalAttentionView {
            terminal_id: "terminal-1".to_string(),
            name: None,
            asked: crate::attention::Asked::Notification("Permission needs input".to_string()),
            at_unix: 1,
        });
        assert_eq!(activity_of(&asking), Activity::Attention);
        asking.running = false;
        assert_eq!(activity_of(&asking), Activity::Ended);
    }

    /// Whose a run is and where it went are said together, and not at all by a run of a
    /// server where nobody has a Unix user.
    #[test]
    fn a_run_says_who_started_it_and_the_work_tree_it_went_in() {
        let run = |started_by: Option<&str>, work_tree: Option<&str>| TaskResourceView {
            started_by: started_by.map(str::to_string),
            work_tree: work_tree.map(str::to_string),
            ..run(false, None)
        };

        assert_eq!(started_by_mark(&run(None, None)), None);
        assert_eq!(
            started_by_mark(&run(Some("alice"), None)).as_deref(),
            Some("@alice")
        );
        assert_eq!(
            started_by_mark(&run(Some("alice"), Some("/home/moon-alice/wt"))).as_deref(),
            Some("@alice in wt")
        );
    }

    #[test]
    fn a_quiet_spell_reads_in_the_largest_unit_that_fits() {
        assert_eq!(quiet_text(45), "45s");
        assert_eq!(quiet_text(125), "2m 05s");
        assert_eq!(quiet_text(3900), "1h 05m");
    }

    /// A run going in another moon, or in an agent started outside one, has no shell here
    /// and is going all the same: its dot says so, rather than reading as a run that ended.
    #[test]
    fn a_run_going_in_another_process_reads_as_running() {
        let elsewhere = TaskResourceView {
            going_elsewhere_in: Some(4242),
            ..run(false, None)
        };

        assert_eq!(activity_of(&elsewhere), Activity::Running);
        assert_eq!(activity_of(&run(false, None)), Activity::Ended);
    }
}
