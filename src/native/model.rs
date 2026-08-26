//! What the window is showing. Plain data: everything here is `Send`, so a worker thread's
//! result can be applied to it without touching the UI's own state.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use egui_frames::{Layout, PaneId};

use crate::{
    api::{AgentKind, AgentLogPayload, CommitView, HunkView, SessionPayload, SubmoduleView},
    native::{panes::Pane, theme::ThemeMode},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Error,
}

pub(crate) struct Toast {
    pub(crate) kind: ToastKind,
    pub(crate) text: String,
    /// Frames left before it fades out. Counted down instead of timed so a stalled UI does
    /// not silently drop messages the user never saw.
    pub(crate) remaining: f32,
}

/// A comment being written against a run of lines in one hunk.
///
/// More than one can be open at a time: selecting elsewhere leaves a typed composer parked
/// where it is rather than moving it or throwing it away.
#[derive(Clone)]
pub(crate) struct Draft {
    pub(crate) hunk_id: String,
    pub(crate) file_path: String,
    pub(crate) header: String,
    /// The raw patch lines the comment is anchored to, exactly as they appear in the hunk.
    pub(crate) selection: String,
    pub(crate) note: String,
    /// Set when the composer has just opened, so the text box takes focus once.
    pub(crate) focus: bool,
    /// Set by the first press of cancel over typed text; the second press is the one that
    /// actually discards. Typing again puts the question away.
    pub(crate) pending_discard: bool,
}

/// One end of a selection: a line index into the hunk's parsed patch lines, and a character
/// column into that line's body text (the `+`/`-`/space marker removed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SelectionPoint {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

/// Marks "the end of whatever line this is on" without knowing how long the line is. Whole
/// lines are selected far more often than the length of each one is at hand.
pub(crate) const LINE_END: usize = usize::MAX;

/// The selected stretch of one hunk: character-precise between two points, so a single word
/// can be picked out of a line, while a plain click still takes the whole line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineSelection {
    pub(crate) hunk_id_hash: u64,
    pub(crate) anchor: SelectionPoint,
    pub(crate) head: SelectionPoint,
}

impl LineSelection {
    pub(crate) fn whole_line(hunk_id_hash: u64, line: usize) -> Self {
        Self {
            hunk_id_hash,
            anchor: SelectionPoint { line, column: 0 },
            head: SelectionPoint {
                line,
                column: LINE_END,
            },
        }
    }

    /// The two ends in document order, whichever way the sweep went.
    fn ordered(&self) -> (SelectionPoint, SelectionPoint) {
        if (self.head.line, self.head.column) < (self.anchor.line, self.anchor.column) {
            (self.head, self.anchor)
        } else {
            (self.anchor, self.head)
        }
    }

    /// The lines the selection actually covers. A selection that merely touches the start of
    /// its last line — the pointer a pixel over the row boundary — has not selected anything
    /// on it, so that line is left out. This is what makes selecting a single line by
    /// dragging possible at all: the row is 15px tall and a drag begins after 6px.
    pub(crate) fn line_range(&self) -> std::ops::RangeInclusive<usize> {
        let (start, end) = self.ordered();
        if end.line > start.line && end.column == 0 {
            start.line..=end.line - 1
        } else {
            start.line..=end.line
        }
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        self.line_range().contains(&index)
    }

    /// The span of characters covered on one line, if the line is part of the selection.
    /// `LINE_END` for the end column means "to the end of the line".
    pub(crate) fn columns_on(&self, index: usize) -> Option<(usize, usize)> {
        if !self.contains(index) {
            return None;
        }
        let (start, end) = self.ordered();
        let from = if index == start.line { start.column } else { 0 };
        let to = if index == end.line { end.column } else { LINE_END };
        Some((from, to))
    }
}

pub(crate) struct AgentLogView {
    /// The review the dispatch belongs to, so refreshing asks the right one.
    pub(crate) session_id: String,
    pub(crate) dispatch_key: String,
    pub(crate) text: String,
}

/// One review, and the UI state that belongs to it rather than to the window.
pub(crate) struct ReviewState {
    pub(crate) session_id: String,
    /// Shared rather than owned: the diff of a big file is megabytes of patch text, and the
    /// UI needs to read it while it also holds a mutable handle on the rest of the model.
    pub(crate) payload: Option<Arc<SessionPayload>>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    /// Bumped whenever an action changes the repo, so the poll loop refetches promptly.
    pub(crate) refresh_requested: bool,

    pub(crate) collapsed_files: HashSet<String>,
    pub(crate) active_hunk_id: Option<String>,
    /// Set to ask the review pane to bring a hunk into view on the next frame.
    pub(crate) scroll_to_hunk: Option<String>,
    pub(crate) selection: Option<LineSelection>,
    /// The hunk a drag is currently sweeping lines in, if the button is still down.
    pub(crate) selecting_in: Option<String>,
    /// Every comment currently being written, each drawn as its own composer. Selecting a
    /// new run opens a new one; the others stay parked at their anchors with their text.
    pub(crate) drafts: Vec<Draft>,
    /// Full patches fetched for hunks whose preview was truncated, keyed by hunk.
    pub(crate) expanded_patches: HashMap<String, String>,
    /// What the find bar over this review is looking for, so the lines being drawn can mark
    /// it. Empty when no bar is open on this pane.
    pub(crate) find_query: String,
    /// The one match the bar has stepped to, which is drawn differently from the rest.
    pub(crate) find_match: Option<crate::native::review::search::Match>,
    pub(crate) history_loaded: Vec<CommitView>,
    pub(crate) history_has_more: bool,
    pub(crate) loading_history: bool,
    pub(crate) pending_discard: Option<String>,
}

impl ReviewState {
    pub(crate) fn new(session_id: String) -> Self {
        Self {
            session_id,
            payload: None,
            error: None,
            loading: true,
            refresh_requested: false,
            collapsed_files: HashSet::new(),
            active_hunk_id: None,
            scroll_to_hunk: None,
            selection: None,
            selecting_in: None,
            drafts: Vec::new(),
            expanded_patches: HashMap::new(),
            find_query: String::new(),
            find_match: None,
            history_loaded: Vec::new(),
            history_has_more: false,
            loading_history: false,
            pending_discard: None,
        }
    }

    pub(crate) fn hunks(&self) -> &[HunkView] {
        self.payload
            .as_ref()
            .map(|payload| payload.hunks.as_slice())
            .unwrap_or_default()
    }

    pub(crate) fn read_only(&self) -> bool {
        self.payload
            .as_ref()
            .is_some_and(|payload| payload.read_only)
    }
}

/// The moontasks board: the tasks the server last reported, and what is being typed into it.
///
/// The board is the repo's `.moontasks` folder, which anything may write to, so nothing here
/// is authoritative — it is the last answer, redrawn until the next one arrives.
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
    /// The column the new-task box is open in, and the title being typed into it.
    pub(crate) composer_in: Option<crate::moontasks::ColumnId>,
    /// Which end of that column the box is standing at, which is the `+` that opened it. The
    /// box is drawn there, so it is where the card it becomes will appear.
    pub(crate) composer_at: crate::moontasks::ColumnEnd,
    /// The agent picked in the open new-task box, overriding the column's remembered
    /// default. Cleared when the box opens or closes: each column offers its own memory.
    pub(crate) composer_agent: Option<crate::api::AgentKind>,
    /// Set when the box has just opened, so it takes the keyboard once.
    pub(crate) composer_focus: bool,
    pub(crate) new_title: String,
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
    /// A notes file a board action just made sure exists, as the repo-relative path a file
    /// pane opens it by — waiting for the window the same way an opened shell does.
    pub(crate) opened_notes: Option<String>,
    /// The task whose title is being edited, if one is.
    pub(crate) renaming: Option<TaskRename>,
    /// Where the card being dragged would land. Worked out at the end of a frame and read by
    /// the next one, which is what lets the board draw the card where it is going instead of
    /// where it came from.
    pub(crate) landing: Option<TaskLanding>,
    /// The card that was just dropped, and the moment it was, so it can be marked for long
    /// enough to find it again among the ones it landed between.
    pub(crate) dropped: Option<TaskDropped>,
    /// A drop the server has not confirmed yet, kept so every board read until then can be
    /// answered with the card where it was put. Without it a read that was already on its way
    /// when the card was dropped puts it back where it came from for a moment.
    pub(crate) pending_place: Option<PendingPlace>,
    /// The column whose heading is being edited, if one is.
    pub(crate) renaming_column: Option<ColumnRename>,
    /// The column whose delete mark has been pressed once, so a stray click cannot take a
    /// column off the board.
    pub(crate) pending_column_delete: Option<crate::moontasks::ColumnId>,
    /// The new-column box at the right-hand end of the board, and what is being typed into it.
    pub(crate) column_composer_open: bool,
    pub(crate) column_composer_focus: bool,
    pub(crate) new_column_label: String,
    /// Where the column being dragged would land, counted in columns from the left. Worked out
    /// at the end of a frame and read by the next one, the same way a card's landing is.
    pub(crate) column_landing: Option<usize>,
    /// A column move the server has not confirmed yet, so every read until then can be
    /// answered with the column where it was put rather than where it came from.
    pub(crate) pending_column_place: Option<PendingColumnPlace>,
    /// The attach-a-session modal, while it is open.
    pub(crate) attach_picker: Option<AttachPicker>,
}

/// The modal that attaches one of an agent's own sessions to a task.
///
/// A task's recorded session id stops pointing anywhere when the user switches sessions
/// inside the agent, or the agent never persisted it — this is where a real one is picked
/// off the agents' own records instead.
pub(crate) struct AttachPicker {
    pub(crate) task_id: String,
    /// The card's title, so the modal says which task the session is going onto.
    pub(crate) task_title: String,
    /// What the agents' records had. `None` while they are still being read.
    pub(crate) sessions: Option<Vec<crate::agent_sessions::AgentSessionView>>,
    pub(crate) error: Option<String>,
    /// A session id typed or pasted by hand, for one the listing does not show — too old to
    /// make the newest few, or one nobody ever spoke in.
    pub(crate) manual_id: String,
    /// The agent the typed id belongs to. `None` until the user picks one.
    pub(crate) manual_agent: Option<crate::api::AgentKind>,
}

/// A drop that has been made on the board being drawn and not yet seen in one being read.
pub(crate) struct PendingPlace {
    pub(crate) task_id: String,
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

/// A card that has just been dropped, marked until [`Self::at`] is that long ago.
pub(crate) struct TaskDropped {
    pub(crate) task_id: String,
    /// egui's clock rather than a wall clock: it is what the fade is drawn against.
    pub(crate) at: f64,
}

/// A card's title, open for editing after a double click.
pub(crate) struct TaskRename {
    pub(crate) task_id: String,
    pub(crate) title: String,
    /// Set when the box has just opened, so it takes the keyboard once.
    pub(crate) focus: bool,
}


/// A shell the board started and wants shown.
pub(crate) struct OpenedShell {
    pub(crate) terminal_id: String,
    pub(crate) command: Option<AgentKind>,
    pub(crate) task_id: String,
}

/// The command palette, and the query typed into it.
pub(crate) struct PaletteState {
    pub(crate) open: bool,
    /// Whether the query is picking a command or naming a file of the repo.
    pub(crate) mode: crate::native::palette::PaletteMode,
    /// What the file finder has found for the query it last searched for.
    pub(crate) files: crate::native::palette::FileSearch,
    pub(crate) query: String,
    pub(crate) highlighted: usize,
    /// The query the highlight was picked under. A keystroke changes which commands are on
    /// the list, so a highlight from before it means nothing — Enter should run the first
    /// match of what is on screen now, not whichever row the old highlight lands on.
    pub(crate) highlight_query: String,
    /// Where the palette drew last frame. A press outside it puts the palette away, and that
    /// has to be known before this frame draws — the box takes the keyboard when it draws, and
    /// a click meant for a shell would lose it again.
    pub(crate) rect: Option<egui::Rect>,
}

impl PaletteState {
    /// Open it on an empty query, at the top of the list, and drawn nowhere yet.
    pub(crate) fn show(&mut self) {
        self.open = true;
        self.mode = crate::native::palette::PaletteMode::Commands;
        // Whatever the last file search found belongs to the query that is being cleared.
        self.files = crate::native::palette::FileSearch::default();
        self.query.clear();
        self.highlighted = 0;
        self.highlight_query.clear();
        self.rect = None;
    }

    /// The same, on the file finder: what is typed names a file of the repo rather than a
    /// command.
    pub(crate) fn show_files(&mut self) {
        self.show();
        self.mode = crate::native::palette::PaletteMode::Files;
    }

    /// Put it away. The rect goes with it so the next one it draws is the one clicks are
    /// measured against.
    pub(crate) fn dismiss(&mut self) {
        self.open = false;
        self.rect = None;
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            open: false,
            mode: crate::native::palette::PaletteMode::Commands,
            files: crate::native::palette::FileSearch::default(),
            query: String::new(),
            highlighted: 0,
            highlight_query: String::new(),
            rect: None,
        }
    }
}

/// What the window is doing before it has a review to show. Opening one runs a handful of
/// git commands, or a round-trip to another machine, so it cannot happen before the window
/// appears.
pub(crate) enum Stage {
    /// Waiting to be told which repo to review, which is how a remote connection starts
    /// when the address was given without a path.
    Prompt {
        repo_path: String,
        error: Option<String>,
    },
    Opening,
    Ready,
}

pub(crate) struct Model {
    pub(crate) stage: Stage,
    pub(crate) theme: ThemeMode,
    /// The panes, and the frames and splits they are arranged in.
    pub(crate) layout: Layout<Pane>,
    /// The review the window was launched on. Submodule reviews are opened beside it.
    pub(crate) root_session_id: String,
    /// The review the last shell was started in. A new shell asked for from a frame that
    /// names no review — a frame of shells, say — opens where the previous one did.
    pub(crate) last_shell_session_id: Option<String>,
    pub(crate) reviews: HashMap<String, ReviewState>,
    pub(crate) submodules: Vec<SubmoduleView>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) palette: PaletteState,
    pub(crate) board: BoardState,
    pub(crate) agent_log: Option<AgentLogView>,
    /// `local`, or the address of the server this window is reviewing through.
    pub(crate) connection: String,
    /// Set once a review is open, so the window picks up shells the server already has.
    pub(crate) adopt_shells_pending: bool,
    /// The same, for the shell `moonshell` opens on: it needs a session to start in.
    pub(crate) open_shell_pending: bool,
    /// The arrangement the last run left behind, applied once the first review opens.
    pub(crate) restored_layout: Option<Layout<Pane>>,
    /// The agent the last run ended on, applied to the session once the review opens.
    pub(crate) restored_agent: Option<AgentKind>,
    /// What each review's commit pane is holding: the message being written, and the last
    /// run. Keyed by review rather than by pane, so closing the tab keeps the message.
    pub(crate) commit_panes: HashMap<String, crate::native::commit_pane::CommitPane>,
    /// The files open in tabs of their own, keyed by the pane showing each one.
    pub(crate) file_editors: HashMap<PaneId, crate::native::file_pane::FileEditor>,
    /// What the markdown renderer keeps between frames — loaded images above all — shared by
    /// every file pane that is previewing.
    pub(crate) markdown_cache: egui_commonmark::CommonMarkCache,
    /// The find bar, when one is open, and the pane it is searching.
    pub(crate) find: Option<crate::native::find::Find>,
    /// The widget id of the last shell the keyboard was in. The review's copy chord checks
    /// it against egui's focus to leave cmd+c to a shell the user just selected text in.
    pub(crate) terminal_with_keyboard: Option<egui::Id>,
    /// A project that has just opened, waiting to be written to the recent list. Set on the
    /// worker thread's result, which is in no position to touch the settings file.
    pub(crate) opened_project: Option<String>,
    /// The project this window is on, once one is open. What the title bar says.
    pub(crate) project_path: Option<String>,
}

impl Model {
    pub(crate) fn review(&mut self, session_id: &str) -> &mut ReviewState {
        self.reviews
            .entry(session_id.to_string())
            .or_insert_with(|| ReviewState::new(session_id.to_string()))
    }

    pub(crate) fn review_ref(&self, session_id: &str) -> Option<&ReviewState> {
        self.reviews.get(session_id)
    }

    /// Close every pane reviewing this session, which is what a commit that took the whole of
    /// the working tree leaves behind: a diff with nothing in it. The review's own state stays,
    /// so opening it again picks up where it left off.
    pub(crate) fn close_review_panes(&mut self, session_id: &str) {
        let reviewing: Vec<_> = self
            .layout
            .panes()
            .filter(|(_, pane)| pane.reviews(session_id))
            .map(|(pane_id, _)| pane_id)
            .collect();
        for pane_id in reviewing {
            self.layout.close_pane(pane_id);
        }
    }

    pub(crate) fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        // A repeated message means the same thing; refresh it instead of stacking copies.
        if let Some(existing) = self.toasts.iter_mut().find(|toast| toast.text == text) {
            existing.remaining = TOAST_LIFETIME;
            existing.kind = kind;
            return;
        }
        self.toasts.push(Toast {
            kind,
            text,
            remaining: TOAST_LIFETIME,
        });
    }

    pub(crate) fn info(&mut self, text: impl Into<String>) {
        self.toast(ToastKind::Info, text);
    }

    pub(crate) fn error(&mut self, text: impl Into<String>) {
        self.toast(ToastKind::Error, text);
    }

    /// Report the outcome of an action: quiet on success, visible on failure.
    pub(crate) fn report(&mut self, outcome: anyhow::Result<()>, context: &str) {
        if let Err(error) = outcome {
            self.error(format!("{context}: {error}"));
        }
    }

    pub(crate) fn set_agent_log(&mut self, session_id: String, payload: AgentLogPayload) {
        self.agent_log = Some(AgentLogView {
            session_id,
            dispatch_key: payload.dispatch_key,
            text: payload.text,
        });
    }

    pub(crate) fn tick_toasts(&mut self, seconds: f32) {
        for toast in &mut self.toasts {
            toast.remaining -= seconds;
        }
        self.toasts.retain(|toast| toast.remaining > 0.0);
    }
}

/// How long a toast stays up, in seconds.
pub(crate) const TOAST_LIFETIME: f32 = 6.0;

pub(crate) fn hash_of(value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(anchor: (usize, usize), head: (usize, usize)) -> LineSelection {
        LineSelection {
            hunk_id_hash: 1,
            anchor: SelectionPoint {
                line: anchor.0,
                column: anchor.1,
            },
            head: SelectionPoint {
                line: head.0,
                column: head.1,
            },
        }
    }

    #[test]
    fn a_clicked_line_covers_exactly_itself() {
        let selection = LineSelection::whole_line(1, 4);

        assert_eq!(selection.line_range(), 4..=4);
        assert_eq!(selection.columns_on(4), Some((0, LINE_END)));
        assert_eq!(selection.columns_on(3), None);
    }

    #[test]
    fn a_sweep_that_only_touches_the_next_line_s_start_leaves_it_out() {
        // The pointer crossed the row boundary but selected nothing on the lower line — the
        // jitter at the end of a one-line drag.
        assert_eq!(selection((4, 2), (5, 0)).line_range(), 4..=4);
        // The moment it covers a character, the line is in.
        assert_eq!(selection((4, 2), (5, 1)).line_range(), 4..=5);
    }

    #[test]
    fn a_sweep_upward_reads_the_same_as_one_downward() {
        let up = selection((6, 3), (4, 1));

        assert_eq!(up.line_range(), 4..=6);
        assert_eq!(up.columns_on(4), Some((1, LINE_END)));
        assert_eq!(up.columns_on(5), Some((0, LINE_END)));
        assert_eq!(up.columns_on(6), Some((0, 3)));
    }

    #[test]
    fn an_upward_sweep_that_starts_at_a_line_s_first_column_leaves_that_line_out() {
        // Pressed at the very start of line 6, swept up: nothing on line 6 is covered.
        assert_eq!(selection((6, 0), (4, 1)).line_range(), 4..=5);
    }

    #[test]
    fn a_word_selection_is_one_line_with_its_columns() {
        let word = selection((2, 8), (2, 13));

        assert_eq!(word.line_range(), 2..=2);
        assert_eq!(word.columns_on(2), Some((8, 13)));
    }
}
