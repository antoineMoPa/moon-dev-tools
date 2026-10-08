//! Account - personal GitHub credentials and saved browser arrangements.
use crate::{
    native::app::App,
    web::account::{Account, Action},
};

impl App {
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
                        .button("[account]")
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
        let state = account.0.borrow();
        let stopped = state.stopped;
        // An account change is a window-wide boundary: no further workspace actions can
        // accidentally target the identity a different browser tab just connected.
        if stopped {
            drop(state);
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
            return;
        }
        drop(state);
        if !self.showing_web_account {
            return;
        }
        let mut showing = true;
        let width = (ctx.content_rect().width() - 32.).clamp(220., 360.);
        let mut action = None;
        let mut cancel_device = false;
        egui::Window::new("Account")
            .open(&mut showing)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .default_width(width)
            .max_width(width)
            .max_height((ctx.content_rect().height() - 60.).max(180.))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let state = account.0.borrow();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(login) = &state.profile.login {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.label(format!("Signed in as {login}"));
                        });
                    }
                    if !state.error.is_empty() {
                        ui.colored_label(ui.visuals().error_fg_color, &state.error);
                    }
                    if state.busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Working…");
                        });
                    }
                    ui.add_enabled_ui(!state.busy, |ui| {
                        if state.profile.id.is_some() {
                            ui.add_space(12.);
                            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                if ui.button("Disconnect GitHub").clicked() {
                                    action = Some(Action::Disconnect);
                                }
                            });
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
                        ui.separator();
                        ui.label("Enter this code on GitHub:");
                        ui.horizontal(|ui| {
                            ui.monospace(&device.user_code);
                            if ui.button("Copy code").clicked() {
                                ui.ctx().copy_text(device.user_code.clone());
                            }
                        });
                        ui.add(
                            egui::Hyperlink::from_label_and_url(
                                "Open GitHub activation",
                                &device.verification_uri,
                            )
                            .open_in_new_tab(true),
                        );
                        ui.label("Waiting for approval…");
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!state.busy, egui::Button::new("Check now"))
                                .clicked()
                            {
                                action = Some(Action::Poll(device.flow_id.clone()));
                            }
                            if ui.button("Cancel").clicked() {
                                cancel_device = true;
                            }
                        });
                    }
                });
            });
        self.showing_web_account = showing;
        if cancel_device {
            account.cancel_device();
        }
        if let Some(action) = action {
            account.dispatch(action);
        }
    }
}
