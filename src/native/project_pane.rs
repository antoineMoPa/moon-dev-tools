//! The project pane: the two commands the Project menu runs, and where they are set.
//!
//! What builds a repo is a fact about the repo, so it is kept in the repo - see
//! [`crate::project`]. This is the frame that writes that file, so a project is configured
//! from the window rather than by finding the file first.

use egui::{RichText, Ui};

use crate::{
    native::{app::App, model::ProjectEditor, theme::SMALL_SIZE, widgets},
    project::ProjectCommand,
};

/// What each box is for, in the order the menu has them. The pane is two of the same row, so
/// what differs between them is here rather than written out twice.
///
/// Every box is labelled above it as well as hinted inside it: the hint is gone the moment
/// anything is typed, and a filled-in box with nothing over it says nothing about which of
/// the two commands it holds.
const BOXES: &[(ProjectCommand, &str)] = &[
    (
        ProjectCommand::Build,
        "the command that builds this project",
    ),
    (
        ProjectCommand::Run,
        // The one word that is not a line of shell - see `crate::project::RESTART_RUN_COMMAND`.
        "the command that runs this project, or @restart to start this window again",
    ),
];

/// How wide a command box gets. A command line is longer than a name and shorter than a
/// paragraph, and the pane is often a narrow column beside a review.
const BOX_WIDTH: f32 = 420.0;

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Project Settings")
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            widgets::divider(ui, &palette);
            ui.add_space(8.0);

            // The file is read when the pane opens; until it is back there is nothing to put
            // in the boxes, and empty boxes would read as a project with no commands set.
            let Some(editor) = &mut app.model.project_editor else {
                ui.label(
                    RichText::new("reading the project file")
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                return;
            };

            // The pane was opened to type in, so the first box has the keyboard from the
            // moment the file is back and there is something to type into.
            let mut takes_keyboard = std::mem::take(&mut app.model.project_focus);
            let mut edited = false;
            for (which, hint) in BOXES {
                ui.label(
                    RichText::new(format!("{} command", which.label()))
                        .size(SMALL_SIZE)
                        .strong(),
                );
                ui.add_space(2.0);
                let entry = ui.add(
                    egui::TextEdit::singleline(editor.text_mut(*which))
                        .hint_text(*hint)
                        .desired_width(BOX_WIDTH)
                        .margin(egui::Margin::symmetric(6, 4)),
                );
                if std::mem::take(&mut takes_keyboard) {
                    entry.request_focus();
                }
                edited |= entry.changed();
                ui.add_space(10.0);
            }

            // Saved as it is typed rather than behind a button: there are two boxes and one
            // file, and a command left unsaved is a menu item that does not do what the pane
            // says it does. The write itself is `App::save_project`, one at a time.
            app.model.project_unsaved |= edited;

            ui.label(
                RichText::new(offered(&app.model.project_editor))
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
        });
}

/// What the menu is offering, out of what the boxes hold - so a box someone has emptied
/// visibly takes its item away rather than leaving one that runs nothing.
fn offered(editor: &Option<ProjectEditor>) -> String {
    let Some(editor) = editor else {
        return String::new();
    };
    let offered: Vec<&str> = BOXES
        .iter()
        .filter(|(which, _)| !editor.text(*which).trim().is_empty())
        .map(|(which, _)| which.label())
        .collect();

    match offered.as_slice() {
        [] => "the Project menu is empty until one of these is filled in".to_string(),
        [one] => format!("the Project menu offers {one}"),
        many => format!("the Project menu offers {}", many.join(" and ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(build: &str, run: &str) -> Option<ProjectEditor> {
        Some(ProjectEditor {
            build: build.to_string(),
            run: run.to_string(),
        })
    }

    #[test]
    fn a_project_with_neither_command_offers_nothing() {
        assert_eq!(
            offered(&editor("  ", "")),
            "the Project menu is empty until one of these is filled in"
        );
    }

    #[test]
    fn a_project_with_one_command_offers_that_one() {
        assert_eq!(offered(&editor("cargo build", "")), "the Project menu offers build");
    }

    #[test]
    fn a_project_with_both_offers_both() {
        assert_eq!(
            offered(&editor("cargo build", "cargo run")),
            "the Project menu offers build and run"
        );
    }
}
