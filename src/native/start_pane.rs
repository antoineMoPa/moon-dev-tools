//! The start window: one task, in a tab of its own. This is what a click on a card opens.
//!
//! Its title and its notes are edited here, in the pane rather than through a file opened
//! beside it - a task you have just opened is one you are about to say something about - and
//! under them are the runs and files it has and the `[start]` button that adds another. Those
//! two are the card's own, drawn from [`crate::native::board::resources`] and
//! [`crate::native::board::start`] rather than laid out again here: one task said twice would
//! be two things to keep in step.
//!
//! Starting a shell or an agent from here closes the pane, because the shell it opens is what
//! this pane was standing in for.

use egui::{Key, Modifiers, RichText, Ui, vec2};

use crate::{
    moontasks::TaskView,
    native::{
        app::App,
        board::{self, actions::{BoardAction, TaskPaneBox}},
        model::TaskEditor,
        theme::{Palette, SMALL_SIZE},
    },
};

/// The gap over the first line, so the pane starts off the tab strip rather than under it.
const TOP_GAP: f32 = 14.0;
/// The gap between the boxes, and between them and what stands under them.
const LINE_GAP: f32 = 16.0;
/// How wide the pane's column is at its widest. A title box the width of a window is a box you
/// lose the caret in, and notes read as prose rather than as a line per screen.
const COLUMN_WIDTH: f32 = 520.0;
/// How many lines of notes the box stands open at. Enough for a paragraph, and it grows with
/// what is written into it.
const NOTES_ROWS: usize = 8;
/// How long the notes are left alone before they are written, once the typing stops. Long
/// enough not to write a file per letter, short enough that closing the tab a moment later
/// keeps what was said.
const NOTES_SETTLE: f64 = 0.8;

pub(crate) fn draw(app: &mut App, ui: &mut Ui, task_id: &str, title: &str) {
    let palette = app.palette_of();

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            let Some(task) = app
                .model
                .board
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
            else {
                draw_missing(app, ui, title, &palette);
                return;
            };

            let mut actions = Vec::new();
            // Scrolled, because none of what is on it has a size of its own: the notes box
            // grows with what is written into it, and a task that has been worked in for a
            // while has more to say than a pane can hold.
            let width = (ui.available_width() - ui.spacing().scroll.bar_width).min(COLUMN_WIDTH);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("task-pane")
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(TOP_GAP);
                        ui.allocate_ui(vec2(width, 0.0), |ui| {
                            ui.vertical(|ui| {
                                draw_editors(app, ui, &task, &mut actions);
                                ui.add_space(LINE_GAP);
                                // What the task has going, in the rows the card lists them in
                                // - the same rows, so a run is stopped, resumed or opened from
                                // whichever of the two you happen to be looking at. A task
                                // with nothing on it says so instead.
                                if task.resources.is_empty() {
                                    ui.label(
                                        RichText::new("nothing is running in this task yet")
                                            .color(palette.muted),
                                    );
                                } else {
                                    board::resources::draw_list(
                                        app,
                                        ui,
                                        &task,
                                        &mut board::gesture::Controls::elsewhere(),
                                        &palette,
                                        &mut actions,
                                    );
                                }
                                ui.add_space(LINE_GAP);
                                board::start::draw_button(
                                    app,
                                    ui,
                                    &task,
                                    &mut board::gesture::Controls::elsewhere(),
                                    &mut actions,
                                );
                            });
                        });
                    });
                });

            for action in actions {
                board::actions::apply(app, action);
            }
        });
}

/// The task's title and its notes, open for editing.
///
/// The title is kept by Enter or by clicking away from the box, and thrown away by Escape - the
/// same three answers the box on the card gives. Its box is a multiline one so that a long
/// title wraps the way it does on the card - in the same letters the card writes it in, since
/// it is the same title and the tab above is already saying it - rather than scrolling
/// sideways out of sight, and
/// Enter is taken off that box and made the keep, because a title is still one line.
///
/// The notes are kept on their own a moment after the typing stops, because there is no one
/// keystroke that ends a paragraph.
///
/// Both boxes are filled in again when the board's answer changes under them - a title renamed
/// on the card, or notes written in the file beside this - and not otherwise: an answer that
/// still has the old title in it, because the rename has not been read back yet, must not be
/// allowed to take back what was just typed.
fn draw_editors(app: &mut App, ui: &mut Ui, task: &TaskView, actions: &mut Vec<BoardAction>) {
    let now = ui.input(|input| input.time);
    // A pane opened by a click on the card's notes opens with the keyboard in that box, which
    // is what the click was reaching for; one opened for a task just created opens in the
    // title, which is what is still being written. Taken here rather than left set, so it is
    // the one frame the pane opened on and not every frame it draws.
    let takes_keyboard = app
        .model
        .board
        .task_box_focus
        .as_ref()
        .filter(|(task_id, _)| task_id == &task.id)
        .map(|(_, which)| *which);
    if takes_keyboard.is_some() {
        app.model.board.task_box_focus = None;
    }
    let notes_take_keyboard = takes_keyboard == Some(TaskPaneBox::Notes);
    let editor = app
        .model
        .board
        .task_editors
        .entry(task.id.clone())
        .or_insert_with(|| TaskEditor {
            title: task.title.clone(),
            notes: task.notes.clone(),
            said_title: task.title.clone(),
            said_notes: task.notes.clone(),
            notes_typed_at: None,
        });
    if editor.said_title != task.title {
        editor.title.clone_from(&task.title);
        editor.said_title.clone_from(&task.title);
    }
    if editor.said_notes != task.notes {
        editor.notes.clone_from(&task.notes);
        editor.said_notes.clone_from(&task.notes);
    }

    let title_id = ui.id().with("task-title");
    // Taken before the box is drawn, so the box never sees it and never adds the line.
    let entered = ui.memory(|memory| memory.has_focus(title_id))
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let title = ui.add(
        egui::TextEdit::multiline(&mut editor.title)
            .id(title_id)
            .hint_text("Task title")
            .desired_width(f32::INFINITY)
            .desired_rows(1)
            .margin(egui::Margin::symmetric(6, 4)),
    );
    if takes_keyboard == Some(TaskPaneBox::Title) {
        title.request_focus();
    }
    // Escape throws the typing away rather than leaving a title on that is not the task's;
    // egui takes the keyboard off the box with the same press, which is what `lost_focus` is
    // below, so the key is taken here before that is read.
    let thrown_away = title.has_focus()
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if thrown_away {
        editor.title.clone_from(&task.title);
    }
    let kept = !thrown_away
        && (entered || title.lost_focus())
        && !editor.title.trim().is_empty()
        && editor.title.trim() != task.title;
    if kept {
        actions.push(BoardAction::Rename(
            task.id.clone(),
            editor.title.trim().to_string(),
        ));
    }

    ui.add_space(LINE_GAP);
    let mut written = egui::TextEdit::multiline(&mut editor.notes)
        .desired_width(f32::INFINITY)
        .desired_rows(NOTES_ROWS)
        .margin(egui::Margin::symmetric(6, 4))
        .show(ui);
    if notes_take_keyboard {
        written.response.request_focus();
        // With the caret past what is already written, which is where writing more starts. A
        // box handed the keyboard with the caret at the top would take the next sentence into
        // the middle of the first one.
        let end = egui::text::CCursor::new(editor.notes.chars().count());
        written
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(end)));
        written.state.clone().store(ui.ctx(), written.response.id);
    }
    let notes = written.response;
    if notes.changed() {
        editor.notes_typed_at = Some(now);
    }
    // Written when the typing has stopped for a moment, or the moment the box is left - which
    // is what makes closing the tab straight after a word safe.
    let settled = editor
        .notes_typed_at
        .is_some_and(|at| now - at >= NOTES_SETTLE);
    if editor.notes_typed_at.is_some() && (settled || notes.lost_focus()) {
        editor.notes_typed_at = None;
        actions.push(BoardAction::SaveNotes {
            task_id: task.id.clone(),
            notes: editor.notes.clone(),
        });
    } else if editor.notes_typed_at.is_some() {
        // The wait is being counted in frames, so it needs frames to count.
        ui.ctx().request_repaint();
    }
}

/// Either the board has not been read yet or the task has been deleted from under the tab.
/// Nothing can be started on a task the board does not have, so this is the whole pane - named,
/// because the tab may be one of several.
fn draw_missing(app: &App, ui: &mut Ui, title: &str, palette: &Palette) {
    ui.label(
        RichText::new(if app.model.board.loaded {
            format!("{title} is no longer on the board")
        } else {
            "reading the board…".to_string()
        })
        .size(SMALL_SIZE)
        .color(palette.muted),
    );
}


