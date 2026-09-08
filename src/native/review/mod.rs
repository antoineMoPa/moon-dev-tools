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

/// The item every mention of a file in a review carries: the file itself, in a tab of its
/// own. A diff says what changed, and the rest of the file is what says whether it should
/// have - so a right-click on a file's name anywhere in the review is the way to the file.
///
/// Drawn inside a context menu, which it closes once the tab has been asked for.
pub(crate) fn open_file_item(app: &mut App, ui: &mut Ui, session_id: &str, file_path: &str) {
    if crate::native::widgets::clickable(ui.button("open the file")).clicked() {
        app.open_file_pane(session_id, file_path);
        ui.close();
    }
}

/// The gesture that goes with it: ⌘-click a file's name and the file opens, the way
/// ⌘-clicking a name inside a diff opens where it is defined. Command on macOS, ctrl
/// elsewhere, and exactly it - shift-⌘ is not this gesture.
///
/// Returns whether the click has been taken, so a name that means something else on a plain
/// click - the sidebar's rows scroll to the file, a move hint jumps to the other hunk - can
/// leave its own click alone. The pointer turns into a hand while ⌘ is down over the name, so
/// the gesture is there to be seen before it is made.
#[must_use]
pub(crate) fn opens_the_file(
    app: &mut App,
    ui: &Ui,
    response: &egui::Response,
    session_id: &str,
    file_path: &str,
) -> bool {
    if !response.contains_pointer()
        || !ui.input(|input| input.modifiers.matches_exact(egui::Modifiers::COMMAND))
    {
        return false;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    if !response.clicked() {
        return false;
    }
    app.open_file_pane(session_id, file_path);
    true
}

/// Both ways to the file at once, for the mentions of one that have nothing else to do on a
/// click: ⌘-click it, or right-click it and pick the file out of a menu of one.
pub(crate) fn opens_the_file_on_its_own(
    app: &mut App,
    ui: &Ui,
    response: &egui::Response,
    session_id: &str,
    file_path: &str,
) {
    let _taken = opens_the_file(app, ui, response, session_id, file_path);
    open_file_menu(app, response, session_id, file_path);
}

/// A context menu offering nothing but the file, for the mentions of one that have nothing
/// else to do to it.
pub(crate) fn open_file_menu(
    app: &mut App,
    response: &egui::Response,
    session_id: &str,
    file_path: &str,
) {
    egui::Popup::context_menu(response).show(|ui| open_file_item(app, ui, session_id, file_path));
}

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
