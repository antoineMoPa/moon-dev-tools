//! One frame of the window: the workspace, and the prompts and toasts drawn over it.

use std::time::Duration;

use egui::{Align, Align2, CornerRadius, Key, Layout as UiLayout, RichText, Ui, vec2};

use crate::{
    api::OpenSessionRequest,
    native::{
        bindings::{self}, find, fonts, logos,
        menu::{MenuAction, NativeMenu},
        model::{Stage, ToastKind},
        palette::{self, CommandAction},
        theme::{self, Palette, SMALL_SIZE},
        widgets,
        workspace::SHELL_REPAINT_INTERVAL,
    },
};

use super::{App, POLL_INTERVAL, TabAction};

/// How big the app's logo is drawn on the screens a window opens on. Big enough to be the
/// thing the eye lands on when the window appears, small enough that what is under it stays
/// in view.
const LOGO_POINTS: f32 = 80.0;
/// Between that logo and the program's name under it.
const LOGO_GAP: f32 = 8.0;

impl App {

    pub(super) fn draw_prompt(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        // A repo on this machine can be pointed at; one on the far side of a remote connection
        // can only be typed out, since this machine cannot browse for it.
        let picks_folders = self.backend().reads_this_machine();
        let mut open_path = None;
        let mut pick_folder = false;

        let frame = self.frame;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                // The logo is drawn above where the heading used to start, so the prompt and
                // the recent projects under it stay where they were in the window.
                ui.add_space((ui.available_height() * 0.28 - LOGO_POINTS - LOGO_GAP).max(0.0));
                // The window's own logo, drawn from the 256px asset: at 80 points it covers
                // 160 pixels on a Retina display, and the 128px one would be upscaled there.
                ui.add(
                    egui::Image::new(logos::logo_image_source(frame, 256))
                        .fit_to_exact_size(vec2(LOGO_POINTS, LOGO_POINTS)),
                );
                ui.add_space(LOGO_GAP);
                ui.label(RichText::new(frame.program()).size(22.0).strong());
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

    pub(super) fn draw_opening(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        let ctx = ui.ctx().clone();
        let frame = self.frame;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                // Same subtraction as the launch screen this replaces, so the name and the
                // spinner stay put as one screen gives way to the other.
                ui.add_space((ui.available_height() * 0.4 - LOGO_POINTS - LOGO_GAP).max(0.0));
                ui.add(
                    egui::Image::new(logos::logo_image_source(frame, 256))
                        .fit_to_exact_size(vec2(LOGO_POINTS, LOGO_POINTS)),
                );
                ui.add_space(LOGO_GAP);
                ui.label(RichText::new(frame.program()).size(20.0).strong());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 140.0).max(0.0) / 2.0);
                    ui.spinner();
                    ui.label(RichText::new(frame.opening()).color(palette.muted));
                });
            });
        });
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    /// A chord that has begun but not finished says so in the corner, the way emacs echoes
    /// `C-x-`: otherwise a half-typed prefix silently swallows the next key.
    pub(super) fn draw_armed_prefix(&mut self, ctx: &egui::Context) {
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

    pub(super) fn draw_toasts(&mut self, ctx: &egui::Context) {
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
                                    if widgets::close_button(ui, &palette).clicked() {
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
        if self.window_theme_frames > 0 {
            self.window_theme_frames -= 1;
            theme::apply_window_theme(ctx, self.model.theme);
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
                MenuAction::ToggleTheme => CommandAction::ToggleTheme,
                MenuAction::InstallLaunchers => CommandAction::InstallLaunchers,
                MenuAction::NewWindow(frame) => CommandAction::NewWindow(frame),
                MenuAction::RestartWindow => CommandAction::RestartWindow,
                MenuAction::OpenFile => CommandAction::OpenFile,
                MenuAction::FindFile => CommandAction::FindFile,
                MenuAction::SearchContent => CommandAction::SearchContent,
                MenuAction::RunProject(which) => CommandAction::RunProject(which),
                MenuAction::OpenProject => {
                    CommandAction::OpenPane(crate::native::panes::OpenPaneRequest::Project)
                }
                MenuAction::OpenReview => {
                    self.open_root_review();
                    continue;
                }
                MenuAction::OpenSubmodules => {
                    CommandAction::OpenPane(crate::native::panes::OpenPaneRequest::Submodules)
                }
                MenuAction::NewTab => {
                    self.pending_tab_action = Some(TabAction::New);
                    continue;
                }
                MenuAction::CloseTab => {
                    self.pending_tab_action = Some(TabAction::Close);
                    continue;
                }
                MenuAction::OpenCommandPalette => {
                    self.model.palette.show();
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
        self.poll_review_requests();
        self.poll_running_shells();
        self.poll_board();
        self.open_shell_the_board_started();
        self.open_file_the_board_readied();
        if std::mem::take(&mut self.model.project_pending) {
            self.load_project();
        }
        self.save_project();
        if std::mem::take(&mut self.model.adopt_shells_pending) {
            self.adopt_existing_shells();
        }
        if std::mem::take(&mut self.model.open_shell_pending) {
            let primary = self.model.layout.primary_frame();
            let session_id = self.model.root_session_id.clone();
            self.spawn_terminal(
                session_id,
                None,
                crate::native::workspace::TerminalPlacement::Tab(primary),
            );
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
            self.run_action(ctx, action);
        }
        if let Some(pane_id) = self.pending_close.take()
            && !self.close_would_lose_edits(pane_id)
        {
            self.close_pane(pane_id);
        }
        self.close_tabs_of_exited_shells(ctx);
        // A task's page is open while its card is marked, so a card let go of by a click on
        // the board takes its page with it - here, where the tree is no longer being drawn.
        crate::native::board::close_pages_let_go_of(self);

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

    format!("🌚 {} | {shortened}", frame.program())
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
