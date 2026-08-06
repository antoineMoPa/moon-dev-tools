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
        theme::{self, Palette, ThemeMode},
        widgets,
        workspace::SHELL_REPAINT_INTERVAL,
    },
};

/// How often an open review refetches, so staging from a shell or another window shows up.
const POLL_INTERVAL: Duration = Duration::from_millis(900);
/// The same, for a window that is not focused.
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(5);
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
        self.menu = NativeMenu::install(self.serves_web);
    }

    pub(crate) fn backend(&self) -> &Arc<dyn Backend> {
        self.tasks.backend()
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
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.28);
                ui.label(RichText::new("🌚 moonreview").size(22.0).strong());
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("connected to {}", self.model.connection))
                        .color(palette.muted),
                );
                ui.add_space(18.0);

                let Stage::Prompt { repo_path, error } = &mut self.model.stage else {
                    return;
                };
                ui.label(RichText::new("Path of the repo to review, on that machine:").color(palette.muted));
                ui.add_space(6.0);
                let entry = ui.add_sized(
                    vec2(460.0, 24.0),
                    egui::TextEdit::singleline(repo_path).hint_text("/home/you/project"),
                );
                let submitted = entry.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                let path = repo_path.trim().to_string();

                if let Some(error) = error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(error.clone()).color(palette.warn));
                }

                ui.add_space(12.0);
                let go = widgets::clickable(
                    ui.add_enabled(!path.is_empty(), egui::Button::new("Open review")),
                )
                .clicked();

                if (go || submitted) && !path.is_empty() {
                    self.model.stage = Stage::Opening;
                    self.open_review(OpenSessionRequest {
                        repo_path: path,
                        diff_target: None,
                        active_commit: None,
                    });
                }
            });
        });
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

    /// The agent the review in front is pointed at, which is the one worth remembering.
    fn selected_agent(&self) -> AgentKind {
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
