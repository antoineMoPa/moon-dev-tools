//! The review pane: a header, a sidebar of files and comments, and the hunks themselves.

pub(crate) mod agents;
pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod header;
pub(crate) mod hunks;
pub(crate) mod image_diff;
pub(crate) mod search;
pub(crate) mod sidebar;

use egui::{RichText, Ui};

use crate::native::{app::App, theme::Palette};

pub(crate) use agents::draw as draw_agents;

pub(crate) fn draw(app: &mut App, ui: &mut Ui, session_id: &str) {
    let palette = app.palette_of();

    // A review that has never loaded has nothing to lay out around yet.
    let has_payload = app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.payload.is_some());
    if !has_payload {
        draw_placeholder(app, ui, session_id, &palette);
        return;
    }

    egui::Panel::top(egui::Id::new(("review-header", session_id)))
        .frame(
            egui::Frame::new()
                .fill(palette.header_bg)
                .inner_margin(egui::Margin::symmetric(8, 5)),
        )
        .show(ui, |ui| header::draw(app, ui, session_id, &palette));

    egui::Panel::left(egui::Id::new(("review-sidebar", session_id)))
        .resizable(true)
        .default_size(270.0)
        .size_range(180.0..=520.0)
        .frame(
            egui::Frame::new()
                .fill(palette.panel)
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| sidebar::draw(app, ui, session_id, &palette));

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.bg)
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| hunks::draw(app, ui, session_id, &palette));
}

fn draw_placeholder(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    let error = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.error.clone());

    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        match error {
            Some(error) => {
                ui.label(RichText::new("this review could not be loaded").color(palette.ink));
                ui.add_space(6.0);
                ui.label(RichText::new(error).color(palette.warn));
                ui.add_space(12.0);
                if crate::native::widgets::clickable(ui.button("try again")).clicked() {
                    app.model.review(session_id).refresh_requested = true;
                }
            }
            None => {
                ui.spinner();
                ui.add_space(6.0);
                ui.label(RichText::new("reading the diff…").color(palette.muted));
            }
        }
    });
}
