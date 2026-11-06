//! The strip over a task's shells and files: the task's whole title and its tags.
//!
//! A tab has room for the start of a title, and a task's shell or file is opened in a tab of
//! its own, so the header is where the rest of it is read - and what the task is marked with,
//! which no tab says at all.

use egui::{RichText, Ui};

use crate::native::{app::App, board::tags, theme::UI_SIZE};

/// The board task's id, as `moontasks::store` has it - named here since that module is not
/// built for the browser, which draws this header too.
const BOARD_TASK_ID: &str = "board-task";

/// Draw the header of `task_id`, if the board has read that task. The board's own answer is
/// used as it stands: a title renamed on the card reads renamed here on the next frame.
///
/// `inspectable` is for a shell whose agent tells moon what it is doing (Minerve): the header
/// then has an `inspect` button at its right, and answers whether it was clicked.
pub(crate) fn draw(app: &App, ui: &mut Ui, task_id: &str, inspectable: bool) -> bool {
    let (title, tags): (String, &[String]) =
        match app.model.board.tasks.iter().find(|task| task.id == task_id) {
            Some(task) => (task.title.clone(), &task.tags),
            // The board's own agent has no card, so its header says which board it is on.
            None if inspectable && task_id == BOARD_TASK_ID => {
                (board_agent_title(app.model.project_path.as_deref()), &[])
            }
            None => return false,
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
            let title_label =
                egui::Label::new(RichText::new(&title).size(UI_SIZE).color(palette.ink))
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
                        |ui| ui.add(title_label),
                    );
                });
            } else {
                ui.add(title_label);
            }
            // A task with no tags has no row of them: an empty wrapped row is still a line tall.
            if !tags.is_empty() {
                tags::draw_read_only(ui, tags, &palette);
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

/// The header of the board's own agent: `Board <name> agent`, where the name is the project's
/// folder, as the title bar names the project.
fn board_agent_title(project_path: Option<&str>) -> String {
    let name = project_path
        .and_then(|path| std::path::Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    format!("Board {name} agent")
}

#[cfg(test)]
mod tests {
    use super::board_agent_title;

    #[test]
    fn the_board_agents_title_names_the_project_folder() {
        assert_eq!(
            board_agent_title(Some("/Users/jameo/prog/moon-dev-tools")),
            "Board moon-dev-tools agent"
        );
    }

    #[test]
    fn the_board_agents_title_says_repo_until_a_project_is_open() {
        assert_eq!(board_agent_title(None), "Board repo agent");
    }
}
