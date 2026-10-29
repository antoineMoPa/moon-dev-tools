//! The menu bar the window draws for itself, along its top: in a browser, whose own bar is
//! the browser's, and in a window that is its machine's desktop - `moon desktop` - where no
//! system is there to draw one.
//!
//! The same menus as the macOS bar in the parent module, less the items that act on the
//! machine the window runs on - new windows, restarting, launchers - which a page has no
//! machine for, and in a page less the two that pick a file. Drawn each frame, so it lists
//! only the commands each target has set, the way the palette does.
//! And one item more than that bar has: the QR code of the page's own address - see
//! `crate::native::login_link::page_qr`.
//!
//! A chord written beside an item is the one its keyboard binding has, so the bar doubles as
//! the place to learn them - the browser keeps a few for itself (⌘T, ⌘W, ⌘N), which is why
//! the tab items are here at all.

use egui::Ui;

use super::MenuAction;
use crate::{
    native::{
        applications_offered::ApplicationsOffered,
        bindings::{self, Action},
        logos,
        theme::Palette,
    },
    project::{ProjectCommand, ProjectConfig},
};

/// The bar's id, and the top panel's.
const BAR_ID: &str = "moonreview-menu-bar";

/// How far below the middle of its line the logo is drawn, in points, to sit level with the
/// letters of "moon" beside it.
const LOGO_DROP: f32 = 1.5;

/// Draw the bar, and say what was picked from it this frame. `project` decides which of the
/// targets' commands are offered: only the ones they have set.
pub(crate) fn draw(
    ui: &mut Ui,
    palette: &Palette,
    project: &ProjectConfig,
    applications: ApplicationsOffered<'_>,
    #[cfg(target_arch = "wasm32")] account_label: &str,
) -> Vec<MenuAction> {
    let mut picked = Vec::new();
    egui::Panel::top(BAR_ID)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(palette.header_bg)
                .inner_margin(egui::Margin {
                    left: 4,
                    right: 4,
                    top: if cfg!(target_arch = "wasm32") { 4 } else { 2 },
                    bottom: 2,
                }),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                #[cfg(target_arch = "wasm32")]
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(account_label)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        picked.push(MenuAction::ShowWebAccount);
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        menus(ui, project, applications, &mut picked);
                    });
                });
                #[cfg(not(target_arch = "wasm32"))]
                menus(ui, project, applications, &mut picked);
            });
        });
    picked
}

/// The bar's menus, moon to Window.
fn menus(
    ui: &mut Ui,
    project: &ProjectConfig,
    applications: ApplicationsOffered<'_>,
    picked: &mut Vec<MenuAction>,
) {
    // The window's own menu, first as a system's is: what there is to start on the server's
    // desktop - see `crate::native::applications_offered`.
    let height = ui.text_style_height(&egui::TextStyle::Button);
    // A button centers its picture on the line, and "moon" has no tall letters: its middle is
    // below the line's. The picture is drawn that much lower inside its own square, which
    // leaves the button's layout alone.
    let drop = LOGO_DROP / height;
    let logo = egui::Image::new(logos::moon_logo_image_source())
        .fit_to_exact_size(egui::Vec2::splat(height))
        .uv(egui::Rect::from_min_max(
            egui::pos2(0.0, -drop),
            egui::pos2(1.0, 1.0 - drop),
        ));
    let moon = ui.menu_button((logo, "moon"), |ui| {
        ui.menu_button("Applications", |ui| {
            super::applications::draw(ui, applications, picked);
        });
        item(ui, "Settings…", None, MenuAction::OpenSettings, picked);
    });
    // Opening the menu asks the server again for what it has installed, so that a package
    // installed since the window opened is listed.
    if moon.response.clicked() {
        picked.push(MenuAction::ListApplications);
    }
    ui.menu_button("File", |ui| {
        // Picked with the window's picker - see `crate::native::file_picker`. A browser's
        // window does not open a file by picking yet.
        #[cfg(not(target_arch = "wasm32"))]
        {
            item(ui, "New File", None, MenuAction::NewFile, picked);
            item(ui, "Open File…", None, MenuAction::OpenFile, picked);
            ui.separator();
        }
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
        project_items(ui, project, picked);
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
        item(ui, "Wire", None, MenuAction::OpenWire, picked);
        item(
            ui,
            "Submodule Status",
            Some(Action::OpenSubmodules),
            MenuAction::OpenSubmodules,
            picked,
        );
        // The one item of the bar that is the page's alone: the code is of the address the
        // page came from, which a window that is no page has none of.
        #[cfg(target_arch = "wasm32")]
        {
            ui.separator();
            item(ui, "Account…", None, MenuAction::ShowWebAccount, picked);
            item(ui, "Show QR Code", None, MenuAction::ShowPageQr, picked);
        }
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

/// The Project menu's commands: a submenu per target with the ones it has set.
fn project_items(ui: &mut Ui, project: &ProjectConfig, picked: &mut Vec<MenuAction>) {
    for target in &project.targets {
        ui.menu_button(&target.name, |ui| {
            for which in ProjectCommand::ALL {
                if target.line(which).is_none() {
                    continue;
                }
                let action = MenuAction::RunProject {
                    target: target.name.clone(),
                    which,
                };
                item(ui, &which.menu_label(), None, action, picked);
            }
        });
    }
    if !project.targets.is_empty() {
        ui.separator();
    }
}

/// A tab as the hamburger lists it.
#[cfg(target_arch = "wasm32")]
pub(crate) struct TabEntry {
    pub(crate) pane_id: egui_frames::PaneId,
    pub(crate) title: String,
    /// Whether it is the tab in front of the frame the keyboard is in, which the menu leaves out.
    pub(crate) in_front: bool,
}

/// The hamburger of a phone's tab strip, whose window has no room for the bar: every open tab,
/// in the order `tabs` has them - a strip of tabs is mostly out of reach there - and then the
/// bar's menus. A window wide enough for the bar has its tabs in its strips, and no hamburger.
#[cfg(target_arch = "wasm32")]
pub(crate) fn hamburger(
    ui: &mut Ui,
    size: egui::Vec2,
    tabs: &[TabEntry],
    project: &ProjectConfig,
    applications: ApplicationsOffered<'_>,
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
        // Said rather than left blank, so a window with one tab does not look as though
        // the list failed to load.
        if tabs.iter().all(|tab| tab.in_front) {
            ui.weak("no other tabs");
        }
        ui.separator();
        menus(ui, project, applications, picked);
    });
}

/// One item of a menu: its label, the chord of the binding that does the same thing when
/// there is one, and what picking it asks for. Picking closes the menu, the way a bar's
/// menus close.
pub(super) fn item(
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
    use crate::{native::theme::ThemeMode, project::BuildTarget};

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
                let applications = ApplicationsOffered {
                    own: &[],
                    installed: None,
                };
                picked_in_ui.lock().expect("the picks").extend(draw(
                    ui,
                    &palette,
                    &project,
                    applications,
                ));
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

    /// Each target has a submenu of only the commands it has set: an item that runs nothing
    /// is worse than no item.
    #[test]
    fn the_project_menu_has_a_submenu_per_target_of_the_commands_it_set() {
        let (mut harness, picked) = bar_over(ProjectConfig {
            targets: vec![
                BuildTarget::typed("app", "cargo build", ""),
                BuildTarget::typed("web", "npm run build", "npm run preview"),
            ],
            indent: None,
            day_ends_at: None,
            palette_shell: None,
        });
        harness.run();

        harness.get_by_label("Project").click();
        harness.run();

        assert!(harness.query_by_label("Project Settings…").is_some());
        harness.get_by_label_contains("app").hover();
        harness.run();
        assert!(harness.query_by_label("Build").is_some(), "app builds");
        assert!(harness.query_by_label("Run").is_none(), "app has no run");

        harness.get_by_label_contains("web").hover();
        harness.run();
        harness.get_by_label("Build and run").click();
        harness.run();

        assert_eq!(
            *picked.lock().expect("the picks"),
            vec![MenuAction::RunProject {
                target: "web".to_string(),
                which: ProjectCommand::BuildAndRun,
            }]
        );
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
