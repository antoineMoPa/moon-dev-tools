//! The strip over a task's shells and files: the task's whole title and its tags.
//!
//! A tab has room for the start of a title, and a task's shell or file is opened in a tab of
//! its own, so the header is where the rest of it is read - and what the task is marked with,
//! which no tab says at all.

use egui::{RichText, Ui};

use crate::native::{app::App, board::tags, theme::UI_SIZE};

/// Draw the header of `task_id`, if the board has read that task. The board's own answer is
/// used as it stands: a title renamed on the card reads renamed here on the next frame.
///
/// `inspectable` is for a shell whose agent tells moon what it is doing (Minerve): the header
/// then has an `inspect` button at its right, and answers whether it was clicked.
pub(crate) fn draw(app: &App, ui: &mut Ui, task_id: &str, inspectable: bool) -> bool {
    let Some(task) = app.model.board.tasks.iter().find(|task| task.id == task_id) else {
        return false;
    };
    let mut inspect = false;
    let palette = app.palette_of();
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 9,
            right: 9,
            top: 3,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            let title =
                egui::Label::new(RichText::new(&task.title).size(UI_SIZE).color(palette.ink))
                    .selectable(false);
            if inspectable {
                // The button first, from the right, and the title wraps in what it leaves.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    inspect = ui
                        .small_button("inspect")
                        .on_hover_text("The conversation and its tool calls")
                        .clicked();
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| ui.add(title),
                    );
                });
            } else {
                ui.add(title);
            }
            // A task with no tags has no row of them: an empty wrapped row is still a line tall.
            if !task.tags.is_empty() {
                tags::draw_read_only(ui, &task.tags, &palette);
            }
        });
    // The line under it, drawn rather than laid out: a separator adds a gap on either side.
    let line = ui.available_rect_before_wrap().left_top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        line.y,
        egui::Stroke::new(1.0, palette.line),
    );
    ui.add_space(1.0);
    inspect
}
