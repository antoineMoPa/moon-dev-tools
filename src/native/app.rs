//! The window: what it is showing, what the keyboard does, and how work gets started.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use egui::{Align, Align2, CornerRadius, Key, Layout as UiLayout, RichText, Ui, vec2};
use egui_frames::{Frames, Layout, PaneId};

use crate::{
    api::{AgentKind, OpenSessionRequest},
    backend::Backend,
    native::{
        Launch,
        bindings::{self, Action, Keymap},
        board, find, fonts,
        menu::{MenuAction, NativeMenu},
        model::{Model, Stage, ToastKind, hash_of},
        palette::{self, CommandAction},
        panes::{OpenPaneRequest, Pane, PaneKind},
        review::diff::{DiffLine, build_diff_lines},
        tasks::Tasks,
        theme::{self, Palette, SMALL_SIZE, ThemeMode},
        widgets,
        workspace::SHELL_REPAINT_INTERVAL,
    },
};

/// How often an open review refetches, so staging from a shell or another window shows up.
const POLL_INTERVAL: Duration = Duration::from_millis(900);
/// The same, for a window that is not focused.
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How long a warned-about quit stays armed. The warning is a toast, so this is how long that
/// toast is up: pressing again while it can still be read is the second press it asks for.
const QUIT_CONFIRM_WINDOW: Duration =
    Duration::from_millis((crate::native::model::TOAST_LIFETIME * 1000.0) as u64);

/// How often an open moontasks board rereads `.moontasks`. Slower than a review: reading it
/// is a directory walk, and a card moves at the pace an agent works rather than a keystroke.
const BOARD_POLL_INTERVAL: Duration = Duration::from_millis(1500);

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
    /// Whether it should take the keyboard once drawn: set for a shell the user just opened,
    /// not for one being reattached because a restored arrangement mentions it.
    pub(crate) focus: bool,
}

pub(crate) struct App {
    pub(crate) model: Model,
    pub(crate) tasks: Tasks,
    pub(crate) terminals: HashMap<String, egui_tty::Terminal>,
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
    /// The keyboard, read through the binding table. It holds the state of a prefix chord
    /// that has begun — the `C-x` of `C-x o` — between frames.
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
                reviews: HashMap::new(),
                submodules: Vec::new(),
                toasts: Vec::new(),
                palette: Default::default(),
                board: Default::default(),
                agent_log: None,
                connection,
                file_editors: HashMap::new(),
                find: None,
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
            menu: None,
            diffs: HashMap::new(),
            hunk_heights: HashMap::new(),
            decoded_images: HashMap::new(),
            keymap: Keymap::default(),
            needs_style: true,
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
        self.menu = NativeMenu::install(self.serves_web, self.frame);
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

    fn open_review(&mut self, open: OpenSessionRequest) {
        let frame = self.frame;
        // Only remembered once the review is actually open: a path that turns out not to be a
        // repo has no business on the launch screen's list.
        let repo_path = open.repo_path.clone();
        self.tasks.spawn(
            move |backend| backend.open_session(open),
            move |model, result| match result {
                Ok(opened) => {
                    model.root_session_id = opened.session_id.clone();
                    // A stored arrangement contributes its splits; what goes in it is new.
                    model.layout = crate::native::workspace::arrangement_for(
                        model.restored_layout.take(),
                        &opened.session_id,
                        frame,
                    );
                    model.review(&opened.session_id);
                    model.stage = Stage::Ready;
                    model.opened_project = Some(repo_path.clone());
                    model.project_path = Some(repo_path.clone());
                    model.adopt_shells_pending = true;
                    model.board.refresh_requested = true;
                    // `moonshell` opens on a shell, which has to be started before there is
                    // anything to draw.
                    model.open_shell_pending = frame == crate::cli::Frame::Shell;
                }
                Err(error) => {
                    let message = format!("{error}");
                    model.stage = Stage::Prompt {
                        repo_path: String::new(),
                        error: Some(message.clone()),
                    };
                    model.error(format!("could not open the review: {message}"));
                }
            },
        );
    }

    /// Refetch every open review, on the poll clock or because something changed it.
    ///
    /// Each refetch re-reads the diff from git, which is not free in a large repo, so a
    /// window nobody is looking at checks back far less often.
    fn poll_reviews(&mut self, focused: bool) {
        let interval = if focused {
            POLL_INTERVAL
        } else {
            BACKGROUND_POLL_INTERVAL
        };
        let due = self.last_poll.elapsed() >= interval;
        let session_ids: Vec<String> = self
            .model
            .reviews
            .values()
            .filter(|review| due || review.refresh_requested)
            .map(|review| review.session_id.clone())
            .collect();
        if session_ids.is_empty() {
            return;
        }
        if due {
            self.last_poll = Instant::now();
        }

        for session_id in session_ids {
            self.model.review(&session_id).refresh_requested = false;
            let key = format!("state:{session_id}");
            let for_apply = session_id.clone();
            self.tasks.spawn_keyed(
                Some(key),
                move |backend| backend.session_state(&session_id),
                move |model, result| {
                    let review = model.review(&for_apply);
                    review.loading = false;
                    match result {
                        Ok(payload) => {
                            review.error = None;
                            // An active hunk that the diff no longer contains would leave the
                            // keyboard acting on something the user cannot see.
                            if let Some(active) = review.active_hunk_id.clone()
                                && !payload.hunks.iter().any(|hunk| hunk.id == active)
                            {
                                review.active_hunk_id = None;
                            }
                            review.history_has_more = payload.history_has_more;
                            if review.history_loaded.is_empty() {
                                review.history_loaded = payload.history_commits.clone();
                            }
                            review.payload = Some(Arc::new(payload));
                        }
                        Err(error) => review.error = Some(format!("{error}")),
                    }
                },
            );
        }
    }

    /// Refetch the moontasks board while a pane is showing it.
    ///
    /// The board is a folder anything may write to — an agent moving its own card, a second
    /// window, a text editor — so the only way to know what is on it is to read it again.
    fn poll_board(&mut self) {
        if self.model.root_session_id.is_empty() || !board::is_open(self) {
            return;
        }
        let due = self.last_board_poll.elapsed() >= BOARD_POLL_INTERVAL;
        if !due && !self.model.board.refresh_requested {
            return;
        }
        if due {
            self.last_board_poll = Instant::now();
        }
        self.model.board.refresh_requested = false;

        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn_keyed(
            Some("tasks".to_string()),
            move |backend| backend.list_tasks(&session_id),
            |model, result| {
                model.board.loaded = true;
                match result {
                    Ok(tasks) => {
                        model.board.error = None;
                        model.board.tasks = tasks;
                    }
                    Err(error) => model.board.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Open a tab on a shell the board just started.
    fn open_shell_the_board_started(&mut self) {
        let Some(opened) = self.model.board.opened_shell.take() else {
            return;
        };
        self.open_pane(OpenPaneRequest::AttachTerminal {
            terminal_id: opened.terminal_id,
            command: opened.command,
            task_id: Some(opened.task_id),
        });
    }

    fn poll_submodules(&mut self) {
        if self.model.root_session_id.is_empty() {
            return;
        }
        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn_keyed(
            Some("submodules".to_string()),
            move |backend| backend.session_submodules(&session_id),
            |model, result| {
                if let Ok(submodules) = result {
                    model.submodules = submodules;
                }
            },
        );
    }

    /// Run whatever the palette or the menu bar asked for.
    pub(crate) fn run_action(&mut self, action: CommandAction) {
        match action {
            CommandAction::OpenPane(request) => self.open_pane(request),
            CommandAction::ToggleTheme => self.set_theme(self.model.theme.toggled()),
            CommandAction::OpenInBrowser => self.open_in_browser(),
            CommandAction::InstallLaunchers => self.install_launchers(),
            CommandAction::NewWindow(frame) => self.open_new_window(frame),
        }
    }

    /// Open another window — of this program or of one of its siblings — on the same repo.
    ///
    /// A window is a process here: each one carries its own review server and its own shells,
    /// so there is nothing to open a second window out of but a second run of the executable.
    /// It is left to run on its own; closing this one does not take it with it.
    fn open_new_window(&mut self, frame: crate::cli::Frame) {
        let Some(executable) = crate::native::programs::executable_for(frame) else {
            self.model.error(format!(
                "{} is not installed beside this one",
                frame.program()
            ));
            return;
        };
        let Some(repo_path) = self.model.project_path.clone() else {
            self.model
                .error("this window is not on a project yet, so there is none to open");
            return;
        };

        let target = self.backend().connect_target();
        let spawned = crate::native::programs::new_window_command(
            &executable,
            &repo_path,
            target.as_deref(),
        )
        .spawn();
        if let Err(error) = spawned {
            self.model.error(format!(
                "could not open a {} window: {error}",
                frame.display_name()
            ));
        }
    }

    /// Write the launchers the OS lists, and say what landed where.
    fn install_launchers(&mut self) {
        match crate::native::launchers::install() {
            Ok(installed) => {
                let names: Vec<&str> = installed
                    .iter()
                    .map(|launcher| launcher.frame.display_name())
                    .collect();
                self.model.info(format!(
                    "{} in {}",
                    names.join(", "),
                    crate::native::launchers::destination_hint()
                ));
            }
            Err(error) => self
                .model
                .error(format!("could not write the launchers: {error}")),
        }
    }

    /// Where a file of the review sits on this machine, if the repo is on this machine at all.
    pub(crate) fn repo_file_path(&self, file_path: &str) -> Option<std::path::PathBuf> {
        let session_id = self.model.root_session_id.clone();
        let payload = self.model.review_ref(&session_id)?.payload.as_ref()?;
        Some(std::path::Path::new(&payload.repo_path).join(file_path))
    }

    fn open_in_browser(&mut self) {
        if !self.serves_web {
            self.model
                .error("this window is not serving the web frontend");
            return;
        }
        let url = self.backend().web_url(&self.model.root_session_id);
        if let Err(error) = webbrowser::open(&url) {
            self.model.error(format!("could not open a browser: {error}"));
        }
    }

    /// ⌘W closes the tab in front. Only a workspace with nothing left in it passes the chord
    /// on to the window itself.
    fn close_active_tab(&mut self, ctx: &egui::Context) {
        match self.active_pane_id() {
            Some(pane_id) => self.pending_close = Some(pane_id),
            None => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// A file with edits that are not on disk takes two presses to close, so a stray ⌘W or a
    /// mis-aimed click cannot throw work away.
    fn close_would_lose_edits(&mut self, pane_id: PaneId) -> bool {
        if !self.file_pane_is_dirty(pane_id) {
            return false;
        }
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return false;
        };
        if editor.close_confirmed {
            return false;
        }
        editor.close_confirmed = true;
        let file_path = editor.file_path.clone();
        self.model
            .error(format!("{file_path} has unsaved edits — close again to discard them"));
        true
    }

    /// Read this frame's keyboard through the binding table and act on what it fired.
    fn apply_shortcuts(&mut self, ctx: &egui::Context) {
        // A shell gets every plain keystroke — `s` there is the letter s — and so does a text
        // box. Only the chords marked as reaching anywhere are the window's while either has
        // the keyboard. The palette is the exception: it is the window's own text box.
        let typing = (ctx.egui_wants_keyboard_input() && !self.model.palette.open)
            || self.active_pane_kind() == Some(PaneKind::Terminal);

        for action in self.keymap.resolve(ctx, typing) {
            self.apply_action(action, ctx);
        }
    }

    fn apply_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::OpenPalette => {
                self.model.palette.open = true;
                self.model.palette.query.clear();
                self.model.palette.highlighted = 0;
            }
            Action::NewShellTab => self.pending_tab_action = Some(TabAction::New),
            Action::CloseTab => self.pending_tab_action = Some(TabAction::Close),
            // Deferred into the same slot the menu bar's item uses: on macOS the chord can
            // arrive as both, and two windows is not what one ⌘N asked for.
            Action::NewWindow => self.pending_action = Some(CommandAction::NewWindow(self.frame)),
            Action::SaveFile => {
                if let Some((pane_id, Pane::File { session_id, .. })) = self.active_pane() {
                    let session_id = session_id.clone();
                    self.save_file_pane(pane_id, &session_id);
                }
            }
            Action::ToggleTheme => self.set_theme(self.model.theme.toggled()),
            Action::AdvanceHunk => self.apply_hunk_shortcut(true),
            Action::ReverseHunk => self.apply_hunk_shortcut(false),
            Action::FocusNextFrame => self.focus_next_frame(ctx),
            Action::Find => find::open(self),
        }
    }

    /// `s` and `u` act on the hunk the review pane has under the caret: stage and unstage in
    /// a working-tree review, mark reviewed and unreviewed in a read-only one.
    fn apply_hunk_shortcut(&mut self, forward: bool) {
        let Some(session_id) = self.focused_review_session() else {
            return;
        };
        let Some(review) = self.model.review_ref(&session_id) else {
            return;
        };
        let Some(active) = review.active_hunk_id.clone() else {
            return;
        };
        let Some(hunk) = review.hunks().iter().find(|hunk| hunk.id == active).cloned() else {
            return;
        };
        let read_only = review.read_only();

        if read_only {
            if hunk.reviewed == forward {
                return;
            }
            let hunk_id = hunk.id.clone();
            let for_call = session_id.clone();
            self.tasks
                .act(&session_id, "could not mark the hunk", move |backend| {
                    backend.set_reviewed(&for_call, &hunk_id, Some(forward))
                });
            return;
        }

        if hunk.staged == forward {
            return;
        }
        let hunk_id = hunk.id.clone();
        let for_call = session_id.clone();
        if forward {
            self.tasks
                .act(&session_id, "could not stage the hunk", move |backend| {
                    backend.stage_hunk(&for_call, &hunk_id)
                });
        } else {
            self.tasks
                .act(&session_id, "could not unstage the hunk", move |backend| {
                    backend.unstage_hunk(&for_call, &hunk_id)
                });
        }
    }

    fn draw_prompt(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        // A repo on this machine can be pointed at; one on the far side of a remote connection
        // can only be typed out, since this machine cannot browse for it.
        let picks_folders = self.backend().reads_this_machine();
        let mut open_path = None;
        let mut pick_folder = false;

        let frame = self.frame;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.28);
                ui.label(
                    RichText::new(format!("🌚 {}", frame.program()))
                        .size(22.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("connected to {}", self.model.connection))
                        .color(palette.muted),
                );
                ui.add_space(18.0);

                let Stage::Prompt { repo_path, error } = &mut self.model.stage else {
                    return;
                };
                ui.label(RichText::new(frame.asks_for_repo(picks_folders)).color(palette.muted));
                ui.add_space(6.0);

                // Browsing for the repo is the whole of it on this machine, so there is
                // nothing to type; a remote repo cannot be browsed for and has to be.
                let typed = (!picks_folders).then(|| {
                    let entry = ui.add_sized(
                        vec2(460.0, 24.0),
                        egui::TextEdit::singleline(repo_path).hint_text("/home/you/project"),
                    );
                    let submitted =
                        entry.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                    (repo_path.trim().to_string(), submitted)
                });

                if let Some(error) = error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(error.clone()).color(palette.warn));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    const BUTTON: egui::Vec2 = egui::vec2(120.0, 24.0);
                    ui.add_space((ui.available_width() - BUTTON.x).max(0.0) / 2.0);

                    match &typed {
                        None => {
                            pick_folder = widgets::clickable(
                                ui.add(egui::Button::new("Choose a repo…").min_size(BUTTON)),
                            )
                            .clicked();
                        }
                        Some((path, submitted)) => {
                            let go = widgets::clickable(ui.add_enabled(
                                !path.is_empty(),
                                egui::Button::new(frame.opens_button()).min_size(BUTTON),
                            ))
                            .clicked();
                            if (go || *submitted) && !path.is_empty() {
                                open_path = Some(path.clone());
                            }
                        }
                    }
                });

                if let Some(recent) = draw_recent_projects(ui, &self.settings, &palette) {
                    open_path = Some(recent);
                }
            });
        });

        // Both deferred: the dialog blocks, and opening a review takes `self`.
        if pick_folder
            && let Some(picked) = self.pick_repo_folder(&ui.ctx().clone())
        {
            open_path = Some(picked);
        }
        if let Some(repo_path) = open_path {
            self.model.stage = Stage::Opening;
            self.open_review(OpenSessionRequest {
                repo_path,
                diff_target: None,
                active_commit: None,
            });
        }
    }

    /// The OS folder picker, opened where the last project was found so the next one is
    /// usually a sibling, or on the home directory. What it comes back with is what opens.
    fn pick_repo_folder(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a repo");
        if let Some(recent) = self.settings.recent_projects.first() {
            let beside = std::path::Path::new(recent).parent().unwrap_or_else(|| {
                // A project at the filesystem root has no parent to open beside it.
                std::path::Path::new(recent)
            });
            if beside.is_dir() {
                dialog = dialog.set_directory(beside);
            }
        }

        let picked = dialog.pick_folder()?;
        // The window loses the keyboard to the dialog, and egui only learns that it is back
        // once something else asks it to draw.
        ctx.request_repaint();
        Some(picked.display().to_string())
    }

    fn draw_opening(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.4);
                ui.label(RichText::new("🌚 moonreview").size(20.0).strong());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 140.0).max(0.0) / 2.0);
                    ui.spinner();
                    ui.label(RichText::new("opening the review…").color(palette.muted));
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    /// Run the find bar's search over a review, and tell the bar what it found.
    ///
    /// A search walks every hunk, so it only happens when the bar asks — a changed query or a
    /// step to another match — rather than on every frame the review draws.
    pub(crate) fn apply_review_find(&mut self, pane_id: PaneId, session_id: &str) {
        let Some((query, at, pending)) = self
            .model
            .find
            .as_ref()
            .filter(|find| find.pane_id == pane_id)
            .map(|find| (find.query.clone(), find.at, find.pending))
        else {
            // No bar on this pane: nothing of a previous search stays marked.
            let review = self.model.review(session_id);
            review.find_query.clear();
            review.find_match = None;
            return;
        };

        self.model.review(session_id).find_query = query.clone();
        if !pending {
            return;
        }

        let found = crate::native::review::search::find_all(self, session_id, &query);
        let current = found.get(at).cloned();
        let review = self.model.review(session_id);
        // Bringing the hunk into view is what makes a match in a file scrolled far away
        // findable; the mark on the line itself says where in the hunk it is.
        review.scroll_to_hunk = current.as_ref().map(|found| found.hunk_id.clone());
        review.find_match = current;
        if let Some(find) = &mut self.model.find {
            find.found(found.len());
        }
    }

    /// A chord that has begun but not finished says so in the corner, the way emacs echoes
    /// `C-x-`: otherwise a half-typed prefix silently swallows the next key.
    fn draw_armed_prefix(&mut self, ctx: &egui::Context) {
        let Some(prefix) = self.keymap.armed_prefix() else {
            return;
        };
        let text = format!("{}-", bindings::describe(prefix));
        let palette = self.palette_of();

        egui::Area::new("moonreview-armed-prefix".into())
            .anchor(Align2::LEFT_BOTTOM, vec2(14.0, -14.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(palette.panel)
                    .stroke(egui::Stroke::new(1.0, palette.line))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).monospace().color(palette.accent));
                    });
            });
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        if self.model.toasts.is_empty() {
            return;
        }
        let palette = self.palette_of();
        let screen = ctx.viewport_rect();

        egui::Area::new("moonreview-toasts".into())
            .anchor(Align2::RIGHT_BOTTOM, vec2(-14.0, -14.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width((screen.width() * 0.4).min(420.0));
                let mut dismissed = None;
                for (index, toast) in self.model.toasts.iter().enumerate() {
                    // The stripe down the left is what distinguishes a failure from a note.
                    let ink = match toast.kind {
                        ToastKind::Info => palette.accent_2,
                        ToastKind::Error => palette.warn,
                    };
                    egui::Frame::new()
                        .fill(palette.panel)
                        .stroke(egui::Stroke::new(1.0, palette.line))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(3.0, 15.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, CornerRadius::same(2), ink);
                                ui.label(RichText::new(&toast.text).color(palette.ink));
                                ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                                    if widgets::quiet_button(ui, "\u{1F5D9}").clicked() {
                                        dismissed = Some(index);
                                    }
                                });
                            });
                        })
                        .response
                        .on_hover_text(&toast.text);
                    ui.add_space(5.0);
                }
                if let Some(index) = dismissed {
                    self.model.toasts.remove(index);
                }
            });

        ctx.request_repaint_after(Duration::from_millis(120));
    }

}


/// Where the pane arrangement is kept between runs. Which agent comments go to is not here:
/// that belongs to the person rather than to the window, so it lives in
/// [`crate::settings`] — one file, in their home directory, that they can read and edit.
const LAYOUT_STORAGE_KEY: &str = "moonreview-workspace-layout";

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
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

    /// The agent the review in front is pointed at, which is the one worth remembering — and
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
    /// selector nobody has touched should leave it exactly as the user last left it — or as
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
            1 => "a shell is still running — quit again to close it".to_string(),
            running => format!("{running} shells are still running — quit again to close them"),
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

    /// A session starts on no agent at all, so the one the last run ended on is put back — once
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

impl App {
    /// One frame of the whole window. Split out of the `eframe::App` impl so the UI tests can
    /// render it without a window or an `eframe::Frame`.
    pub(crate) fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // The palette belongs to whichever context is being drawn into, which is not
        // necessarily the one the app was built with. Image loaders go the same way: without
        // them on this context, every image diff draws as a load error.
        if self.needs_style {
            theme::apply(ctx, self.model.theme);
            self.needs_style = false;
        }
        if !self.loaders_installed {
            egui_extras::install_image_loaders(ctx);
            self.loaders_installed = true;
        }
        if !self.fonts_installed {
            fonts::install(ctx);
            self.fonts_installed = true;
        }
        self.tasks.drain(&mut self.model);
        self.remember_opened_project();
        self.update_window_title(ctx);
        self.drain_attachments();
        self.model
            .tick_toasts(ctx.input(|input| input.stable_dt).min(0.25));

        match self.model.stage {
            Stage::Prompt { .. } => {
                self.draw_prompt(ui);
                self.draw_toasts(ctx);
                return;
            }
            Stage::Opening => {
                self.draw_opening(ui);
                self.draw_toasts(ctx);
                return;
            }
            Stage::Ready => {}
        }

        for action in self.menu.as_ref().map(NativeMenu::drain).unwrap_or_default() {
            self.pending_action = Some(match action {
                MenuAction::OpenInBrowser => CommandAction::OpenInBrowser,
                MenuAction::ToggleTheme => CommandAction::ToggleTheme,
                MenuAction::InstallLaunchers => CommandAction::InstallLaunchers,
                MenuAction::NewWindow(frame) => CommandAction::NewWindow(frame),
                MenuAction::NewTab => {
                    self.pending_tab_action = Some(TabAction::New);
                    continue;
                }
                MenuAction::CloseTab => {
                    self.pending_tab_action = Some(TabAction::Close);
                    continue;
                }
                    MenuAction::OpenCommandPalette => {
                    self.model.palette.open = true;
                    self.model.palette.query.clear();
                    self.model.palette.highlighted = 0;
                    continue;
                }
            });
        }

        self.quit_would_kill_shells(ctx);
        self.apply_shortcuts(ctx);
        match self.pending_tab_action.take() {
            Some(TabAction::New) => self.open_shell_tab(),
            Some(TabAction::Close) => self.close_active_tab(ctx),
            None => {}
        }
        let focused = ctx.input(|input| input.focused);
        self.poll_reviews(focused);
        self.poll_submodules();
        self.poll_board();
        self.open_shell_the_board_started();
        if std::mem::take(&mut self.model.adopt_shells_pending) {
            self.adopt_existing_shells();
        }
        if std::mem::take(&mut self.model.open_shell_pending) {
            let primary = self.model.layout.primary_frame();
            self.spawn_terminal(None, crate::native::workspace::TerminalPlacement::Tab(primary));
        }
        self.apply_restored_agent();
        self.remember_selected_agent();
        self.prune_diff_cache();

        self.draw_workspace(ui);
        palette::draw(self, ctx);
        find::draw(self, ctx);
        self.draw_armed_prefix(ctx);
        self.draw_toasts(ctx);

        // Deferred so a pane is never mutated while the tree that holds it is being drawn.
        if let Some(action) = self.pending_action.take() {
            self.run_action(action);
        }
        if let Some(pane_id) = self.pending_close.take()
            && !self.close_would_lose_edits(pane_id)
        {
            self.close_pane(pane_id);
        }
        self.close_tabs_of_exited_shells();

        // Closing the last tab closes the window: an empty workspace has nothing to show and
        // no way back other than the palette.
        if self.model.layout.is_empty() {
            if self.had_panes {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            self.had_panes = true;
        }

        // Terminals whose shell has gone stop repainting, so nothing spins on a dead pane.
        ctx.request_repaint_after(if self.has_live_shell() {
            SHELL_REPAINT_INTERVAL
        } else {
            POLL_INTERVAL
        });
    }
}

/// How wide the recent projects column is. Wider than the picker button it sits under, so a
/// project's path has room beside its name.
const RECENT_COLUMN_WIDTH: f32 = 260.0;

/// What the window is called: the executable, and the project it is open on once there is
/// one. Several windows on several projects is the ordinary way to work, and the title bar is
/// the only place that says which is which.
///
/// The home directory is written as `~`, which is how a path is read at a glance.
pub(crate) fn window_title(frame: crate::cli::Frame, project: Option<&str>) -> String {
    let Some(project) = project else {
        return format!("🌚 {}", frame.program());
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .filter(|home| !home.as_os_str().is_empty());
    let shortened = home
        .and_then(|home| {
            std::path::Path::new(project)
                .strip_prefix(&home)
                .ok()
                .map(|rest| format!("~/{}", rest.display()))
        })
        .unwrap_or_else(|| project.to_string());

    format!("🌚 {} — {shortened}", frame.program())
}

/// The projects opened before, under the picker on the launch screen. Clicking one opens it,
/// which is the whole point: the common case is going back to what you were on yesterday.
///
/// Each row says the project's own directory name, with the path it sits under beside it, so
/// two checkouts of the same repo can be told apart.
fn draw_recent_projects(
    ui: &mut Ui,
    settings: &crate::settings::Settings,
    palette: &Palette,
) -> Option<String> {
    if settings.recent_projects.is_empty() {
        return None;
    }
    let mut open = None;

    ui.add_space(22.0);
    ui.label(RichText::new("Recent projects").color(palette.muted));
    ui.add_space(6.0);

    for path in &settings.recent_projects {
        let directory = std::path::Path::new(path);
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let parent = directory
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();

        let row = ui.horizontal(|ui| {
            // The rows share a left edge, in a column centred under the picker button.
            ui.add_space((ui.available_width() - RECENT_COLUMN_WIDTH).max(0.0) / 2.0);
            let text_starts_at = ui.cursor().left();
            // Selectable labels take the click for themselves, which would leave the row
            // live only in the slivers above and below the text.
            ui.add(egui::Label::new(RichText::new(&name).strong()).selectable(false));
            ui.add(
                egui::Label::new(
                    RichText::new(widgets::elide_path(&parent, 52))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                )
                .selectable(false),
            );
            text_starts_at
        });

        // The whole row reads as the link, but only from where its text begins: the empty
        // strip that centres the column under the picker is not part of it.
        let mut clickable_area = row.response.rect;
        clickable_area.min.x = row.inner;
        if widgets::clickable(
            ui.interact(clickable_area, row.response.id, egui::Sense::click())
                .on_hover_text(path.as_str()),
        )
        .clicked()
        {
            open = Some(path.clone());
        }
    }
    open
}
