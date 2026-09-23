//! What the window is showing. Plain data: everything here is `Send`, so a worker thread's
//! result can be applied to it without touching the UI's own state.

mod board;
mod palette;
mod review;

pub(crate) use board::{
    AttachPicker, BoardState, CardMenu, Carrying, ColumnRename, OpenedFile, OpenedShell,
    PendingCard, PendingColumnPlace, PendingPlace, TabRename, TaskDraft, TaskEditor, TaskLanding,
    TaskRename,
};
pub(crate) use palette::PaletteState;
pub(crate) use review::{
    AgentLogView, Draft, LINE_END, LineSelection, ReviewState, ScrollTo, SelectionPoint,
};

use std::collections::HashMap;

use egui_frames::{Layout, PaneId};

use crate::{
    api::{AgentKind, AgentLogPayload, RepoStatusView},
    moontasks::ReviewRequestView,
    native::{panes::Pane, theme::ThemeMode, workspace_color::WorkspaceColor},
    project::{ProjectCommand, ProjectConfig},
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
    /// The color this window's ground is painted, which is how one window is told from
    /// another. Read from the settings once the project is known - see
    /// `App::follow_project_color` - and changed by the palette's commands or the project
    /// pane's swatches.
    pub(crate) workspace_color: WorkspaceColor,
    /// The panes, and the frames and splits they are arranged in.
    pub(crate) layout: Layout<Pane>,
    /// The review the window was launched on. Submodule reviews are opened beside it.
    pub(crate) root_session_id: String,
    /// The review the last shell was started in. A new shell asked for from a frame that
    /// names no review - a frame of shells, say - opens where the previous one did.
    pub(crate) last_shell_session_id: Option<String>,
    pub(crate) reviews: HashMap<String, ReviewState>,
    /// The reviewed repo and how many of its files have changed, as the submodule hub shows
    /// it at the top of its list. None until the hub's first answer arrives.
    pub(crate) root_repo_status: Option<RepoStatusView>,
    pub(crate) submodules: Vec<RepoStatusView>,
    /// What the submodule hub's box is being narrowed by. It lives on the model rather
    /// than in the pane so it survives the pane being closed and opened again.
    pub(crate) submodule_filter: String,
    /// Set when the hub is opened or brought forward, so its box takes the keyboard: the
    /// hub is a list to find one submodule in, and typing is how it is found.
    pub(crate) submodule_filter_focus: bool,
    /// Every repo the board's tasks have asked to have looked at, in deploy order - see
    /// [`crate::moontasks::ReviewRequestView`]. Kept on the model rather than carried on a
    /// [`crate::moontasks::TaskView`] because the commit pane reads it too, and it has to be
    /// there whether or not a board is open.
    pub(crate) review_requests: Vec<ReviewRequestView>,
    /// How many times the rows above have been changed from the board - a line dismissed or
    /// crossed off - since the window opened. The change is made to the rows at once and to
    /// the file on a worker thread, and a read of the files that started before the change
    /// would put the row back for a tick: the read carries the count it started under, and
    /// is dropped if the count has moved on - see `App::poll_review_requests`.
    pub(crate) review_request_amendments: u64,
    /// The shells the server says have something running in them, as of the last poll. What
    /// quitting would interrupt is these rather than every open shell, so this is what the
    /// quit warning is about - see `App::quit_would_kill_shells`.
    pub(crate) shells_running_a_command: Vec<String>,
    /// The shells asking for a person, as of the last poll - see [`crate::attention`]. Each
    /// ask is posted to the messages once, which `attention_posted` is the record of.
    pub(crate) shells_wanting_attention: Vec<crate::api::TerminalAttentionView>,
    /// When each shell's last posted ask was made, by terminal id.
    pub(crate) attention_posted: HashMap<String, u64>,
    pub(crate) toasts: Vec<Toast>,
    /// Every message the window has posted, toast or error, whether or not it was read
    /// before it faded - see [`crate::native::messages`].
    pub(crate) messages: crate::native::messages::MessageLog,
    /// What each session's language servers are doing, as of the last poll - see
    /// [`crate::native::status_bar`]. Keyed by session, because a window reviewing a
    /// submodule beside its repo has a set of servers per review.
    pub(crate) language_servers_working: HashMap<String, crate::native::status_bar::ServersWorking>,
    pub(crate) palette: PaletteState,
    /// The rename under way, if there is one - see [`crate::native::renaming`].
    pub(crate) renaming: Option<crate::native::renaming::Renaming>,
    /// Places a language server named, waiting for a frame that can go to them or list them -
    /// see [`crate::native::places`].
    pub(crate) places_found: Option<crate::native::places::FoundPlaces>,
    /// Code actions being asked for or picked from - see [`crate::native::code_actions`].
    pub(crate) code_acting: Option<crate::native::code_actions::CodeActing>,
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
    /// The visualizations agents have announced, and the pages their panes show - see
    /// [`crate::native::visualizations`].
    pub(crate) visualizations: crate::native::visualizations::Visualizations,
    /// The agent the last run ended on, applied to the session once the review opens.
    pub(crate) restored_agent: Option<AgentKind>,
    /// What each review's commit pane is holding: the message being written, and the last
    /// run. Keyed by review rather than by pane, so closing the tab keeps the message.
    pub(crate) commit_panes: HashMap<String, crate::native::commit_pane::CommitPane>,
    /// The files open in tabs of their own, keyed by the pane showing each one.
    pub(crate) file_editors: HashMap<PaneId, crate::native::file_pane::FileEditor>,
    /// The dated entry `Tools › Work Log` asked for, by the tab it is to go into, while that
    /// tab's text is still on its way - see [`crate::native::work_log`]. The tab is opened
    /// by the answer to a call that makes the file, which has no editor to put it in yet.
    pub(crate) work_log_entries_waiting: HashMap<PaneId, crate::native::work_log::NewEntry>,
    /// The extensions open in tabs, keyed by the pane each one draws.
    pub(crate) extension_panes: HashMap<PaneId, crate::native::extension_pane::ExtensionPane>,
    /// What the markdown renderer keeps between frames - loaded images above all - shared by
    /// every file pane that is previewing.
    pub(crate) markdown_cache: egui_commonmark::CommonMarkCache,
    /// The find bar, when one is open, and the pane it is searching.
    pub(crate) find: Option<crate::native::find::Find>,
    /// The widget id of the last shell the keyboard was in. The review's copy chord checks
    /// it against egui's focus to leave cmd+c to a shell the user just selected text in.
    pub(crate) terminal_with_keyboard: Option<egui::Id>,
    /// What the server said each shell is called, for every shell whose tab has asked: an
    /// agent's shell is named as it starts - `claude - 1` - and any shell is named by retyping
    /// its tab's title. `None` for a shell that has no name, whose tab reads what the program
    /// in it sets. The server holds the name, since a task's shell outlives its tab; a tab
    /// asks once, the first time it is drawn without an answer, and a rename from here keeps
    /// the answer up - see `App::read_terminal_name`.
    pub(crate) terminal_names: HashMap<String, Option<String>>,
    /// The shell's tab whose title is open for retyping, if one is.
    pub(crate) renaming_tab: Option<TabRename>,
    /// A project that has just opened, waiting to be written to the recent list. Set on the
    /// worker thread's result, which is in no position to touch the settings file.
    pub(crate) opened_project: Option<String>,
    /// The project this window is on, once one is open. What the title bar says.
    pub(crate) project_path: Option<String>,
    /// The commands the Project menu runs, as the repo's `.moonreview.json` has them. Read
    /// when the review opens, and again whenever the configuration pane is opened or saves,
    /// because the file is one a person may also edit by hand.
    pub(crate) project: ProjectConfig,
    /// Set when a review opens, so the commands above are read for it.
    pub(crate) project_pending: bool,
    /// Set when the configuration pane is opened, so its first box takes the keyboard: the
    /// pane is two boxes and nothing else, and typing is what it is opened for.
    pub(crate) project_focus: bool,
    /// What the configuration pane's two boxes hold. Seeded from the file when the pane is
    /// opened, and what a save writes back.
    pub(crate) project_editor: Option<ProjectEditor>,
    /// Set by a keystroke in one of those boxes, cleared by the write it causes. The pane
    /// saves as it is typed in, and this is what keeps that to one write at a time: a second
    /// keystroke while a write is in flight is written by the next one rather than by a write
    /// of its own, which could land in either order.
    pub(crate) project_unsaved: bool,
    /// The shell whose end restarts the window: a `build and run` of a project whose run
    /// command is the restart word. The line typed into that shell only exits on a build that
    /// came out well - a failed one keeps the shell open on its errors - so the shell ending
    /// is the rebuilt program being ready to start. Cleared when the tab is closed by hand,
    /// which is the restart being called off.
    pub(crate) restart_on_shell_exit: Option<String>,
    /// The shell the Project menu's commands are typed into. A build asked for a second time
    /// goes back to the shell the first one ran in - the output of both is then in one tab,
    /// read the way a shell one typed the command into oneself is read - rather than opening
    /// another tab beside it every time.
    ///
    /// Only while that shell is open and waiting at its prompt: a shell that has been closed,
    /// or that still has something running in it, is not one to type a build into, and the
    /// next command opens a shell of its own.
    pub(crate) project_shell: Option<String>,
}

/// What the configuration pane holds, mid-edit: the two commands as text, and the
/// indentation as the choice it is.
///
/// The commands are text rather than commands because a box someone has emptied is still a
/// box: it becomes a command that is not set only when the pane saves - see
/// [`ProjectConfig::typed`].
#[derive(Default)]
pub(crate) struct ProjectEditor {
    pub(crate) build: String,
    pub(crate) run: String,
    /// What a Tab press puts into this repo's files. Not a box, so a click on the row picks
    /// it outright and the pane saves the moment it is picked.
    pub(crate) indent: egui_moon_editor::Indent,
}

impl ProjectEditor {
    /// The box one of the menu's commands is typed in. The one place a command is paired
    /// with its box, so the pane draws the two rows from the same list it reads them by.
    pub(crate) fn text_mut(&mut self, which: ProjectCommand) -> &mut String {
        match which {
            ProjectCommand::Build => &mut self.build,
            ProjectCommand::Run => &mut self.run,
            // Built out of the two boxes rather than stored, so it has no box of its own.
            ProjectCommand::BuildAndRun => unreachable!("build and run has no box"),
        }
    }

    pub(crate) fn of(config: &ProjectConfig) -> Self {
        Self {
            build: config.build.clone().unwrap_or_default(),
            run: config.run.clone().unwrap_or_default(),
            indent: config.indent(),
        }
    }
}

impl Model {
    /// The repo the window was launched on, once its review has answered - which is the repo
    /// the board's folder is in. `None` until then, and on a window that is still asking which
    /// repo to open.
    pub(crate) fn root_repo_path(&self) -> Option<std::path::PathBuf> {
        let payload = self.review_ref(&self.root_session_id)?.payload.as_ref()?;
        Some(std::path::PathBuf::from(&payload.repo_path))
    }

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

    /// The same for the commit pane of a review, once it has done what it was opened for.
    pub(crate) fn close_commit_pane(&mut self, session_id: &str) {
        let committing: Vec<_> = self
            .layout
            .panes()
            .filter(|(_, pane)| pane.commits(session_id))
            .map(|(pane_id, _)| pane_id)
            .collect();
        for pane_id in committing {
            self.layout.close_pane(pane_id);
        }
    }

    pub(crate) fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        // Written down before anything else, and every time it is posted: the toast below
        // folds a repeat into the one already up, and the log is where "it happened again"
        // is recorded - see [`crate::native::messages`].
        self.messages.record(
            kind,
            text.clone(),
            crate::native::messages::now_unix(),
            std::time::Instant::now(),
        );
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

    /// The shells asking for a person, freshly polled. A notification not posted yet goes to
    /// the messages as a line from the shell - once: the next poll carries the same ask, and a
    /// message repeated every second would be the desktop notification nobody wanted.
    ///
    /// A bare bell is not posted at all. Shells ring it for a finished command, a completion
    /// that found nothing, a `^G` in a pager - and a toast for each of those is a window
    /// forever saying "rang its bell" about nothing. The bell still marks the shell's card
    /// red, which is where a look can be taken when there is time for one.
    pub(crate) fn take_attention(&mut self, asking: Vec<crate::api::TerminalAttentionView>) {
        for ask in &asking {
            let crate::attention::Asked::Notification(message) = &ask.asked else {
                continue;
            };
            if self.attention_posted.get(&ask.terminal_id) == Some(&ask.at_unix) {
                continue;
            }
            self.attention_posted
                .insert(ask.terminal_id.clone(), ask.at_unix);
            self.info(format!(
                "{}: {message}",
                ask.name.as_deref().unwrap_or("a shell")
            ));
        }
        self.shells_wanting_attention = asking;
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
