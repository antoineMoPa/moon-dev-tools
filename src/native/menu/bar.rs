//! The menu bar the window draws for itself, along the top of the page, in a browser.
//!
//! The same menus as the macOS bar in the parent module, less the items that act on the
//! machine the window runs on - file pickers, new windows, restarting, launchers - which a
//! page has no machine for. Drawn each frame rather than built once, so it can list only the
//! project commands the project has set, the way the palette does, where the system bar has
//! to carry all three and say no when one is picked.
//!
//! A chord written beside an item is the one its keyboard binding has, so the bar doubles as
//! the place to learn them - the browser keeps a few for itself (⌘T, ⌘W, ⌘N), which is why
//! the tab items are here at all.

use egui::Ui;

use super::MenuAction;
use crate::{
    native::{
        bindings::{self, Action},
        theme::Palette,
    },
    project::{ProjectCommand, ProjectConfig},
};

/// The bar's id, and the top panel's.
const BAR_ID: &str = "moonreview-menu-bar";

/// Draw the bar, and say what was picked from it this frame. `project` decides which of the
/// project's commands are offered: only the ones it has set.
pub(crate) fn draw(ui: &mut Ui, palette: &Palette, project: &ProjectConfig) -> Vec<MenuAction> {
    let mut picked = Vec::new();
    egui::Panel::top(BAR_ID)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(palette.header_bg)
                .inner_margin(egui::Margin::symmetric(4, 2)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    item(
                        ui,
                        "Find File…",
                        Some(Action::FindFile),
                        MenuAction::FindFile,
                        &mut picked,
                    );
                    item(
                        ui,
                        "Search Contents…",
                        Some(Action::SearchContent),
                        MenuAction::SearchContent,
                        &mut picked,
                    );
                });
                ui.menu_button("View", |ui| {
                    item(
                        ui,
                        "Switch Light and Dark",
                        Some(Action::ToggleTheme),
                        MenuAction::ToggleTheme,
                        &mut picked,
                    );
                    item(
                        ui,
                        "Command Palette",
                        Some(Action::OpenPalette),
                        MenuAction::OpenCommandPalette,
                        &mut picked,
                    );
                });
                ui.menu_button("Project", |ui| {
                    item(
                        ui,
                        "Switch Project…",
                        None,
                        MenuAction::SwitchProject,
                        &mut picked,
                    );
                    ui.separator();
                    let mut offered_any = false;
                    for which in [
                        ProjectCommand::Build,
                        ProjectCommand::Run,
                        ProjectCommand::BuildAndRun,
                    ] {
                        if project.line(which).is_none() {
                            continue;
                        }
                        offered_any = true;
                        let mut label = which.label().to_string();
                        label[..1].make_ascii_uppercase();
                        item(ui, &label, None, MenuAction::RunProject(which), &mut picked);
                    }
                    if offered_any {
                        ui.separator();
                    }
                    item(
                        ui,
                        "Project Settings…",
                        None,
                        MenuAction::OpenProject,
                        &mut picked,
                    );
                });
                ui.menu_button("Tools", |ui| {
                    item(
                        ui,
                        "Review",
                        Some(Action::OpenReview),
                        MenuAction::OpenReview,
                        &mut picked,
                    );
                    item(ui, "Tasks", None, MenuAction::OpenTasks, &mut picked);
                    item(ui, "Work Log", None, MenuAction::OpenWorkLog, &mut picked);
                    item(
                        ui,
                        "Submodule Status",
                        Some(Action::OpenSubmodules),
                        MenuAction::OpenSubmodules,
                        &mut picked,
                    );
                });
                ui.menu_button("Window", |ui| {
                    item(
                        ui,
                        "New Terminal Tab",
                        Some(Action::NewShellTab),
                        MenuAction::NewTab,
                        &mut picked,
                    );
                    item(
                        ui,
                        "Close Tab",
                        Some(Action::CloseTab),
                        MenuAction::CloseTab,
                        &mut picked,
                    );
                });
            });
        });
    picked
}

/// One item of a menu: its label, the chord of the binding that does the same thing when
/// there is one, and what picking it asks for. Picking closes the menu, the way a bar's
/// menus close.
fn item(
    ui: &mut Ui,
    label: &str,
    bound: Option<Action>,
    action: MenuAction,
    picked: &mut Vec<MenuAction>,
) {
    let mut button = egui::Button::new(label);
    if let Some(chord) = bound.and_then(bindings::chord_of) {
        button = button.shortcut_text(bindings::describe(chord));
    }
    if ui.add(button).clicked() {
        picked.push(action);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable as _};

    use super::*;
    use crate::native::theme::ThemeMode;

    /// A bar over a project, whose picks are collected across the harness's frames.
    fn bar_over(
        project: ProjectConfig,
    ) -> (
        Harness<'static>,
        std::sync::Arc<std::sync::Mutex<Vec<MenuAction>>>,
    ) {
        let picked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let picked_in_ui = std::sync::Arc::clone(&picked);
        let harness = Harness::builder()
            .with_size(egui::vec2(600.0, 300.0))
            .build_ui(move |ui| {
                let palette = Palette::of(ThemeMode::Dark);
                picked_in_ui
                    .lock()
                    .expect("the picks")
                    .extend(draw(ui, &palette, &project));
            });
        (harness, picked)
    }

    /// An item's label, as the accessibility tree reads it: the text and its chord, spaced.
    fn labelled(label: &str, bound: Action) -> String {
        let chord = bindings::describe(bindings::chord_of(bound).expect("the action is bound"));
        format!("{label} {chord}")
    }

    #[test]
    fn picking_an_item_asks_for_its_action_once_and_closes_the_menu() {
        let (mut harness, picked) = bar_over(ProjectConfig::default());
        harness.run();
        let review = labelled("Review", Action::OpenReview);

        harness.get_by_label("Tools").click();
        harness.run();
        harness.get_by_label(&review).click();
        harness.run();

        assert_eq!(
            *picked.lock().expect("the picks"),
            vec![MenuAction::OpenReview]
        );
        assert!(
            harness.query_by_label(&review).is_none(),
            "the menu should close on a pick"
        );
    }

    /// Only the commands the project has set are offered: an item that runs nothing is
    /// worse than no item.
    #[test]
    fn the_project_menu_lists_only_the_commands_that_are_set() {
        let (mut harness, _) = bar_over(ProjectConfig {
            build: Some("cargo build".to_string()),
            run: None,
            indent: None,
        });
        harness.run();

        harness.get_by_label("Project").click();
        harness.run();

        assert!(harness.query_by_label("Build").is_some(), "build is set");
        assert!(harness.query_by_label("Run").is_none(), "run is not set");
        assert!(
            harness.query_by_label("Build and run").is_none(),
            "needs both"
        );
        assert!(harness.query_by_label("Project Settings…").is_some());
    }

    /// An item reads its chord beside it, so the bar is where the bindings are learned.
    #[test]
    fn an_item_with_a_binding_shows_its_chord() {
        let (mut harness, _) = bar_over(ProjectConfig::default());
        harness.run();

        harness.get_by_label("File").click();
        harness.run();

        let find_file = labelled("Find File…", Action::FindFile);
        assert!(
            harness.query_by_label(&find_file).is_some(),
            "the find file item should read {find_file}"
        );
    }
}
