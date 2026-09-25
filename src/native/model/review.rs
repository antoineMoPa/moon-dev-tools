//! What the window holds of a review: the comments being written, the lines selected, and
//! where the list is to scroll.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::api::{CommitView, HunkView, SessionPayload};

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
    /// its last line - the pointer a pixel over the row boundary - has not selected anything
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
        let to = if index == end.line {
            end.column
        } else {
            LINE_END
        };
        Some((from, to))
    }
}

pub(crate) struct AgentLogView {
    /// The review the dispatch belongs to, so refreshing asks the right one.
    pub(crate) session_id: String,
    pub(crate) dispatch_key: String,
    pub(crate) text: String,
}

/// Where the diff pane is asked to scroll on its next draw: the top of a hunk, or one line
/// of it. A sidebar row, a move hint and an agent row ask for the hunk; the find bar asks for
/// the line its match is on, since a hunk can be taller than the window and its top says
/// nothing about where in it the match is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ScrollTo {
    pub(crate) hunk_id: String,
    /// A line of the hunk, indexed the way its parsed lines are - see `App::diff_lines` -
    /// which lands mid-screen. The hunk's top when there is none.
    pub(crate) line_index: Option<usize>,
}

impl ScrollTo {
    pub(crate) fn hunk(hunk_id: String) -> Self {
        Self {
            hunk_id,
            line_index: None,
        }
    }
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
    /// Set to ask the review pane to bring a hunk, or a line of one, into view on the next
    /// frame - see [`ScrollTo`].
    pub(crate) scroll_to: Option<ScrollTo>,
    /// Whether a pane too narrow for the sidebar beside the diff is showing the sidebar in
    /// its place - see `review::sidebar_fits_beside`.
    pub(crate) sidebar_in_front: bool,
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
    /// A name ⌘-clicked on a diff row, once it has been looked up and before a frame that
    /// can open a pane has read it. It waits on the review rather than on the pane showing it
    /// because the answer belongs to the review the click was made in - a second review open
    /// beside this one has its own - and a review pane has no editor of its own to park it on.
    pub(crate) looking_up: Option<crate::native::definition::LookedUp>,
    pub(crate) history_loaded: Vec<CommitView>,
    pub(crate) history_has_more: bool,
    pub(crate) loading_history: bool,
    pub(crate) pending_discard: Option<String>,
    /// What a click on a file's staging dot asked for - staged or not - by file path, while
    /// git is still being told. The row wears it at once rather than waiting out the round
    /// trip, and it is dropped as soon as a fetched diff says the same thing.
    pub(crate) asked_file_staging: HashMap<String, bool>,
    /// How far each hunk's code is scrolled sideways, in points, by hunk id. A hunk nobody
    /// has scrolled is not in here and sits at its left edge.
    pub(crate) code_scroll_x: HashMap<String, f32>,
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
            scroll_to: None,
            sidebar_in_front: false,
            selection: None,
            selecting_in: None,
            drafts: Vec::new(),
            expanded_patches: HashMap::new(),
            find_query: String::new(),
            find_match: None,
            looking_up: None,
            history_loaded: Vec::new(),
            history_has_more: false,
            loading_history: false,
            pending_discard: None,
            asked_file_staging: HashMap::new(),
            code_scroll_x: HashMap::new(),
        }
    }

    pub(crate) fn hunks(&self) -> &[HunkView] {
        self.payload
            .as_ref()
            .map(|payload| payload.hunks.as_slice())
            .unwrap_or_default()
    }

    pub(crate) fn hunk_by_id(&self, hunk_id: &str) -> Option<&HunkView> {
        self.hunks().iter().find(|hunk| hunk.id == hunk_id)
    }

    /// The first of a file's hunks in the order the diff lays them out, which is the order
    /// of the file: the one nearest its top.
    pub(crate) fn first_hunk_of(&self, file_path: &str) -> Option<&HunkView> {
        self.hunks().iter().find(|hunk| hunk.file_path == file_path)
    }

    pub(crate) fn read_only(&self) -> bool {
        self.payload
            .as_ref()
            .is_some_and(|payload| payload.read_only)
    }
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
        // The pointer crossed the row boundary but selected nothing on the lower line - the
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
