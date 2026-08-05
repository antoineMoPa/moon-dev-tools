//! What the window is showing. Plain data: everything here is `Send`, so a worker thread's
//! result can be applied to it without touching the UI's own state.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    api::{AgentLogPayload, CommitView, HunkView, SessionPayload, SubmoduleView},
    native::{layout::WorkspaceLayout, theme::ThemeMode},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarTab {
    Files,
    Comments,
}

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

pub(crate) struct FullFileView {
    pub(crate) file_path: String,
    pub(crate) content: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct AgentLogView {
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

    pub(crate) sidebar_tab: SidebarTab,
    pub(crate) show_reviewed: bool,
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
    pub(crate) full_file: Option<FullFileView>,
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
            sidebar_tab: SidebarTab::Files,
            show_reviewed: true,
            collapsed_files: HashSet::new(),
            active_hunk_id: None,
            scroll_to_hunk: None,
            selection: None,
            selecting_in: None,
            draft: None,
            expanded_patches: HashMap::new(),
            full_file: None,
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

    /// How the review pane is filtered right now.
    pub(crate) fn filter(&self) -> HunkFilter {
        HunkFilter {
            show_reviewed: self.show_reviewed,
        }
    }
}

/// The sidebar's reviewed toggle, applied to a payload the caller already holds. Keeping this
/// separate from [`ReviewState`] is what lets the review pane read the hunks through a shared
/// payload while it still edits the rest of the model.
pub(crate) struct HunkFilter {
    pub(crate) show_reviewed: bool,
}

impl HunkFilter {
    pub(crate) fn matches(&self, hunk: &HunkView) -> bool {
        self.show_reviewed || !hunk.reviewed
    }

    pub(crate) fn apply<'h>(&self, hunks: &'h [HunkView]) -> Vec<&'h HunkView> {
        hunks.iter().filter(|hunk| self.matches(hunk)).collect()
    }
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
    pub(crate) layout: WorkspaceLayout,
    /// The review the window was launched on. Submodule reviews are opened beside it.
    pub(crate) root_session_id: String,
    pub(crate) reviews: HashMap<String, ReviewState>,
    pub(crate) submodules: Vec<SubmoduleView>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) palette: PaletteState,
    pub(crate) agent_log: Option<AgentLogView>,
    /// `local`, or the address of the server this window is reviewing through.
    pub(crate) connection: String,
    /// Set once a review is open, so the window picks up shells the server already has.
    pub(crate) adopt_shells_pending: bool,
    /// The arrangement the last run left behind, applied once the first review opens.
    pub(crate) restored_layout: Option<WorkspaceLayout>,
    /// Set once the window should ask a frame to close a pane whose terminal has gone.
    pub(crate) show_shortcuts: bool,
    /// The tab being dragged, if any: its pane id.
    pub(crate) dragging_pane: Option<String>,
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

    pub(crate) fn set_agent_log(&mut self, payload: AgentLogPayload) {
        self.agent_log = Some(AgentLogView {
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
