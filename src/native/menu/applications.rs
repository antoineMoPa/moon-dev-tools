//! Applications menu - the items of `moon › Applications`: the person's own entries, then
//! the applications installed on the server, each under the submenu of its category.
//!
//! A machine has dozens of them, which one flat menu is too long for. Their desktop entries
//! say what kind of program each is - see `crate::desktop_entries` - in the main categories
//! the freedesktop menu specification registers, and those are the submenus, as they are in
//! the menu of any Linux desktop.

use std::collections::BTreeMap;

use egui::Ui;

use super::{MenuAction, bar::item};
use crate::{
    api::applications::{InstalledApplication, InstalledApplications},
    native::applications_offered::{ApplicationPlace, ApplicationsOffered},
};

/// The submenu each main category is listed under: every main category the menu
/// specification registers, called what desktops' menus call it.
const SUBMENU_OF_MAIN_CATEGORY: &[(&str, &str)] = &[
    ("AudioVideo", "Sound & Video"),
    ("Audio", "Sound & Video"),
    ("Video", "Sound & Video"),
    ("Development", "Development"),
    ("Education", "Education"),
    ("Game", "Games"),
    ("Graphics", "Graphics"),
    ("Network", "Internet"),
    ("Office", "Office"),
    ("Science", "Science"),
    ("Settings", "Settings"),
    ("System", "System"),
    ("Utility", "Accessories"),
];

/// Where an application whose entry names no main category is listed.
const SUBMENU_OF_NO_MAIN_CATEGORY: &str = "Other";

/// What the menu says in place of the installed applications of a server off Linux.
const NOT_ON_LINUX: &str = "the server is not on Linux: it has no applications to list";

/// What it says on a server on Linux that has none, and no entry of the person's own.
const NONE_INSTALLED: &str = "no applications installed";

/// Draw the items, and add what was picked to `picked`.
pub(super) fn draw(ui: &mut Ui, offered: ApplicationsOffered<'_>, picked: &mut Vec<MenuAction>) {
    for (place, application) in offered.own.iter().enumerate() {
        item(
            ui,
            &application.name,
            None,
            MenuAction::StartApplication(ApplicationPlace::Own(place)),
            picked,
        );
    }
    let installed = match offered.installed {
        // The server has not answered yet, which it does within a frame or two.
        None => return,
        Some(InstalledApplications::NotOnLinux) => {
            ui.weak(NOT_ON_LINUX);
            return;
        }
        Some(InstalledApplications::Listed(installed)) => installed,
    };
    // Said rather than left blank, so an empty menu does not look like one that failed to
    // load.
    if installed.is_empty() && offered.own.is_empty() {
        ui.weak(NONE_INSTALLED);
    }
    if !installed.is_empty() && !offered.own.is_empty() {
        ui.separator();
    }
    for (submenu, places) in places_by_submenu(installed) {
        ui.menu_button(submenu, |ui| {
            for place in places {
                item(
                    ui,
                    &installed[place].name,
                    None,
                    MenuAction::StartApplication(ApplicationPlace::Installed(place)),
                    picked,
                );
            }
        });
    }
}

/// The submenu an application is listed under: that of the first main category its entry
/// names. An entry names its categories from the general to the particular, and an
/// application is listed once.
fn submenu_of(application: &InstalledApplication) -> &'static str {
    application
        .categories
        .iter()
        .find_map(|category| {
            SUBMENU_OF_MAIN_CATEGORY
                .iter()
                .find(|(main, _)| main == category)
                .map(|(_, submenu)| *submenu)
        })
        .unwrap_or(SUBMENU_OF_NO_MAIN_CATEGORY)
}

/// The submenus that have an application, in alphabetical order, each with the places of
/// its applications in `installed` - which is sorted by name, so each submenu is too.
fn places_by_submenu(installed: &[InstalledApplication]) -> BTreeMap<&'static str, Vec<usize>> {
    let mut places_by_submenu: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
    for (place, application) in installed.iter().enumerate() {
        places_by_submenu
            .entry(submenu_of(application))
            .or_default()
            .push(place);
    }
    places_by_submenu
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(name: &str, categories: &[&str]) -> InstalledApplication {
        InstalledApplication {
            name: name.to_owned(),
            command: name.to_lowercase(),
            categories: categories.iter().map(|category| category.to_string()).collect(),
        }
    }

    #[test]
    fn an_application_is_listed_once_under_its_first_main_category() {
        let installed = [
            installed("Chromium", &["Network", "WebBrowser"]),
            installed("Totem", &["GNOME", "AudioVideo", "Video"]),
            installed("Vim", &["Utility", "TextEditor"]),
            installed("X Clock", &["Utility"]),
            installed("Zed", &[]),
        ];

        let by_submenu: Vec<(&str, Vec<usize>)> = places_by_submenu(&installed).into_iter().collect();

        assert_eq!(
            by_submenu,
            [
                ("Accessories", vec![2, 3]),
                ("Internet", vec![0]),
                ("Other", vec![4]),
                ("Sound & Video", vec![1]),
            ]
        );
    }
}
