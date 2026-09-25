//! The window itself: what it holds, how it is built, and what it hands to eframe.

mod actions;
mod draw;
mod settings;
mod switching;
#[cfg(not(target_arch = "wasm32"))]
mod windows;

pub(crate) use draw::window_title;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use egui_frames::{Frames, Layout, PaneId};
use web_time::Instant;

use crate::{
    api::AgentKind,
    backend::Backend,
    native::{
        Launch,
        bindings::Keymap,
        model::{Model, Stage, hash_of},
        palette::CommandAction,
        panes::Pane,
        review::diff::{DiffLine, attach_syntax, build_diff_lines},
        tasks::Tasks,
        theme::{self, Palette, ThemeMode},
        workspace_color::WorkspaceColor,
    },
};

/// How often an open review refetches, so staging from a shell or another window shows up.
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(900);
/// The same, for a window that is not focused.
pub(super) const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How long a warned-about quit stays armed. The warning is a toast, so this is how long that
/// toast is up: pressing again while it can still be read is the second press it asks for.
pub(super) const QUIT_CONFIRM_WINDOW: Duration =
    Duration::from_millis((crate::native::model::TOAST_LIFETIME * 1000.0) as u64);

/// How often an open moontasks board rereads `.moontasks`. Slower than a review: reading it
/// is a directory walk, and a card moves at the pace an agent works rather than a keystroke.
pub(super) const BOARD_POLL_INTERVAL: Duration = Duration::from_millis(1500);

/// How many frames from startup the window-theme command is repeated over - see
/// [`theme::apply_window_theme`]. Enough frames for the window to be fully up, and few enough
/// to be over in the blink the window takes to appear.
const WINDOW_THEME_FRAMES: u8 = 5;

/// What the window was asked to do with its tabs this frame. Both the menu bar and the
/// keyboard can ask, and on macOS one ⌘W can arrive as both, so the request is a single slot
/// that is acted on once a frame rather than a call made from wherever it came in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabAction {
    New,
    Close,
}

/// Terminals attach on a worker thread, because a remote one opens a socket. The emulator
/// itself is `!Send`, so the finished attachment is handed back here to be turned into a
/// pane on the UI thread.
pub(crate) type AttachInbox = Arc<Mutex<Vec<AttachedTerminal>>>;

/// A shell that finished attaching, waiting for the UI thread to turn it into a live pane.
pub(crate) struct AttachedTerminal {
    pub(crate) terminal_id: String,
    pub(crate) attachment: Result<egui_tty::TtyStream>,
    pub(crate) held_by: TerminalHolder,
}

/// Which of the window's two sets of emulators a shell belongs to.
///
/// They are held apart because they end differently: a workspace shell that exits takes its
/// tab with it, while a commit run's emulator is kept after `git` is gone - what it printed is
/// the account of how the commit went, and the pane goes on showing it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalHolder {
    Workspace,
    CommitPane,
}

pub(crate) struct App {
    pub(crate) model: Model,
    pub(crate) tasks: Tasks,
    pub(crate) terminals: HashMap<String, egui_tty::Terminal>,
    /// The pty of each commit pane's last run, kept until that pane runs something else.
    pub(crate) commit_terminals: HashMap<String, egui_tty::Terminal>,
    /// The workspace widget: the tab strips, the splits, and a drag in flight.
    pub(crate) frames: Frames,
    pub(crate) attaching: AttachInbox,
    /// Panes whose terminal could not be attached, so the pane can say so.
    pub(crate) terminal_errors: HashMap<String, String>,
    last_poll: Instant,
    /// When the repo and its submodules were last asked how much is changed in them - see
    /// [`App::poll_submodules`].
    last_submodules_poll: Instant,
    last_board_poll: Instant,
    /// When the window last asked which shells have something running in them.
    last_running_shells_poll: Instant,
    /// When the board's review requests were last read - see [`App::poll_review_requests`].
        last_review_requests_poll: Instant,
    /// When the window last asked what the language servers are doing - see
    /// [`crate::native::status_bar`]. Kept here rather than per pane: the question is about
    /// the session's servers, and every pane on that session is waiting on the same answer.
    pub(super) last_lsp_work_poll: Instant,
    /// Deferred so a pane is never added or removed while the tree holding it is drawn.
    pub(crate) pending_action: Option<CommandAction>,
    pub(crate) pending_close: Option<PaneId>,
    /// The tab whose menu said "close other tabs": every other tab of its frame is asked to
    /// close, one at a time.
    pub(crate) pending_close_of_others: Option<PaneId>,
    pending_tab_action: Option<TabAction>,
    /// The chord that raises each tab within cmd+1..cmd+9's reach - the active frame's tabs -
    /// worked out before the strips are drawn and worn at the right of their titles.
    pub(crate) tab_shortcuts: HashMap<PaneId, String>,
    /// The tab the keyboard was last handed to, so a different one coming to the front -
    /// however it got there - is noticed once rather than every frame it stays there.
    pub(crate) keyboard_pane: Option<PaneId>,
    /// The tab that was in front last frame, so one arriving is noticed once rather than every
    /// frame it stays there. See [`App::follow_task_in_front`].
    pub(crate) front_pane: Option<PaneId>,
    /// The tab owed the keyboard, waiting for its own draw to take it. A shell and a file
    /// editor can only ask for focus from inside the widget that would hold it.
    pub(crate) pane_taking_keyboard: Option<PaneId>,
    /// Whether this window's file panes talk to language servers at all - see
    /// [`crate::native::lsp_document`].
    ///
    /// Off as an `App` is built, and turned on by the window `crate::native::run` opens,
    /// which is the only caller that wants it on: every other caller is a ui test, and a
    /// test that opened a `.rs` pane would start the rust-analyzer the machine running the
    /// tests really has and then sit out a cold index of the fixture repo. A test that is
    /// about this wiring turns it on for its own pane, on a file nothing serves - see
    /// `crate::native::file_pane::FileEditor::asks_language_servers_for_test`.
    pub(crate) asks_language_servers: bool,
    /// The keyboard, read through the binding table. It holds the state of a prefix chord
    /// that has begun - the `C-x` of `C-x o` - between frames.
    keymap: Keymap,
    /// The macOS menu bar, if this platform has one.
    #[cfg(not(target_arch = "wasm32"))]
    menu: Option<crate::native::menu::NativeMenu>,
    /// Parsed diffs, keyed by hunk. Word diffing a hunk is quadratic in its line lengths,
    /// which a file like `Cargo.lock` has thousands of, so it must not happen per frame.
    diffs: HashMap<String, CachedDiff>,
    /// Until when this frame may go on reading hunks as code - see
    /// [`CODE_READING_PER_FRAME`].
    code_reading_until: Instant,
    /// Whether a hunk was drawn this frame with its lines left plain for want of time, so
    /// another frame has to follow to read it.
    hunks_left_unread: bool,
    /// What each hunk card measured the last time it was drawn, so the diff pane can skip the
    /// ones that are scrolled out of sight instead of laying them out again.
    pub(crate) hunk_heights: HashMap<String, f32>,
    /// Image diffs, decoded from the `data:` URIs they arrive as and keyed by a hash of the
    /// URI. `None` marks one that could not be read, so it is not retried every frame.
    pub(crate) decoded_images: HashMap<u64, Option<(&'static str, Arc<[u8]>)>>,
    /// Set whenever the palette has to be pushed into the context it is drawing into, which
    /// is the first frame and every theme switch.
    needs_style: bool,
    /// Frames still to repeat the window-theme command over - see
    /// [`theme::apply_window_theme`] for why once is not enough.
    window_theme_frames: u8,
    /// Whether the context being drawn into has egui's image loaders. Installing them twice
    /// would stack a second copy of each, so this is set once and never cleared.
    loaders_installed: bool,
    /// The same, for the bold and italic faces code is set in and the system fonts a
    /// shell's output needs to draw its boxes and spinners.
    fonts_installed: bool,
    /// Whether the workspace has ever held a pane. An empty one means the last tab was
    /// closed and the window is done; before the first review opens it means nothing yet.
    had_panes: bool,
    /// Which of the three executables this is, and so what the window opens on.
    frame: crate::cli::Frame,
    /// What the title bar was last told to say, so it is only told again when it changes.
    window_title: String,
    /// Until when a quit that was warned about goes through unasked.
    quit_armed_until: Option<Instant>,
    // `moon open`'s, down to `window_is_in_front` - see `open_from_shell`. A browser has no
    // shell to type it in.
    /// The socket a `moon open` typed in a shell reaches this window on, and the record
    /// saying which project it is on. `None` in a ui test, which listens for nothing - see
    /// [`App::listen_for_shell_asks`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) shell_asks: Option<crate::instances::window::ShellAsks>,
    /// The project that record was last written with, so it is rewritten when the window
    /// opens another project and not on every frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) project_asks_reach_this_window_on: Option<std::path::PathBuf>,
    /// Files shells have asked for, waiting their turn: opening a tab goes through the one
    /// deferred slot every other pane change does, so they are opened one to a frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) asked_files: std::collections::VecDeque<crate::instances::window::OpenFileAsked>,
    /// The tabs `moon edit --wait` asks opened, which the shell that asked is waiting on -
    /// see [`crate::native::open_from_shell::WaitedTab`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) waited_tabs: Vec<crate::native::open_from_shell::WaitedTab>,
    /// The sessions this window opened so it could take files of projects it is not itself
    /// on - see [`crate::native::open_from_shell`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) sessions_for_asked_files: crate::native::open_from_shell::SessionsForAskedFiles,
    /// Whether the window was in front on the last frame, so the frame it comes forward on
    /// is the one that writes that down for the shells to read.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) window_is_in_front: bool,
    /// The native webviews laid over webview panes - see [`crate::native::webview_pane`].
    pub(crate) webviews: crate::native::webview_pane::Webviews,
}

struct CachedDiff {
    /// Hash of the patch text the lines were built from. The poll loop hands back a fresh
    /// payload every second, and almost always an identical one.
    patch_hash: u64,
    lines: Arc<Vec<DiffLine>>,
    /// The character count of the longest body among the lines, which is how far the hunk
    /// can be scrolled sideways. Counted once here rather than walked on every frame.
    widest_body_columns: usize,
    /// Whether the lines carry their syntax, or were built plain because the frame had spent
    /// its [`CODE_READING_PER_FRAME`] already.
    read_as_code: bool,
}

/// How long one frame may spend reading hunks as code before it draws the rest plain and
/// leaves them to the frames after it.
///
/// The first frame of a review has no card heights to skip the off-screen cards by, so it
/// builds the lines of every hunk in the diff. Reading them all as code in that one frame is
/// a second or so natively, and seven in a browser, where the grammar's backtracking regexes
/// run in wasm - with the page frozen the whole time, since the browser's thread is the one
/// drawing. Spread over frames, a card shows plain for a frame or two and takes its colours
/// as soon as it comes up; the cards out of sight are skipped once measured and are only read
/// when scrolled to.
const CODE_READING_PER_FRAME: Duration = Duration::from_millis(8);

impl App {
    /// Built from a bare [`egui::Context`] rather than an `eframe::CreationContext`, so the
    /// UI tests can drive the real window contents without a real window.
    pub(crate) fn new(ctx: egui::Context, launch: Launch) -> Self {
        let theme = if ctx.theme() == egui::Theme::Dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };

        let tasks = Tasks::new(Arc::clone(&launch.backend), ctx);
        let connection = launch.backend.describe();

        let stage = match &launch.open {
            Some(_) => Stage::Opening,
            None => Stage::Prompt {
                repo_path: String::new(),
                error: None,
            },
        };

        let mut app = Self {
            model: Model {
                stage,
                theme,
                // Marked once the project is known, which the review opening is what says.
                workspace_color: WorkspaceColor::default(),
                layout: Layout::new(),
                // Until the workspace is first drawn and measured.
                columns_fit: true,
                root_session_id: String::new(),
                last_shell_session_id: None,
                reviews: HashMap::new(),
                root_repo_status: None,
                submodules: Vec::new(),
                project: Default::default(),
                project_pending: false,
                project_focus: false,
                project_editor: None,
                project_unsaved: false,
                restart_on_shell_exit: None,
                project_shell: None,
                submodule_filter: String::new(),
                review_requests: Vec::new(),
                                review_request_amendments: 0,
                submodule_filter_focus: false,
                shells_running_a_command: Vec::new(),
                shells_wanting_attention: Vec::new(),
                attention_posted: HashMap::new(),
                toasts: Vec::new(),
                messages: Default::default(),
                language_servers_working: HashMap::new(),
                palette: Default::default(),
                renaming: None,
                places_found: None,
                code_acting: None,
                board: Default::default(),
                agent_log: None,
                connection,
                commit_panes: HashMap::new(),
                file_editors: HashMap::new(),
                work_log_entries_waiting: HashMap::new(),
                #[cfg(not(target_arch = "wasm32"))]
                extension_panes: HashMap::new(),
                markdown_cache: Default::default(),
                find: None,
                terminal_with_keyboard: None,
                terminal_names: HashMap::new(),
                renaming_tab: None,
                opened_project: None,
                project_path: None,
                adopt_shells_pending: false,
                open_shell_pending: false,
                restored_layout: None,
                visualizations: Default::default(),
                // The agent the person last picked, put back once the settings have come
                // and the review says this machine still has it - see `App::load_settings`.
                restored_agent: None,
                settings: None,
            },
            tasks,
            terminals: HashMap::new(),
            commit_terminals: HashMap::new(),
            frames: Frames::new(),
            attaching: Arc::new(Mutex::new(Vec::new())),
            terminal_errors: HashMap::new(),
            // Backdated so the first frame fetches instead of waiting out an interval.
            last_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            last_submodules_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            last_board_poll: Instant::now()
                .checked_sub(BOARD_POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            last_running_shells_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
                        last_review_requests_poll: Instant::now()
                .checked_sub(BOARD_POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            // Backdated like the rest, so the first pane that has a server behind it is asked
            // about straight away rather than after the first interval.
            last_lsp_work_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            pending_action: None,
            pending_close: None,
            pending_close_of_others: None,
            pending_tab_action: None,
            tab_shortcuts: HashMap::new(),
            keyboard_pane: None,
            front_pane: None,
            pane_taking_keyboard: None,
            #[cfg(not(target_arch = "wasm32"))]
            menu: None,
            diffs: HashMap::new(),
            code_reading_until: Instant::now(),
            hunks_left_unread: false,
            hunk_heights: HashMap::new(),
            decoded_images: HashMap::new(),
            // Turned on by the window itself - see the field.
            asks_language_servers: false,
            keymap: Keymap::default(),
            needs_style: true,
            window_theme_frames: WINDOW_THEME_FRAMES,
            loaders_installed: false,
            fonts_installed: false,
            had_panes: false,
            frame: launch.frame,
            // What `run` opened the window with, so the first frame has nothing to say.
            window_title: window_title(launch.frame, None),
            quit_armed_until: None,
            // Started by the window itself - see the field.
            #[cfg(not(target_arch = "wasm32"))]
            shell_asks: None,
            #[cfg(not(target_arch = "wasm32"))]
            project_asks_reach_this_window_on: None,
            #[cfg(not(target_arch = "wasm32"))]
            asked_files: std::collections::VecDeque::new(),
            #[cfg(not(target_arch = "wasm32"))]
            waited_tabs: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            sessions_for_asked_files: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(not(target_arch = "wasm32"))]
            window_is_in_front: false,
            webviews: Default::default(),
        };

        app.load_settings();
        if let Some(open) = launch.open {
            app.open_review(open);
        }
        app
    }

    /// Put up the application menu. Only [`crate::native::run`] calls this: on macOS the
    /// menu bar may only be built on the main thread, which is where the window lives but is
    /// not where a test runs.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn install_menu(&mut self) {
        let picks_files = self.backend().reads_this_machine();
        self.menu = crate::native::menu::NativeMenu::install(picks_files, self.frame);
    }

    pub(crate) fn backend(&self) -> &Arc<dyn Backend> {
        self.tasks.backend()
    }

    /// Which of the three programs this window is.
    pub(crate) fn frame(&self) -> crate::cli::Frame {
        self.frame
    }

    pub(crate) fn palette_of(&self) -> Palette {
        Palette::of_workspace(self.model.theme, self.model.workspace_color)
    }

    pub(crate) fn set_theme(&mut self, theme: ThemeMode) {
        self.model.theme = theme;
        self.needs_style = true;
    }

    /// The parsed lines of a hunk's patch, built once per distinct patch text.
    ///
    /// Reading the code is part of building them rather than something the painter does:
    /// syntax is worked out once per patch here and thrown away with it, where per row or per
    /// frame it would be a grammar run for every line on screen, every frame. Once the frame
    /// has spent its [`CODE_READING_PER_FRAME`], lines come back plain, and are built again
    /// with their syntax by the first frame that asks for them with time left.
    pub(crate) fn diff_lines(
        &mut self,
        hunk_id: &str,
        patch: &str,
        file_path: &str,
    ) -> Arc<Vec<DiffLine>> {
        let patch_hash = hash_of(patch);
        let has_time_to_read = Instant::now() < self.code_reading_until;
        if let Some(cached) = self.diffs.get(hunk_id)
            && cached.patch_hash == patch_hash
            && (cached.read_as_code || !has_time_to_read)
        {
            self.hunks_left_unread |= !cached.read_as_code;
            return Arc::clone(&cached.lines);
        }

        let mut lines = build_diff_lines(patch);
        if has_time_to_read {
            attach_syntax(&mut lines, file_path);
        } else {
            self.hunks_left_unread = true;
        }
        let widest_body_columns = lines
            .iter()
            .filter(|line| !line.is_chrome())
            .map(|line| line.body().chars().count())
            .max()
            .unwrap_or(0);
        let lines = Arc::new(lines);
        self.diffs.insert(
            hunk_id.to_string(),
            CachedDiff {
                patch_hash,
                lines: Arc::clone(&lines),
                widest_body_columns,
                read_as_code: has_time_to_read,
            },
        );
        lines
    }

    /// Give the frame about to draw the panes its time for reading hunks as code - see
    /// [`CODE_READING_PER_FRAME`].
    pub(crate) fn start_reading_hunks(&mut self) {
        self.code_reading_until = Instant::now() + CODE_READING_PER_FRAME;
        self.hunks_left_unread = false;
    }

    /// Whether the panes just drawn left a hunk plain, so a frame has to follow to read it
    /// even with nothing else moving.
    pub(crate) fn hunks_left_unread(&self) -> bool {
        self.hunks_left_unread
    }

    /// The character count of the longest line [`diff_lines`](Self::diff_lines) last built
    /// for a hunk. Asked for after the lines themselves, which is what put it there.
    pub(crate) fn widest_diff_body(&self, hunk_id: &str) -> usize {
        self.diffs
            .get(hunk_id)
            .expect("the hunk's lines are built before their width is asked for")
            .widest_body_columns
    }

    /// Drop cached diffs for hunks the review no longer has, so switching commits in a big
    /// repo does not leave every previous diff in memory.
    fn prune_diff_cache(&mut self) {
        if self.diffs.len() < 4096 {
            return;
        }
        let live: std::collections::HashSet<String> = self
            .model
            .reviews
            .values()
            .flat_map(|review| review.hunks().iter().map(|hunk| hunk.id.clone()))
            .collect();
        self.diffs.retain(|hunk_id, _| live.contains(hunk_id));
        self.hunk_heights
            .retain(|hunk_id, _| live.contains(hunk_id));
    }
}

/// Where the pane arrangement is kept between runs. Which agent comments go to is not here:
/// that belongs to the person rather than to the window, so it lives in
/// [`crate::settings`] - one file, in the server's home directory, that they can read and edit.
const LAYOUT_STORAGE_KEY: &str = "moonreview-workspace-layout";

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.draw(ui);
        // Here rather than in `draw`: a webview is a child of the window, whose handle only
        // this is handed, and the ui tests draw without one.
        #[cfg(target_os = "macos")]
        self.place_webviews(frame, ui.ctx());
        #[cfg(not(target_os = "macos"))]
        let _ = frame;
    }

    /// eframe's default clear color is a dark gray whatever the theme, and the workspace
    /// leaves a sliver of it showing above the first frame - a dark bar in light mode. The
    /// app's own palette is used rather than the visuals handed in, which lag behind it on
    /// the first frames.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::Palette::of_workspace(self.model.theme, self.model.workspace_color)
            .bg
            .to_normalized_gamma_f32()
    }

    /// What dictation and the emoji picker typed since the last frame, as if typed on the
    /// keyboard - see [`super::text_without_a_key`].
    #[cfg(target_os = "macos")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input
            .events
            .extend(super::text_without_a_key::take().into_iter().map(egui::Event::Text));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(encoded) = serde_json::to_string(&self.model.layout) {
            storage.set_string(LAYOUT_STORAGE_KEY, encoded);
        }
    }
}

impl App {
    /// Take the arrangement the last run stored, to be applied once a review is open.
    ///
    /// A malformed or outdated value is simply ignored: a window that opens on the default
    /// arrangement is a far better outcome than one that refuses to open.
    pub(crate) fn restore_layout_from(&mut self, storage: Option<&dyn eframe::Storage>) {
        let Some(encoded) = storage.and_then(|storage| storage.get_string(LAYOUT_STORAGE_KEY))
        else {
            return;
        };
        match serde_json::from_str::<Layout<Pane>>(&encoded) {
            Ok(stored) => self.model.restored_layout = Some(stored),
            Err(error) => eprintln!("[moonreview] ignoring a stored layout: {error}"),
        }
    }

    /// The agent the review in front is pointed at, which is the one worth remembering - and
    /// the one a new task starts with.
    pub(crate) fn selected_agent(&self) -> AgentKind {
        let session_id = self
            .focused_review_session()
            .unwrap_or_else(|| self.model.root_session_id.clone());
        self.model
            .review_ref(&session_id)
            .and_then(|review| review.payload.as_ref())
            .map(|payload| payload.selected_agent)
            .unwrap_or_default()
    }


    /// Quitting kills every shell the window holds, along with whatever they were in the
    /// middle of, so the first ⌘Q says so and the second one goes through.
    ///
    /// A shell sitting at its prompt has nothing to interrupt, and asking about it would make
    /// the warning something to click through rather than something to read: what counts is a
    /// shell with a command running in it - see `App::shells_running_a_command`.
    ///
    /// Closing the last shell's tab is not this: it ends that shell deliberately, and the
    /// window that follows it out has nothing left running to warn about.
    fn quit_would_kill_shells(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        let running = self.shells_running_a_command();
        if running == 0 {
            return false;
        }
        // Armed by the warning, and only for as long as the warning is still on screen: a ⌘Q
        // an hour later is as much of a surprise as the first one was.
        if self
            .quit_armed_until
            .is_some_and(|until| Instant::now() < until)
        {
            return false;
        }

        self.quit_armed_until = Some(Instant::now() + QUIT_CONFIRM_WINDOW);
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.model.error(match running {
            1 => "a shell is still running - quit again to close it".to_string(),
            running => format!("{running} shells are still running - quit again to close them"),
        });
        true
    }

    /// Say which project this window is on in its title bar. Sent only when it changes: a
    /// viewport command is a message to the windowing system, not a thing to repeat 60 times a
    /// second.
    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = window_title(self.frame, self.model.project_path.as_deref());
        if title == self.window_title {
            return;
        }
        self.window_title = title.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }


    /// A session starts on no agent at all, so the one the last run ended on is put back - once
    /// the review has said which agents this machine actually has, since asking for one that is
    /// no longer installed is refused.
    fn apply_restored_agent(&mut self) {
        let Some(agent) = self.model.restored_agent else {
            return;
        };
        let session_id = self.model.root_session_id.clone();
        let Some(payload) = self
            .model
            .review_ref(&session_id)
            .and_then(|review| review.payload.clone())
        else {
            return;
        };

        self.model.restored_agent = None;
        if agent == AgentKind::None || payload.selected_agent == agent {
            return;
        }
        // An agent that has since left the machine is simply forgotten: a window that opens
        // with no agent picked is better than one that opens complaining.
        if !payload
            .available_agents
            .iter()
            .any(|option| option.kind == agent && option.available)
        {
            return;
        }

        let for_call = session_id.clone();
        self.tasks
            .act(&session_id, "could not restore the agent", move |backend| {
                backend.set_agent(&for_call, agent)
            });
    }
}
