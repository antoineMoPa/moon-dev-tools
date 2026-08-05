//! The window: what it is showing, what the keyboard does, and how work gets started.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use egui::{Align, Align2, CornerRadius, Key, Layout, RichText, Ui, vec2};

use crate::{
    api::{AgentKind, OpenSessionRequest},
    backend::{Backend, TerminalAttachment},
    native::{
        Launch,
        layout::{self, OpenPaneRequest, Pane, PaneKind, default_layout, make_id},
        menu::{MenuAction, NativeMenu},
        model::{Model, Stage, ToastKind, hash_of},
        palette::{self, CommandAction},
        review,
        review::diff::{DiffLine, build_diff_lines},
        tasks::Tasks,
        terminal::TerminalPane,
        theme::{self, Palette, ThemeMode},
        widgets,
    },
};

/// How often an open review refetches, so staging from a shell or another window shows up.
const POLL_INTERVAL: Duration = Duration::from_millis(900);
/// The same, for a window that is not focused.
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Where a new shell's pane lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalPlacement {
    /// Beside the shells already open, or in a new column down the right if there are none.
    WithOtherShells,
    /// A new full-height column down the right of the workspace.
    RightColumn,
    /// Another tab in this frame, for a workspace with no room left to split.
    Tab(String),
}

/// Terminals attach on a worker thread, because a remote one opens a socket. The emulator
/// itself is `!Send`, so the finished attachment is handed back here to be turned into a
/// pane on the UI thread.
type AttachInbox = Arc<Mutex<Vec<(String, Result<TerminalAttachment>)>>>;

pub(crate) struct App {
    pub(crate) model: Model,
    pub(crate) tasks: Tasks,
    pub(crate) terminals: HashMap<String, TerminalPane>,
    attaching: AttachInbox,
    /// Panes whose terminal could not be attached, so the pane can say so.
    pub(crate) terminal_errors: HashMap<String, String>,
    pub(crate) serves_web: bool,
    last_poll: Instant,
    /// Deferred so a pane is never added or removed while the tree holding it is drawn.
    pub(crate) pending_action: Option<CommandAction>,
    pub(crate) pending_close: Option<String>,
    /// The macOS menu bar, if this platform has one.
    menu: Option<NativeMenu>,
    /// Where each frame and tab was drawn this frame, so a released drag resolves against
    /// what was on screen rather than against geometry derived a second time.
    pub(crate) frame_rects: Vec<(String, egui::Rect)>,
    pub(crate) tab_rects: Vec<(String, String, egui::Rect)>,
    /// Where inside a tab the pointer grabbed it, so the tab drawn under the pointer keeps the
    /// spot it was picked up by instead of snapping its corner to the cursor.
    pub(crate) tab_grab_offset: egui::Vec2,
    /// Parsed diffs, keyed by hunk. Word diffing a hunk is quadratic in its line lengths,
    /// which a file like `Cargo.lock` has thousands of, so it must not happen per frame.
    diffs: HashMap<String, CachedDiff>,
    /// Set whenever the palette has to be pushed into the context it is drawing into, which
    /// is the first frame and every theme switch.
    needs_style: bool,
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
        egui_extras::install_image_loaders(&ctx);

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
                layout: layout::empty_layout(),
                root_session_id: String::new(),
                reviews: HashMap::new(),
                submodules: Vec::new(),
                toasts: Vec::new(),
                palette: Default::default(),
                agent_log: None,
                connection,
                dragging_pane: None,
                adopt_shells_pending: false,
                restored_layout: None,
            },
            tasks,
            terminals: HashMap::new(),
            attaching: Arc::new(Mutex::new(Vec::new())),
            terminal_errors: HashMap::new(),
            serves_web: launch.serves_web,
            // Backdated so the first frame fetches instead of waiting out an interval.
            last_poll: Instant::now()
                .checked_sub(POLL_INTERVAL)
                .unwrap_or_else(Instant::now),
            pending_action: None,
            pending_close: None,
            menu: None,
            frame_rects: Vec::new(),
            tab_rects: Vec::new(),
            tab_grab_offset: egui::Vec2::ZERO,
            diffs: HashMap::new(),
            needs_style: true,
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
    }

    fn open_review(&mut self, open: OpenSessionRequest) {
        self.tasks.spawn(
            move |backend| backend.open_session(open),
            |model, result| match result {
                Ok(opened) => {
                    model.root_session_id = opened.session_id.clone();
                    // A stored arrangement contributes its splits; the review itself is new.
                    model.layout = match model.restored_layout.take() {
                        Some(stored) if stored.is_coherent() => {
                            layout::with_review_pane(layout::shape_only(stored), &opened.session_id)
                        }
                        _ => default_layout(&opened.session_id),
                    };
                    model.review(&opened.session_id);
                    model.stage = Stage::Ready;
                    model.adopt_shells_pending = true;
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

    /// Adopt shells the server is already running that this window has no tab for.
    ///
    /// A remote server outlives any one window, and the embedded one is shared with the web
    /// frontend, so a shell started elsewhere is still a shell this window can show.
    fn adopt_existing_shells(&mut self) {
        let session_id = self.model.root_session_id.clone();
        if session_id.is_empty() {
            return;
        }

        self.tasks.spawn_keyed(
            Some("adopt-shells".to_string()),
            move |backend| backend.list_terminals(&session_id),
            |model, result| {
                let Ok(terminal_ids) = result else {
                    return;
                };
                let known: std::collections::HashSet<String> = model
                    .layout
                    .panes
                    .values()
                    .filter_map(|pane| match pane {
                        Pane::Terminal { terminal_id, .. } => Some(terminal_id.clone()),
                        _ => None,
                    })
                    .collect();

                for terminal_id in terminal_ids {
                    if known.contains(&terminal_id) {
                        continue;
                    }
                    let pane = Pane::Terminal {
                        pane_id: make_id("pane"),
                        terminal_id,
                        command: None,
                    };
                    let layout = std::mem::replace(&mut model.layout, layout::empty_layout());
                    let active = layout.active_frame_id.clone();
                    model.layout = match layout.frame_holding_kind(PaneKind::Terminal, &active) {
                        Some(frame_id) => layout::add_pane(layout, &frame_id, pane, None),
                        None => layout::add_pane_in_right_column(layout, pane),
                    };
                }
            },
        );
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

    /// Open a pane where its kind belongs: reviews with reviews, shells with shells, and a
    /// brand new right-hand column for the first shell.
    pub(crate) fn open_pane(&mut self, request: OpenPaneRequest) {
        let layout = std::mem::replace(&mut self.model.layout, layout::empty_layout());
        let active_frame = layout.active_frame_id.clone();

        match request {
            OpenPaneRequest::Review { session_id, title } => {
                // A review that is already open is brought forward instead of duplicated.
                if let Some(pane) = layout.find_review_pane(&session_id) {
                    let pane_id = pane.pane_id().to_string();
                    self.model.layout = layout::focus_pane(layout, &pane_id);
                    return;
                }
                let frame_id = layout
                    .frame_holding_kind(PaneKind::Review, &active_frame)
                    .unwrap_or_else(|| layout.primary_frame_id());
                self.model.review(&session_id);
                self.model.layout = layout::add_pane(
                    layout,
                    &frame_id,
                    Pane::Review {
                        pane_id: make_id("pane"),
                        session_id,
                        title,
                    },
                    None,
                );
            }
            OpenPaneRequest::ReviewRepo { repo_path, title } => {
                self.model.layout = layout;
                // The session has to exist before a pane can point at it, and creating one
                // runs git in the repo, so the pane appears once that comes back.
                self.tasks.spawn(
                    move |backend| {
                        backend.open_session(OpenSessionRequest {
                            repo_path,
                            diff_target: None,
                            active_commit: None,
                        })
                    },
                    move |model, result| match result {
                        Ok(opened) => {
                            model.review(&opened.session_id);
                            let layout =
                                std::mem::replace(&mut model.layout, layout::empty_layout());
                            let frame_id = layout
                                .frame_holding_kind(PaneKind::Review, &layout.active_frame_id)
                                .unwrap_or_else(|| layout.primary_frame_id());
                            model.layout = layout::add_pane(
                                layout,
                                &frame_id,
                                Pane::Review {
                                    pane_id: make_id("pane"),
                                    session_id: opened.session_id,
                                    title,
                                },
                                None,
                            );
                        }
                        Err(error) => {
                            model.error(format!("could not open that review: {error}"))
                        }
                    },
                );
            }
            OpenPaneRequest::Agents => {
                if let Some(pane) = layout.find_pane_of_kind(PaneKind::Agents) {
                    let pane_id = pane.pane_id().to_string();
                    self.model.layout = layout::focus_pane(layout, &pane_id);
                    return;
                }
                let frame_id = layout
                    .frame_holding_kind(PaneKind::Agents, &active_frame)
                    .unwrap_or_else(|| layout.primary_frame_id());
                self.model.layout = layout::add_pane(
                    layout,
                    &frame_id,
                    Pane::Agents {
                        pane_id: make_id("pane"),
                    },
                    None,
                );
            }
            OpenPaneRequest::Terminal { command } => {
                self.model.layout = layout;
                self.spawn_terminal(command, TerminalPlacement::WithOtherShells);
            }
        }
    }

    /// Start a shell on the reviewed repo and open a pane attached to it.
    pub(crate) fn spawn_terminal(
        &mut self,
        command: Option<AgentKind>,
        placement: TerminalPlacement,
    ) {
        let session_id = self.model.root_session_id.clone();
        if session_id.is_empty() {
            self.model.error("no review is open yet");
            return;
        }
        let inbox = Arc::clone(&self.attaching);

        self.tasks.spawn(
            move |backend| {
                let terminal_id = backend.create_terminal(&session_id, command)?;
                let attachment = backend.attach_terminal(&session_id, &terminal_id);
                Ok((terminal_id, attachment))
            },
            move |model, result| match result {
                Ok((terminal_id, attachment)) => {
                    let pane = Pane::Terminal {
                        pane_id: make_id("pane"),
                        terminal_id: terminal_id.clone(),
                        command,
                    };
                    let layout = std::mem::replace(&mut model.layout, layout::empty_layout());
                    model.layout = match &placement {
                        TerminalPlacement::WithOtherShells => {
                            let active_frame = layout.active_frame_id.clone();
                            match layout.frame_holding_kind(PaneKind::Terminal, &active_frame) {
                                Some(frame_id) => layout::add_pane(layout, &frame_id, pane, None),
                                None => layout::add_pane_in_right_column(layout, pane),
                            }
                        }
                        TerminalPlacement::RightColumn => {
                            layout::add_pane_in_right_column(layout, pane)
                        }
                        // The frame is gone if it was closed while the shell was starting.
                        TerminalPlacement::Tab(frame_id) if layout.frames.contains_key(frame_id) => {
                            layout::add_pane(layout, frame_id, pane, None)
                        }
                        TerminalPlacement::Tab(_) => layout::add_pane_in_right_column(layout, pane),
                    };
                    if let Ok(mut inbox) = inbox.lock() {
                        inbox.push((terminal_id, attachment));
                    }
                }
                Err(error) => model.error(format!("could not start a shell: {error}")),
            },
        );
    }

    /// Reattach a shell whose pane is on screen but whose emulator is not — which happens
    /// when a restored layout mentions a terminal this window has not attached yet.
    pub(crate) fn attach_terminal(&mut self, terminal_id: &str) {
        let key = format!("attach:{terminal_id}");
        if self.tasks.is_busy(&key) || self.terminal_errors.contains_key(terminal_id) {
            return;
        }
        let session_id = self.model.root_session_id.clone();
        let inbox = Arc::clone(&self.attaching);
        let terminal_id = terminal_id.to_string();
        let for_inbox = terminal_id.clone();

        self.tasks.spawn_keyed(
            Some(key),
            move |backend| Ok(backend.attach_terminal(&session_id, &terminal_id)),
            move |_model, result| {
                let attachment = result.and_then(|attachment| attachment);
                if let Ok(mut inbox) = inbox.lock() {
                    inbox.push((for_inbox, attachment));
                }
            },
        );
    }

    fn drain_attachments(&mut self) {
        let ready = {
            let Ok(mut inbox) = self.attaching.lock() else {
                return;
            };
            std::mem::take(&mut *inbox)
        };

        for (terminal_id, attachment) in ready {
            match attachment.and_then(|attachment| TerminalPane::new(terminal_id.clone(), attachment))
            {
                Ok(pane) => {
                    self.terminal_errors.remove(&terminal_id);
                    self.terminals.insert(terminal_id, pane);
                }
                Err(error) => {
                    let message = format!("{error}");
                    self.model
                        .error(format!("shell {terminal_id} is unavailable: {message}"));
                    self.terminal_errors.insert(terminal_id, message);
                }
            }
        }
    }

    pub(crate) fn close_pane(&mut self, pane_id: &str) {
        let pane = self.model.layout.panes.get(pane_id).cloned();
        let layout = std::mem::replace(&mut self.model.layout, layout::empty_layout());
        self.model.layout = layout::close_pane(layout, pane_id);

        // Closing a shell's tab ends the shell: unlike the web frontend, where a closed tab
        // may just be a navigation away, this is the only window it had.
        if let Some(Pane::Terminal { terminal_id, .. }) = pane {
            self.terminals.remove(&terminal_id);
            self.terminal_errors.remove(&terminal_id);
            let session_id = self.model.root_session_id.clone();
            self.tasks.spawn(
                move |backend| backend.close_terminal(&session_id, &terminal_id),
                |model, result| model.report(result, "could not close the shell"),
            );
        }
    }

    fn apply_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() && !self.model.palette.open {
            // A text box has the keyboard; only the palette's own chord still applies.
            let opened = ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    Key::P,
                ))
            });
            if opened {
                self.model.palette.open = true;
                self.model.palette.query.clear();
                self.model.palette.highlighted = 0;
            }
            return;
        }

        // A shell gets every plain keystroke: `s` there means the letter s. Only the
        // command chords are app-wide.
        let plain_keys_are_ours = self.active_pane_kind() != Some(PaneKind::Terminal);

        let (open_palette, toggle_theme) = ctx.input_mut(|input| {
            (
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    Key::P,
                )),
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    Key::J,
                )),
            )
        });
        let (stage, unstage) = if plain_keys_are_ours {
            ctx.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, Key::S),
                    input.consume_key(egui::Modifiers::NONE, Key::U),
                )
            })
        } else {
            (false, false)
        };

        if open_palette {
            self.model.palette.open = true;
            self.model.palette.query.clear();
            self.model.palette.highlighted = 0;
        }
        if toggle_theme {
            self.set_theme(self.model.theme.toggled());
        }
        if stage || unstage {
            self.apply_hunk_shortcut(stage);
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

    /// The pane in front of the active frame, which is what the keyboard is talking to.
    fn active_pane(&self) -> Option<&Pane> {
        let frame = self
            .model
            .layout
            .frames
            .get(&self.model.layout.active_frame_id)?;
        self.model.layout.panes.get(frame.active_pane_id.as_ref()?)
    }

    fn active_pane_kind(&self) -> Option<PaneKind> {
        self.active_pane().map(Pane::kind)
    }

    /// The review in the frontmost pane of the active frame, if that pane is a review.
    pub(crate) fn focused_review_session(&self) -> Option<String> {
        match self.active_pane()? {
            Pane::Review { session_id, .. } => Some(session_id.clone()),
            _ => None,
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
                let go = ui
                    .add_enabled(!path.is_empty(), egui::Button::new("Open review"))
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

    /// Draw whatever pane is in front of a frame.
    pub(crate) fn draw_pane(&mut self, ui: &mut Ui, pane: &Pane) {
        match pane {
            Pane::Review { session_id, .. } => {
                let session_id = session_id.clone();
                review::draw(self, ui, &session_id);
            }
            Pane::Agents { .. } => review::draw_agents(self, ui),
            Pane::Terminal { terminal_id, .. } => self.draw_terminal(ui, terminal_id),
        }
    }

    fn draw_terminal(&mut self, ui: &mut Ui, terminal_id: &str) {
        let palette = self.palette_of();
        if let Some(error) = self.terminal_errors.get(terminal_id) {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new(error.clone()).color(palette.warn));
            });
            return;
        }

        let Some(pane) = self.terminals.get_mut(terminal_id) else {
            self.attach_terminal(terminal_id);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("attaching…").color(palette.muted));
            });
            return;
        };

        let font = egui::FontId::monospace(theme::CODE_SIZE);
        let wants_repaint = pane.ui(ui, &palette, font);
        if wants_repaint {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
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
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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


/// Where the pane arrangement is kept between runs.
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
        match serde_json::from_str::<crate::native::layout::WorkspaceLayout>(&encoded) {
            Ok(stored) => self.model.restored_layout = Some(stored),
            Err(error) => eprintln!("[moonreview] ignoring a stored layout: {error}"),
        }
    }
}

impl App {
    /// One frame of the whole window. Split out of the `eframe::App` impl so the UI tests can
    /// render it without a window or an `eframe::Frame`.
    pub(crate) fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // The palette belongs to whichever context is being drawn into, which is not
        // necessarily the one the app was built with.
        if self.needs_style {
            theme::apply(ctx, self.model.theme);
            self.needs_style = false;
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
                    MenuAction::OpenCommandPalette => {
                    self.model.palette.open = true;
                    self.model.palette.query.clear();
                    self.model.palette.highlighted = 0;
                    continue;
                }
            });
        }

        self.apply_shortcuts(ctx);
        let focused = ctx.input(|input| input.focused);
        self.poll_reviews(focused);
        self.poll_submodules();
        if std::mem::take(&mut self.model.adopt_shells_pending) {
            self.adopt_existing_shells();
        }
        self.prune_diff_cache();

        self.draw_workspace(ui);
        palette::draw(self, ctx);
        self.draw_toasts(ctx);

        // Deferred so a pane is never mutated while the tree that holds it is being drawn.
        if let Some(action) = self.pending_action.take() {
            self.run_action(action);
        }
        if let Some(pane_id) = self.pending_close.take() {
            self.close_pane(&pane_id);
        }

        // Terminals whose shell has gone stop repainting, so nothing spins on a dead pane.
        let live_terminals = self
            .terminals
            .values()
            .any(|terminal| !terminal.has_exited());
        if live_terminals {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            ctx.request_repaint_after(POLL_INTERVAL);
        }
    }
}
