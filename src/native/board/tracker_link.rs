//! Tracker link - a task's issue in a tracker kept elsewhere - Linear, Jira, GitHub, GitLab:
//! the id a card shows under its description, and the box the link is written in on the
//! task's pane.
//!
//! What a link is and how its id is read out of it are [`crate::moontasks::remote_tracker`].

use egui::{Key, Modifiers, RichText, Ui};

use crate::{
    moontasks::{TaskView, remote_tracker},
    native::{
        app::App,
        board::{BoardAction, gesture::Controls},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// The issue's id - `BM-3343` - as a link that opens the issue in the browser, and the gap
/// under it. Nothing is drawn for a task with no link, so only a card that has one gives it
/// a row.
pub(super) fn draw_on_card(ui: &mut Ui, task: &TaskView, card: &mut Controls, palette: &Palette) {
    let url = &task.remote_task_tracker_url;
    if url.is_empty() {
        return;
    }
    // Cut to the row: a link with no id in it is written as the link, which is longer than
    // a card is wide.
    let id = widgets::cut_to_fit(
        ui,
        &remote_tracker::label_of(url),
        egui::FontId::proportional(SMALL_SIZE),
        palette.accent,
        ui.available_width(),
        1,
    );
    let link = ui.add(egui::Link::new(id)).on_hover_text(url);
    if card.pressed(&link) {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    ui.add_space(3.0);
}

/// The box the link is written in, on the task's pane. Kept by Enter or by clicking away,
/// thrown away by Escape, the way the title above it is; emptied, it takes the link off.
///
/// Filled in again when the board's answer changes under it and not otherwise, so an answer
/// that has not caught up with what was just typed cannot take it back.
pub(crate) fn draw_field(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    ui.label(
        RichText::new("Tracker link")
            .size(SMALL_SIZE)
            .color(palette.muted),
    );
    ui.add_space(3.0);

    let editor =
        app.model.board.task_editors.get_mut(&task.id).expect(
            "the pane's title and notes boxes open the task's editor before this box draws",
        );
    if editor.said_tracker_url != task.remote_task_tracker_url {
        editor.tracker_url.clone_from(&task.remote_task_tracker_url);
        editor
            .said_tracker_url
            .clone_from(&task.remote_task_tracker_url);
    }

    let entry = ui.add(
        egui::TextEdit::singleline(&mut editor.tracker_url)
            .hint_text("https://linear.app/…/issue/BM-3343")
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(6, 4)),
    );
    // Taken before `lost_focus` is read, which the same press of Escape also sets.
    let thrown_away =
        entry.has_focus() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if thrown_away {
        editor.tracker_url.clone_from(&task.remote_task_tracker_url);
    }
    let written = editor.tracker_url.trim();
    if !thrown_away && entry.lost_focus() && written != task.remote_task_tracker_url {
        actions.push(BoardAction::SetRemoteTrackerUrl(
            task.id.clone(),
            written.to_string(),
        ));
    }
}
