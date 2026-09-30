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
pub(crate) fn draw(
    ui: &mut Ui,
    palette: &Palette,
    project: &ProjectConfig,
    tabs: &[TabEntry],
) -> Vec<MenuAction> {
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
                menus(ui, project, &mut picked);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    hamburger(ui, egui::vec2(30.0, 20.0), tabs, None, &mut picked);
                });
            });
        });
    picked
}

/// The bar's menus, File to Window.
fn menus(ui: &mut Ui, project: &ProjectConfig, picked: &mut Vec<MenuAction>) {
    ui.menu_button("File", |ui| {
        item(
            ui,
            "Find File…",
            Some(Action::FindFile),
            MenuAction::FindFile,
            picked,
        );
        item(
            ui,
            "Search Contents…",
            Some(Action::SearchContent),
            MenuAction::SearchContent,
            picked,
        );
    });
    ui.menu_button("View", |ui| {
        item(
            ui,
            "Switch Light and Dark",
            Some(Action::ToggleTheme),
            MenuAction::ToggleTheme,
            picked,
        );
        item(
            ui,
            "Command Palette",
            Some(Action::OpenPalette),
            MenuAction::OpenCommandPalette,
            picked,
        );
    });
    ui.menu_button("Project", |ui| {
        item(
            ui,
            "Switch Project…",
            None,
            MenuAction::SwitchProject,
            picked,
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
            item(ui, &label, None, MenuAction::RunProject(which), picked);
        }
        if offered_any {
            ui.separator();
        }
        item(
            ui,
            "Project Settings…",
            None,
            MenuAction::OpenProject,
            picked,
        );
    });
    ui.menu_button("Tools", |ui| {
        item(
            ui,
            "Review",
            Some(Action::OpenReview),
            MenuAction::OpenReview,
            picked,
        );
        item(ui, "Tasks", None, MenuAction::OpenTasks, picked);
        item(ui, "Work Log", None, MenuAction::OpenWorkLog, picked);
        item(
            ui,
            "Submodule Status",
            Some(Action::OpenSubmodules),
            MenuAction::OpenSubmodules,
            picked,
        );
    });
    ui.menu_button("Window", |ui| {
        item(
            ui,
            "New Terminal Tab",
            Some(Action::NewShellTab),
            MenuAction::NewTab,
            picked,
        );
        item(
            ui,
            "Close Tab",
            Some(Action::CloseTab),
            MenuAction::CloseTab,
            picked,
        );
    });
}

/// A tab as the hamburger at the right of the bar lists it.
pub(crate) struct TabEntry {
    pub(crate) pane_id: egui_frames::PaneId,
    pub(crate) title: String,
    /// Whether it is the tab in front of the frame the keyboard is in, which the menu leaves out.
    pub(crate) in_front: bool,
}

/// The hamburger: every open tab, in the order `tabs` has them - a window's frames show only
/// a few at a time, and on a phone a strip of tabs is mostly out of reach. A window with no bar
/// of its own passes its `project`, and the bar's menus follow the tabs in the list.
pub(crate) fn hamburger(
    ui: &mut Ui,
    size: egui::Vec2,
    tabs: &[TabEntry],
    project: Option<&ProjectConfig>,
    picked: &mut Vec<MenuAction>,
) {
    // Painted rather than typed: no font is promised to have the glyph.
    let (button, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let ink = if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let reach = size.x * 0.27;
    let gap = size.y * 0.25;
    for line in 0..3 {
        let y = button.center().y + (line as f32 - 1.0) * gap;
        ui.painter().line_segment(
            [
                egui::pos2(button.center().x - reach, y),
                egui::pos2(button.center().x + reach, y),
            ],
            egui::Stroke::new(1.5, ink),
        );
    }
    egui::Popup::menu(&response).width(240.0).show(|ui| {
        // Not the tab in front: there is nowhere to switch to from it, and its row would only
        // push the others down.
        for tab in tabs.iter().filter(|tab| !tab.in_front) {
            let row = egui::Button::new(egui::RichText::new(&tab.title)).min_size(egui::vec2(ui.available_width(), 28.0));
            if ui.add(row).clicked() {
                picked.push(MenuAction::FocusTab(tab.pane_id));
                ui.close();
            }
        }
        if let Some(project) = project {
            // Said rather than left blank, so a window with one tab does not look as though
            // the list failed to load.
            if tabs.iter().all(|tab| tab.in_front) {
                ui.weak("no other tabs");
            }
            ui.separator();
            menus(ui, project, picked);
        }
    });
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
                    .extend(draw(ui, &palette, &project, &[]));
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
