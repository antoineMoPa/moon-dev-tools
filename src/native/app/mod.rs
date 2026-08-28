//! The window itself: what it holds, how it is built, and what it hands to eframe.

mod actions;
mod draw;

pub(crate) use draw::window_title;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use egui_frames::{Frames, Layout, PaneId};

use crate::{
    api::AgentKind,
    backend::Backend,
    native::{
        Launch,
        bindings::Keymap,
        menu::NativeMenu,
        model::{Model, Stage, hash_of},
        palette::CommandAction,
        panes::Pane,
        review::diff::{DiffLine, build_diff_lines},
        tasks::Tasks,
        theme::{self, Palette, ThemeMode},
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
    pub(crate) serves_web: bool,
    last_poll: Instant,
    last_board_poll: Instant,
    /// Deferred so a pane is never added or removed while the tree holding it is drawn.
    pub(crate) pending_action: Option<CommandAction>,
    pub(crate) pending_close: Option<PaneId>,
    pending_tab_action: Option<TabAction>,
    /// The chord that raises each tab within cmd+1..cmd+9's reach - the active frame's tabs -
    /// worked out before the strips are drawn and worn at the right of their titles.
    pub(crate) tab_shortcuts: HashMap<PaneId, String>,
    /// The tab the keyboard was last handed to, so a different one coming to the front -
    /// however it got there - is noticed once rather than every frame it stays there.
    pub(crate) keyboard_pane: Option<PaneId>,
    /// The tab owed the keyboard, waiting for its own draw to take it. A shell and a file
    /// editor can only ask for focus from inside the widget that would hold it.
    pub(crate) pane_taking_keyboard: Option<PaneId>,
    /// The keyboard, read through the binding table. It holds the state of a prefix chord
    /// that has begun - the `C-x` of `C-x o` - between frames.
    keymap: Keymap,
    /// The macOS menu bar, if this platform has one.
    menu: Option<NativeMenu>,
    /// Parsed diffs, keyed by hunk. Word diffing a hunk is quadratic in its line lengths,
    /// which a file like `Cargo.lock` has thousands of, so it must not happen per frame.
    diffs: HashMap<String, CachedDiff>,
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
    /// The same, for the system fonts a shell's output needs to draw its boxes and spinners.
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
    /// What `~/.moonreview/settings.json` said, and what it will be written back as.
    settings: crate::settings::Settings,
}

struct CachedDiff {
    /// Hash of the patch text the lines were built from. The poll loop hands back a fresh
    /// payload every second, and almost always an identical one.
    patch_hash: u64,
    lines: Arc<Vec<DiffLine>>,
}

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
        let settings = crate::settings::load();

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
                layout: Layout::new(),
                root_session_id: String::new(),
                last_shell_session_id: None,
                reviews: HashMap::new(),
                submodules: Vec::new(),
                submodule_filter: String::new(),
                submodule_filter_focus: false,
                toasts: Vec::new(),
                palette: Default::default(),
                board: Default::default(),
                agent_log: None,
                connection,
                commit_panes: HashMap::new(),
                file_editors: HashMap::new(),
                markdown_cache: Default::default(),
                find: None,
                terminal_with_keyboard: None,
                opened_project: None,
                project_path: None,
                adopt_shells_pending: false,
                open_shell_pending: false,
                restored_layout: None,
                // The agent the person last picked, put back once the review says this
                // machine still has it.
                restored_agent: Some(settings.selected_agent),
            },
            tasks,
            terminals: HashMap::new(),
            commit_terminals: HashMap::new(),
            frames: Frames::new(),
            attaching: Arc::new(Mutex::new(Vec::new())),
            terminal_errors: HashMap::new(),
            serves_web: launch.serves_web,
            // Backdated so the first frame fetches instead of waiting out an interval.
            last_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            last_board_poll: Instant::now()
                .checked_sub(BOARD_POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            pending_action: None,
            pending_close: None,
            pending_tab_action: None,
            tab_shortcuts: HashMap::new(),
            keyboard_pane: None,
            pane_taking_keyboard: None,
            menu: None,
            diffs: HashMap::new(),
            hunk_heights: HashMap::new(),
            decoded_images: HashMap::new(),
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
            settings,
        };

        if let Some(open) = launch.open {
            app.open_review(open);
        }
        app
    }

    /// Put up the application menu. Only [`crate::native::run`] calls this: on macOS the
    /// menu bar may only be built on the main thread, which is where the window lives but is
    /// not where a test runs.
    pub(crate) fn install_menu(&mut self) {
        let picks_files = self.backend().reads_this_machine();
        self.menu = NativeMenu::install(self.serves_web, picks_files, self.frame);
    }

    pub(crate) fn backend(&self) -> &Arc<dyn Backend> {
        self.tasks.backend()
    }

    /// Which of the three programs this window is.
    pub(crate) fn frame(&self) -> crate::cli::Frame {
        self.frame
    }

    pub(crate) fn palette_of(&self) -> Palette {
        Palette::of(self.model.theme)
    }

    pub(crate) fn set_theme(&mut self, theme: ThemeMode) {
        self.model.theme = theme;
        self.needs_style = true;
    }

    /// The parsed lines of a hunk's patch, built once per distinct patch text.
    pub(crate) fn diff_lines(&mut self, hunk_id: &str, patch: &str) -> Arc<Vec<DiffLine>> {
        let patch_hash = hash_of(patch);
        if let Some(cached) = self.diffs.get(hunk_id)
            && cached.patch_hash == patch_hash
        {
            return Arc::clone(&cached.lines);
        }

        let lines = Arc::new(build_diff_lines(patch));
        self.diffs.insert(
            hunk_id.to_string(),
            CachedDiff {
                patch_hash,
                lines: Arc::clone(&lines),
            },
        );
        lines
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
        self.hunk_heights.retain(|hunk_id, _| live.contains(hunk_id));
    }
}

/// Where the pane arrangement is kept between runs. Which agent comments go to is not here:
/// that belongs to the person rather than to the window, so it lives in
/// [`crate::settings`] - one file, in their home directory, that they can read and edit.
const LAYOUT_STORAGE_KEY: &str = "moonreview-workspace-layout";

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    /// eframe's default clear color is a dark gray whatever the theme, and the workspace
    /// leaves a sliver of it showing above the first frame - a dark bar in light mode. The
    /// app's own palette is used rather than the visuals handed in, which lag behind it on
    /// the first frames.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::Palette::of(self.model.theme).bg.to_normalized_gamma_f32()
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

    /// Keep `settings.json` in step with the selector at the top of the review.
    ///
    /// Written when the choice changes rather than on a clock: the file is one line, and a
    /// selector nobody has touched should leave it exactly as the user last left it - or as
    /// they last edited it by hand.
    pub(crate) fn remember_selected_agent(&mut self) {
        // Before the restored agent has been put back, the session still reads as `None`, and
        // writing that would throw away the very choice being restored.
        if self.model.restored_agent.is_some() {
            return;
        }
        let selected = self.selected_agent();
        if selected == self.settings.selected_agent {
            return;
        }

        self.settings.selected_agent = selected;
        if let Err(error) = crate::settings::store(&self.settings) {
            // Worth saying once, but not worth a toast every frame: the review still works.
            eprintln!("[moonreview] could not save settings: {error}");
        }
    }

    /// Quitting kills every shell the window holds, along with whatever they were in the
    /// middle of, so the first ⌘Q says so and the second one goes through.
    ///
    /// Closing the last shell's tab is not this: it ends that shell deliberately, and the
    /// window that follows it out has nothing left running to warn about.
    fn quit_would_kill_shells(&mut self, ctx: &egui::Context) -> bool {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        let running = self.running_shells();
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

    /// Write a project that has just opened to the head of the recent list, so the next launch
    /// screen offers it.
    fn remember_opened_project(&mut self) {
        let Some(path) = self.model.opened_project.take() else {
            return;
        };
        if !self.settings.remember_project(&path) {
            return;
        }
        if let Err(error) = crate::settings::store(&self.settings) {
            // Worth saying once, but not worth a toast: the review is open either way.
            eprintln!("[moonreview] could not save settings: {error}");
        }
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
