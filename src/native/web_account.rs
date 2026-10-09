//! Account - personal GitHub credentials and saved browser arrangements.
use crate::{
    native::{
        app::App,
        theme::{self, Palette, ThemeMode},
        workspace_color::WorkspaceColor,
    },
    web::account::{Account, Action},
};

impl App {
    pub(crate) fn web_account_label(&self) -> String {
        let login = self
            .web_account
            .as_ref()
            .and_then(|account| account.0.borrow().profile.login.clone());
        format!("[{}]", login.as_deref().unwrap_or("account"))
    }

    /// A phone keeps its menus in the tab strip; reserve a header for Account
    /// even while no project is open, so workspace content cannot cover it.
    pub(crate) fn draw_web_account_header(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette_of();
        egui::Panel::top("web-account-header")
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.header_bg)
                    .inner_margin(egui::Margin {
                        left: 4,
                        right: 4,
                        top: 4,
                        bottom: 2,
                    }),
            )
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(self.web_account_label())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.showing_web_account = true;
                    }
                });
            });
    }

    pub(crate) fn draw_web_account(&mut self, ctx: &egui::Context) {
        let Some(account) = self.web_account.clone() else {
            return;
        };
        account.tick();
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        if account.0.borrow().stopped {
            draw_account_changed(ctx, &account);
            return;
        }
        if !self.showing_web_account {
            return;
        }
        draw_account_window(ctx, &account, Some(&mut self.showing_web_account));
    }
}

/// An account change is a window-wide boundary: no further workspace actions can
/// accidentally target the identity a different browser tab just connected.
fn draw_account_changed(ctx: &egui::Context, account: &Account) {
    egui::Modal::new(egui::Id::new("account changed")).show(ctx, |ui| {
        ui.heading("Account changed");
        ui.label("Reload to use the current account.");
        let state = account.0.borrow();
        if !state.error.is_empty() {
            ui.label(&state.error);
        }
        if ui.button("Reload").clicked() {
            Account::reload();
        }
    });
}

/// The Account window: who is signed in, and the way in or out. `open` is what its close
/// mark turns off, which the window has none of where it is all there is - see
/// [`SignInPane`].
fn draw_account_window(ctx: &egui::Context, account: &Account, open: Option<&mut bool>) {
    let width = (ctx.content_rect().width() - 32.).clamp(220., 360.);
    let mut action = None;
    let mut cancel_device = false;
    let mut window = egui::Window::new("Account")
        .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
        .default_width(width)
        .max_width(width)
        .max_height((ctx.content_rect().height() - 60.).max(180.))
        .resizable(false)
        .collapsible(false);
    if let Some(open) = open {
        window = window.open(open);
    }
    window.show(ctx, |ui| {
        let state = account.0.borrow();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                if let Some(login) = &state.profile.login {
                    ui.add_space(12.);
                    ui.label(format!("Signed in as {login}"));
                }
                // Only a server that gives each person a Unix user says one.
                if let Some(unix_user) = &state.profile.unix_user {
                    ui.weak(format!("Unix user {unix_user}"))
                        .on_hover_text("Your shells and agents on this server run as this user.");
                }
                if state.profile.only_sign_in_is_offered() {
                    ui.add_space(12.);
                    ui.weak("Sign in to use this server.");
                }
                if !state.error.is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, &state.error);
                }
                if state.busy {
                    ui.spinner();
                    ui.label("Working…");
                }
                ui.add_enabled_ui(!state.busy, |ui| {
                    if state.profile.id.is_some() {
                        ui.add_space(12.);
                        if ui.button("Disconnect GitHub").clicked() {
                            action = Some(Action::Disconnect);
                        }
                        ui.add_space(12.);
                    }
                    if state.profile.id.is_none() && state.device.is_none() {
                        ui.add_space(12.);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    state.profile.device_flow,
                                    egui::Button::new("Sign in with GitHub"),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_disabled_hover_text(
                                    "Set MOON_GITHUB_CLIENT_ID on the server to enable GitHub sign-in.",
                                )
                                .clicked()
                            {
                                action = Some(Action::Device);
                            }
                        });
                        ui.add_space(12.);
                    }
                });
                if let Some(device) = &state.device {
                    ui.label("Enter this code on GitHub:");
                    ui.monospace(&device.user_code);
                    if ui
                        .button("Copy code")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        ui.ctx().copy_text(device.user_code.clone());
                    }
                    ui.add(
                        egui::Hyperlink::from_label_and_url(
                            "Open GitHub activation",
                            &device.verification_uri,
                        )
                        .open_in_new_tab(true),
                    );
                    ui.add_space(12.);
                    ui.label("Waiting for approval…");
                    if ui
                        .add_enabled(!state.busy, egui::Button::new("Check now"))
                        .clicked()
                    {
                        action = Some(Action::Poll(device.flow_id.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_device = true;
                    }
                }
            });
        });
    });
    if cancel_device {
        account.cancel_device();
    }
    if let Some(action) = action {
        account.dispatch(action);
    }
}

/// The whole window on a server that lets nobody past the Account pane before they sign in
/// with GitHub, until this browser has: the sign-in, over nothing. There is no backend under
/// it, so nothing is asked that the server would refuse. Signing in loads the page again,
/// which is what opens the window proper - see `Account::perform`.
pub(crate) struct SignInPane {
    account: Account,
    theme: ThemeMode,
    styled: bool,
}

impl SignInPane {
    pub(crate) fn new(ctx: &egui::Context, account: Account) -> Self {
        let theme = match ctx.theme() {
            egui::Theme::Dark => ThemeMode::Dark,
            egui::Theme::Light => ThemeMode::Light,
        };
        Self {
            account,
            theme,
            styled: false,
        }
    }
}

impl eframe::App for SignInPane {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.styled {
            theme::apply(&ctx, self.theme, WorkspaceColor::default());
            self.styled = true;
        }
        egui::CentralPanel::default().show(ui, |_| {});
        self.account.tick();
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        if self.account.0.borrow().stopped {
            draw_account_changed(&ctx, &self.account);
            return;
        }
        draw_account_window(&ctx, &self.account, None);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Palette::of(self.theme).bg.to_normalized_gamma_f32()
    }
}
