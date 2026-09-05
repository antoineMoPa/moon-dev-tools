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
    moontasks::{ColumnEnd, ColumnId, TaskView},
    native::{
        app::App,
        board::{
            self,
            actions::{BoardAction, CreateFromDraft, TaskPaneBox},
        },
        model::TaskEditor,
        theme::{Palette, SMALL_SIZE},
        widgets,
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

pub(crate) fn draw(app: &mut App, ui: &mut Ui, task_id: &str) {
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
                draw_while_the_board_is_read(ui, &palette);
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

/// The pane a new task is written on, before there is a task to write it on: the same two
/// boxes, with `[create]` standing where the task's `[start]` will.
///
/// `[create]` makes the task and turns this very pane into that task's own, in the tab it is
/// already in. Nothing is created before it is pressed - not by leaving the boxes, not by
/// closing the tab: the folder under `.moontasks` is named after the title and keeps that name
/// for the rest of the task's life, so a task created to be named later would be a folder
/// called nothing for the rest of its life.
pub(crate) fn draw_new_task(
    app: &mut App,
    ui: &mut Ui,
    column: &ColumnId,
    joins: ColumnEnd,
    draft_id: &str,
) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            let mut created = None;
            let width = (ui.available_width() - ui.spacing().scroll.bar_width).min(COLUMN_WIDTH);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("new-task-pane")
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(TOP_GAP);
                        ui.allocate_ui(vec2(width, 0.0), |ui| {
                            ui.vertical(|ui| {
                                created = draw_draft(app, ui, column, joins, draft_id);
                            });
                        });
                    });
                });

            if let Some(create) = created {
                board::actions::apply(app, BoardAction::Create(create));
            }
        });
}

/// The two boxes of a new-task pane and the `[create]` under them: `Some` once the button has
/// been pressed, or Enter answered for a title, with something in the title box.
///
/// The button stands where the task's `[start]` will, so the pane keeps its shape as it becomes
/// the task's own. Escape clears the title rather than putting one back, because there is no
/// title yet to put back.
fn draw_draft(
    app: &mut App,
    ui: &mut Ui,
    column: &ColumnId,
    joins: ColumnEnd,
    draft_id: &str,
) -> Option<CreateFromDraft> {
    // The keyboard the `+` promised this pane, taken on the frame it opened on rather than
    // asked for again on every frame it draws.
    let takes_keyboard = app
        .model
        .board
        .task_box_focus
        .as_ref()
        .is_some_and(|(id, which)| id == draft_id && *which == TaskPaneBox::Title);
    if takes_keyboard {
        app.model.board.task_box_focus = None;
    }
    let draft = app
        .model
        .board
        .drafts
        .entry(draft_id.to_string())
        .or_default();
    // Nothing is answered for twice: the task is already being made from what is in the boxes.
    let creating = draft.creating;

    let title_id = ui.id().with("new-task-title");
    // Taken before the box is drawn, so the box never sees it and never adds the line.
    let entered = ui.memory(|memory| memory.has_focus(title_id))
        && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let title = ui.add(
        egui::TextEdit::multiline(&mut draft.title)
            .id(title_id)
            .hint_text("Task title")
            .desired_width(f32::INFINITY)
            .desired_rows(1)
            .margin(egui::Margin::symmetric(6, 4)),
    );
    if takes_keyboard {
        title.request_focus();
    }
    if title.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        draft.title.clear();
    }

    ui.add_space(LINE_GAP);
    ui.add(
        egui::TextEdit::multiline(&mut draft.notes)
            .hint_text("Notes")
            .desired_width(f32::INFINITY)
            .desired_rows(NOTES_ROWS)
            .margin(egui::Margin::symmetric(6, 4)),
    );
    let title = draft.title.trim().to_string();
    let notes = draft.notes.clone();

    ui.add_space(LINE_GAP);
    // Off while the box is empty - a task is its title - and while the one that was written is
    // already being made, so the button cannot ask for the same task twice.
    let ready = !title.is_empty() && !creating;
    let create =
        widgets::clickable(ui.add_enabled(ready, egui::Button::new("[create]").frame(false)))
            .on_hover_text("Make this task's card, and open it here");

    if !ready || !(entered || create.clicked()) {
        return None;
    }
    Some(CreateFromDraft {
        draft_id: draft_id.to_string(),
        column: column.clone(),
        joins,
        title,
        notes,
        // Enter is someone still writing: the notes are what they write next. The button is
        // pressed with the hand, which has left the keyboard where it was.
        keyboard_goes_to_notes: entered,
    })
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
/// allowed to take back what was just typed. The notes have a second way of being behind the
/// box, which is [`accept_notes`].
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
            written_notes: None,
        });
    if editor.said_title != task.title {
        editor.title.clone_from(&task.title);
        editor.said_title.clone_from(&task.title);
    }
    accept_notes(editor, &task.notes);

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
    let thrown_away =
        title.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
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
        // Held until the board reads these words back, so the answer that carries them - and
        // every answer still on its way from before them - cannot put the box back to what it
        // said before the next letters were typed.
        editor.written_notes = Some(editor.notes.clone());
        actions.push(BoardAction::SaveNotes {
            task_id: task.id.clone(),
            notes: editor.notes.clone(),
        });
    } else if editor.notes_typed_at.is_some() {
        // The wait is being counted in frames, so it needs frames to count.
        ui.ctx().request_repaint();
    }
}

/// Take what the board says a task's notes are into the box, or leave the box as it is.
///
/// The box is written to `notes.md` a moment after the typing stops, and the board is read
/// again both on a timer and as soon as that write lands. So the answer that comes back says
/// what the notes were when the typing paused, while the box has gone on being typed into -
/// and filling the box in from it would take those letters back. The editor holds the words it
/// wrote until an answer carries them, and passes over every answer until one does: they are
/// all older than the box.
///
/// An answer nobody here wrote is somebody else writing the file - an agent, or the file open
/// in a pane beside this - and the box takes it.
fn accept_notes(editor: &mut TaskEditor, notes: &str) {
    if editor.said_notes == notes {
        return;
    }
    editor.said_notes = notes.to_string();
    match &editor.written_notes {
        // The write has been read back: the file and the box are talking about the same words
        // again, and the box keeps whatever has been typed on top of them since.
        Some(written) if written == notes => editor.written_notes = None,
        // An answer from before the write, or from between two of them.
        Some(_) => {}
        None => editor.notes = notes.to_string(),
    }
}

/// What the pane is while the board has no answer about this task: the first read of the board,
/// or the moment between a task being created and the read that first has it. There is nothing
/// to draw about a task nobody has heard of, and the read is a moment away.
///
/// A task the board did have and no longer has has been deleted, and that closes the tab rather
/// than leaving it standing here - see [`crate::native::board::cards::accept_board`].
fn draw_while_the_board_is_read(ui: &mut Ui, palette: &Palette) {
    ui.label(
        RichText::new("reading the board…")
            .size(SMALL_SIZE)
            .color(palette.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::accept_notes;
    use crate::native::model::TaskEditor;

    /// An editor with words in the box, the board's last answer, and a write waiting to be
    /// read back.
    fn editor(notes: &str, said_notes: &str, written_notes: Option<&str>) -> TaskEditor {
        TaskEditor {
            title: "Write the parser".to_string(),
            notes: notes.to_string(),
            notes_typed_at: None,
            said_title: "Write the parser".to_string(),
            said_notes: said_notes.to_string(),
            written_notes: written_notes.map(str::to_string),
        }
    }

    /// The answer the write itself asked for: it carries the words as they stood when the
    /// typing paused, and the box has been typed into since. The box keeps its letters, and
    /// the wait is over - the board and the box are talking about the same words again.
    #[test]
    fn the_answer_carrying_our_own_write_does_not_take_back_what_was_typed_after_it() {
        let mut editor = editor("Ship it by Friday", "Ship it", Some("Ship it by"));

        accept_notes(&mut editor, "Ship it by");

        assert_eq!(
            editor.notes, "Ship it by Friday",
            "the box kept its letters"
        );
        assert_eq!(editor.said_notes, "Ship it by");
        assert_eq!(editor.written_notes, None, "the write has been read back");
    }

    /// A read already on its way when the write landed answers with what was in the file
    /// before it. It is older than the box twice over and says nothing the box does not know.
    #[test]
    fn an_answer_from_before_the_write_is_passed_over() {
        let mut editor = editor("Ship it by Friday", "Ship", Some("Ship it by"));

        accept_notes(&mut editor, "Ship it");

        assert_eq!(
            editor.notes, "Ship it by Friday",
            "the box kept its letters"
        );
        assert_eq!(
            editor.written_notes.as_deref(),
            Some("Ship it by"),
            "the write is still waiting to be read back"
        );
    }

    /// Nobody here wrote this - the file changed beside the box, an agent or an editor - so
    /// the box takes it.
    #[test]
    fn an_answer_nobody_here_wrote_is_taken_into_the_box() {
        let mut editor = editor("Ship it by Friday", "Ship it by Friday", None);

        accept_notes(&mut editor, "Ship it by Friday, says the agent");

        assert_eq!(editor.notes, "Ship it by Friday, says the agent");
        assert_eq!(editor.said_notes, "Ship it by Friday, says the agent");
    }

    /// The board saying again what it said last time is the ordinary frame, and nothing about
    /// the box moves.
    #[test]
    fn the_answer_the_board_gave_last_time_changes_nothing() {
        let mut editor = editor("Ship it by Friday", "Ship it", Some("Ship it"));

        accept_notes(&mut editor, "Ship it");

        assert_eq!(editor.notes, "Ship it by Friday");
        assert_eq!(
            editor.written_notes.as_deref(),
            Some("Ship it"),
            "an answer that has not changed is not the write being read back"
        );
    }
}
