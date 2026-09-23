//! What the window holds of the task board: the cards being carried, renamed, tagged and
//! written, and the task tabs opened off them.

use std::collections::{HashMap, HashSet};

use egui_frames::PaneId;

use crate::api::AgentKind;

/// The moontasks board: the tasks the server last reported, and what is being typed into it.
///
/// The board is the repo's `.moontasks` folder, which anything may write to, so nothing here
/// is authoritative - it is the last answer, redrawn until the next one arrives.
#[derive(Default)]
pub(crate) struct BoardState {
    pub(crate) tasks: Vec<crate::moontasks::TaskView>,
    /// The board's columns, left to right, as the last read had them. Empty until the first
    /// answer arrives, which is what `loaded` says.
    pub(crate) columns: Vec<crate::moontasks::BoardColumn>,
    pub(crate) error: Option<String>,
    pub(crate) loaded: bool,
    /// What is typed into the filter bar over the columns. Every column shows the cards that
    /// match it and nothing else; empty is a board showing all of its cards.
    pub(crate) filter: String,
    /// Set when the filter box is to take the keyboard next frame, which is how cmd+F over the
    /// board reaches it.
    pub(crate) filter_focus: bool,
    /// The tasks being written on a pane of their own before they exist, one for each new-task
    /// pane open, under the draft id that pane carries.
    pub(crate) drafts: HashMap<String, TaskDraft>,
    /// Where the card being written on the new-task pane will land, while it is being written.
    /// The column draws an empty card there, so what is being written has its place on the
    /// board from the moment the `+` is pressed.
    pub(crate) card_being_written: Option<PendingCard>,
    /// Set when something changed the board, so the next frame refetches rather than waiting
    /// out the poll interval.
    pub(crate) refresh_requested: bool,
    /// The task whose delete button has been pressed once, so a stray click cannot throw a
    /// task's folder away.
    pub(crate) pending_delete: Option<String>,
    /// The same, for a run being taken off a task.
    pub(crate) pending_resource_delete: Option<String>,
    /// A shell a board action just started, waiting for the window to open a tab on it. The
    /// backend call finishes on a worker thread, which is in no position to touch the panes.
    pub(crate) opened_shell: Option<OpenedShell>,
    /// A file a board action just readied - the task's notes, made sure to exist, or a file
    /// just linked to a card - waiting for the window the same way an opened shell does.
    pub(crate) opened_file: Option<OpenedFile>,
    /// A visualization kept on a task whose row was just clicked, by its copy's path, waiting
    /// for the next frame the way an opened file does: opened while the card or the task's pane
    /// is drawn, the click would hand the keyboard straight back to the frame it landed in.
    pub(crate) opened_visualization: Option<String>,
    /// A task a board action just made, by its id and title, whose page is to open on the next
    /// frame the way a started shell's tab does.
    pub(crate) opened_task_page: Option<(String, String)>,
    /// The title and notes as they are being typed on a task's own pane, one for each pane
    /// open, so the board reading itself again does not overwrite a half-typed word.
    pub(crate) task_editors: HashMap<String, TaskEditor>,
    /// The pane that is to open with the keyboard in one of its boxes, and which box: the
    /// notes for a click on a card's notes, since that click is someone about to write them,
    /// and the title for a new-task pane, which is what the `+` was pressed to write. Named by
    /// the task the pane is of, or by the draft id of one that has no task yet. Taken by the
    /// pane on the first frame it draws.
    pub(crate) task_box_focus: Option<(String, crate::native::board::actions::TaskPaneBox)>,
    /// The task whose title is being edited, if one is.
    pub(crate) renaming: Option<TaskRename>,
    /// What is being typed into a task's tag box, one for each task whose box has been open.
    /// Kept per task the way the panes' editors are, so a card's box and that task's pane
    /// are writing the same word rather than two.
    pub(crate) tagging: HashMap<String, TagComposer>,
    /// The card whose tag box is open inside it, if one is - opened by its `[tags]`.
    pub(crate) tagging_card: Option<String>,
    /// The cards the board has marked. One is a task to read - its page opens with it -
    /// and several are a group to drag. See [`crate::native::board::selection`].
    pub(crate) marked: HashSet<String>,
    /// The card a shift+click measures its run from: the last one clicked.
    pub(crate) mark_anchor: Option<String>,
    /// The tasks whose pages are to be put away, because a click on the board let their cards
    /// go. Drained once the window has drawn - a pane is never closed while the tree that
    /// holds it is being drawn.
    pub(crate) pages_to_close: Vec<String>,
    /// Whether the cross on the empty card standing for a task being written has been pressed:
    /// the pane it is being written on is to be put away, and put away it is once the window
    /// has drawn, for the same reason the pages above are.
    pub(crate) new_task_let_go_of: bool,
    /// The press the pointer is making on the board, if it is making one. The board works out
    /// what a press was from where it began and how far it carried, rather than asking egui,
    /// because what it is asking about is cards - see [`crate::native::board::gesture`].
    pub(crate) press: Option<crate::native::board::gesture::Press>,
    /// The cards a drag is carrying, once the press has carried far enough to be one.
    pub(crate) carrying: Option<Carrying>,
    /// Where the board itself was drawn last frame. A column lays out every card it has and
    /// the board lays out every column, room for them or not, so a card's place carries on past
    /// the board's edge and under whatever pane is next to it - and a press over there is not
    /// that card's. This is the whole of where the board's cards can be pressed.
    ///
    /// `None` until the board has been drawn once, which is a board with nowhere to press.
    pub(crate) showing: Option<egui::Rect>,
    /// Which way this scrolling gesture is moving the board: sideways across the columns, or
    /// up and down inside one. Settled on the first frame of the gesture and let go of when
    /// the scrolling stops - see [`crate::native::board::hold_the_off_axis`].
    pub(crate) scroll_axis: Option<crate::native::board::motion::Axis>,
    /// Where the card being dragged would land. Worked out at the end of a frame and read by
    /// the next one, which is what lets the board draw the card where it is going instead of
    /// where it came from.
    pub(crate) landing: Option<TaskLanding>,
    /// A drop the server has not confirmed yet, kept so every board read until then can be
    /// answered with the card where it was put. Without it a read that was already on its way
    /// when the card was dropped puts it back where it came from for a moment.
    pub(crate) pending_place: Option<PendingPlace>,
    /// The column whose heading is being edited, if one is.
    pub(crate) renaming_column: Option<ColumnRename>,
    /// The column whose delete mark has been pressed once, so a stray click cannot take a
    /// column off the board.
    pub(crate) pending_column_delete: Option<crate::moontasks::ColumnId>,
    /// The new-column box and what is being typed into it.
    pub(crate) column_composer_open: bool,
    pub(crate) column_composer_focus: bool,
    /// Where the box is standing, counted in columns from the left - so the column it is
    /// standing in for is added there rather than at the end. `None` is the right-hand end,
    /// which is where the board's own `+` opens it.
    pub(crate) column_composer_at: Option<usize>,
    pub(crate) new_column_label: String,
    /// Where the column being dragged would land, counted in columns from the left. Worked out
    /// at the end of a frame and read by the next one, the same way a card's landing is.
    pub(crate) column_landing: Option<usize>,
    /// A column move the server has not confirmed yet, so every read until then can be
    /// answered with the column where it was put rather than where it came from.
    pub(crate) pending_column_place: Option<PendingColumnPlace>,
    /// The attach-a-session modal, while it is open.
    pub(crate) attach_picker: Option<AttachPicker>,
    /// The card a right click opened the task menu on, while that menu is up.
    pub(crate) card_menu: Option<CardMenu>,
}

/// A card's own menu, and where on the board it was opened.
///
/// The position is kept rather than taken from the pointer each frame: the menu stands where
/// the click was made, and the hand is free to travel down it.
pub(crate) struct CardMenu {
    pub(crate) task_id: String,
    pub(crate) at: egui::Pos2,
}

/// The modal that attaches one of an agent's own sessions to a task.
///
/// A task's recorded session id stops pointing anywhere when the user switches sessions
/// inside the agent, or the agent never persisted it - this is where a real one is picked
/// off the agents' own records instead.
pub(crate) struct AttachPicker {
    pub(crate) task_id: String,
    /// The card's title, so the modal says which task the session is going onto.
    pub(crate) task_title: String,
    /// What the agents' records had. `None` while they are still being read.
    pub(crate) sessions: Option<Vec<crate::agent_sessions::AgentSessionView>>,
    pub(crate) error: Option<String>,
    /// A session id typed or pasted by hand, for one the listing does not show - too old to
    /// make the newest few, or one nobody ever spoke in.
    pub(crate) manual_id: String,
    /// The agent the typed id belongs to. `None` until the user picks one.
    pub(crate) manual_agent: Option<crate::api::AgentKind>,
}

/// The cards a drag is carrying: the one on the cursor, and the run it is bringing with it -
/// the marks, or the one card alone.
#[derive(Clone)]
pub(crate) struct Carrying {
    pub(crate) primary: String,
    /// Every card being carried, in the order the board holds them.
    pub(crate) task_ids: Vec<String>,
}

impl Carrying {
    pub(crate) fn carries(&self, task_id: &str) -> bool {
        self.task_ids.iter().any(|carried| carried == task_id)
    }
}

/// A drop that has been made on the board being drawn and not yet seen in one being read.
///
/// A drop carries every card that was picked up, and they land as a run from `index`.
pub(crate) struct PendingPlace {
    pub(crate) task_ids: Vec<String>,
    pub(crate) status: crate::moontasks::ColumnId,
    pub(crate) index: usize,
}

/// The same, for a column dragged to another place on the board.
pub(crate) struct PendingColumnPlace {
    pub(crate) column_id: crate::moontasks::ColumnId,
    pub(crate) index: usize,
}

/// The place a dragged card would take: a column, and how many of that column's other cards
/// are above it.
#[derive(Clone, PartialEq)]
pub(crate) struct TaskLanding {
    pub(crate) status: crate::moontasks::ColumnId,
    pub(crate) index: usize,
}

/// A column's heading, open for editing after a double click.
pub(crate) struct ColumnRename {
    pub(crate) column_id: crate::moontasks::ColumnId,
    pub(crate) label: String,
    /// Set when the box has just opened, so it takes the keyboard once.
    pub(crate) focus: bool,
}

/// A shell's tab title, open for editing after a double click on the tab - the same shape a
/// column's heading has.
pub(crate) struct TabRename {
    pub(crate) pane_id: PaneId,
    pub(crate) name: String,
    /// Set when the box has just opened, so it takes the keyboard once.
    pub(crate) focus: bool,
}

/// The tag being typed into one task's tag box.
#[derive(Default)]
pub(crate) struct TagComposer {
    pub(crate) text: String,
    /// Set when the box is to take the keyboard next frame: the frame the card's box opened
    /// on, and the frame after a tag was entered, so the next one is typed without reaching
    /// for it.
    pub(crate) focus: bool,
    /// The tags last written to the task, until the board reads back exactly those.
    ///
    /// The write goes out on a worker thread, and the next tag is typed well inside the time
    /// it takes to come back. Built on the task's tags as the board still has them, that next
    /// tag would be written over the one before; built on these, it goes after it. Several
    /// writes can be out at once, and only the last of them is what the box stands for.
    pub(crate) sent: Option<Vec<String>>,
}

impl TagComposer {
    /// The tags the box stands for: the ones last sent, until the board has them.
    pub(crate) fn tags_over(&mut self, board_tags: &[String]) -> Vec<String> {
        if self.sent.as_deref() == Some(board_tags) {
            self.sent = None;
        }
        self.sent.clone().unwrap_or_else(|| board_tags.to_vec())
    }
}

/// A card's title, open for editing after a double click.
/// One task's title and notes, open for editing on that task's pane.
pub(crate) struct TaskEditor {
    pub(crate) title: String,
    pub(crate) notes: String,
    /// When the notes were last typed into, on `egui`'s own clock. The notes are written a
    /// moment after the typing stops rather than on every letter, and this is that moment
    /// being waited for; `None` once they are written.
    pub(crate) notes_typed_at: Option<f64>,
    /// What the board last said the title and the notes were. The boxes are filled in again
    /// when the board's answer changes from this and not otherwise, so an answer that has not
    /// caught up with what was just typed cannot take it back.
    pub(crate) said_title: String,
    pub(crate) said_notes: String,
    /// The notes handed to `SaveNotes` and not read back off the board yet. A read that was
    /// already on its way when they were written answers with what was in the file before
    /// them, and the read the write itself asks for answers with them as they stood at that
    /// moment - neither is allowed to take back the letters typed since, so answers are passed
    /// over until one carries this. `None` once one does, or where nothing has been written.
    pub(crate) written_notes: Option<String>,
}

pub(crate) struct TaskRename {
    pub(crate) task_id: String,
    pub(crate) title: String,
    /// Set when the box has just opened, so it takes the keyboard once.
    pub(crate) focus: bool,
    /// Where the title sat when the double click opened this box. The third click of a triple
    /// lands within a few pixels of the first two, so a click in here while the box is open is
    /// the triple finishing on it - even where the box is drawn narrower than the title was.
    pub(crate) title_rect: egui::Rect,
}

/// A file the board readied and wants shown: where it is, and the task it was opened from,
/// which the pane carries so the board can mark that task's card while the file is in front.
pub(crate) struct OpenedFile {
    /// Relative to the repo, which is how a file pane opens one.
    pub(crate) file_path: String,
    pub(crate) task_id: String,
}

/// Where the card being written on the new-task pane is going: the column the `+` belonged to,
/// and which of its two ends it was.
pub(crate) struct PendingCard {
    pub(crate) column: crate::moontasks::ColumnId,
    pub(crate) joins: crate::moontasks::ColumnEnd,
}

/// A task being written on a new-task pane, before there is a task to write it on.
///
/// The `+` on a column opens the pane and nothing else: the folder under `.moontasks` is named
/// after the title and keeps that name for the rest of the task's life, so the task is not
/// created until there is a title to name it after.
#[derive(Default)]
pub(crate) struct TaskDraft {
    pub(crate) title: String,
    pub(crate) notes: String,
    /// Set while the task is being created, so leaving the title box again on the way out does
    /// not create a second task.
    pub(crate) creating: bool,
}

/// A shell the board started and wants shown.
pub(crate) struct OpenedShell {
    pub(crate) terminal_id: String,
    pub(crate) command: Option<AgentKind>,
    pub(crate) task_id: String,
}
