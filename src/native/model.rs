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
}

/// A run of selected lines inside one hunk, as indices into its preview lines.
#[derive(Clone, Copy)]
pub(crate) struct LineSelection {
    pub(crate) hunk_id_hash: u64,
    pub(crate) anchor: usize,
    pub(crate) head: usize,
}

impl LineSelection {
    pub(crate) fn range(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.min(self.head)..=self.anchor.max(self.head)
    }

    pub(crate) fn contains(&self, index: usize) -> bool {
        self.range().contains(&index)
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
    pub(crate) draft: Option<Draft>,
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
            draft: None,
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
    pub(crate) error: Option<String>,
    pub(crate) loaded: bool,
    /// Whether the new-task box is open in the TODO column, and the title being typed into
    /// it. The agent it will start is not here: that is the one the review's selector holds,
    /// so picking one on the board and picking one in the review are the same choice.
    pub(crate) composer_open: bool,
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
    /// The task whose title is being edited, if one is.
    pub(crate) renaming: Option<TaskRename>,
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
    pub(crate) query: String,
    pub(crate) highlighted: usize,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            highlighted: 0,
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
    /// The files open in tabs of their own, keyed by the pane showing each one.
    pub(crate) file_editors: HashMap<PaneId, crate::native::file_pane::FileEditor>,
    /// The find bar, when one is open, and the pane it is searching.
    pub(crate) find: Option<crate::native::find::Find>,
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
