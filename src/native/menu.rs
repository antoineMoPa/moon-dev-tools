//! The application menu.
//!
//! On macOS the menu bar belongs to the system, not to the window, so it is built with the
//! platform API. Everywhere else there is no system-wide bar to put these in, and the same
//! actions are reached from the command palette — which is also where macOS users can find
//! them, so nothing lives only in the menu.

/// Something the menu asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MenuAction {
    OpenInBrowser,
    ToggleTheme,
    OpenCommandPalette,
    /// Ask the OS which file of the repo to open for editing.
    OpenFile,
    NewTab,
    CloseTab,
    /// Open another window of one of the three programs, on its launch screen.
    NewWindow(crate::cli::Frame),
    InstallLaunchers,
    /// Open the script the board picks work up by.
    EditAutopilot,
    ViewAutopilotLog,
}

#[cfg(target_os = "macos")]
mod platform {
    use muda::{
        Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };

    use super::MenuAction;
    use crate::{
        cli::{Frame, NEW_WINDOW_FRAMES},
        native::programs,
    };

    /// The menu, kept alive for as long as the window: dropping it would take the bar with it.
    pub(crate) struct NativeMenu {
        _menu: Menu,
        open_in_browser: MenuId,
        toggle_theme: MenuId,
        command_palette: MenuId,
        open_file: MenuId,
        new_tab: MenuId,
        close_tab: MenuId,
        /// One per program that is installed, in [`NEW_WINDOW_FRAMES`] order.
        new_windows: Vec<(MenuId, Frame)>,
        install_launchers: MenuId,
        edit_autopilot: MenuId,
        view_autopilot_log: MenuId,
    }

    impl NativeMenu {
        /// Install the menu bar. `serves_web` decides whether the browser item is offered at
        /// all, since a window with no server behind it has nothing to open; `picks_files`
        /// the same for Open File, which needs the repo to be on this machine for the OS
        /// picker to reach it. `frame` is which of the three programs this window is — the
        /// one whose new window takes ⌘N.
        pub(crate) fn install(serves_web: bool, picks_files: bool, frame: Frame) -> Option<Self> {
            let menu = Menu::new();

            // Written on demand rather than by the installer, which drops executables on PATH
            // and knows nothing of Launchpad.
            let install_launchers = MenuItem::new("Install Desktop Launchers", true, None);

            // macOS expects the first submenu to be the application menu, and it is what
            // gives the window a real Quit item.
            let app_menu = Submenu::new("Moonreview", true);
            app_menu
                .append_items(&[
                    &PredefinedMenuItem::about(None, None),
                    &PredefinedMenuItem::separator(),
                    &install_launchers,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .ok()?;

            // A File menu with the one thing this window opens files for: reading and editing
            // one in a tab. ⌘O is what the chord means everywhere else, and the review's own
            // way to a file — clicking it in the sidebar — opens the same tab.
            let open_file = MenuItem::new(
                "Open File…",
                picks_files,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyO)),
            );
            let file_menu = Submenu::new("File", true);
            file_menu.append(&open_file).ok()?;

            let open_in_browser = MenuItem::new(
                "Open in Browser",
                serves_web,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyB)),
            );
            let toggle_theme = MenuItem::new(
                "Switch Light and Dark",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyJ)),
            );
            let command_palette = MenuItem::new(
                "Command Palette",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::META | Modifiers::SHIFT),
                    Code::KeyP,
                )),
            );

            let view_menu = Submenu::new("View", true);
            view_menu
                .append_items(&[
                    &open_in_browser,
                    &PredefinedMenuItem::separator(),
                    &toggle_theme,
                    &command_palette,
                ])
                .ok()?;

            // There is deliberately no Edit menu.
            //
            // The predefined Copy, Paste and Select All items carry ⌘C, ⌘V and ⌘A as their key
            // equivalents, and macOS hands a chord to the menu bar before the window ever sees
            // it. Those items act on the `copy:` and `paste:` selectors, which nothing in a
            // winit window implements, so the chord was swallowed and nothing happened —
            // while ⌘⇧C, which no menu item claims, fell through to the window and worked.
            //
            // Without them the chords reach egui, which turns them into `Event::Copy` and
            // `Event::Paste` itself. Adding them back takes plain ⌘C away again.

            // ⌘W and ⌘T are the window's own items rather than the predefined ones, so the
            // system does not close the whole window out from under a single tab.
            let new_tab = MenuItem::new(
                "New Terminal Tab",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyT)),
            );
            let close_tab = MenuItem::new(
                "Close Tab",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyW)),
            );

            // A window of each program, so moonshell is a menu item away from the board rather
            // than a trip to the terminal. Each opens on its launch screen, asking which repo
            // to work in. Only the ones installed beside this executable are offered: an item
            // that could not open anything would be a broken promise.
            //
            // ⌘N opens another window of this same program, which is what the chord means in
            // every other application; the other two are named and unbound.
            let new_windows: Vec<(MenuItem, Frame)> = NEW_WINDOW_FRAMES
                .iter()
                .filter(|offered| programs::executable_for(**offered).is_some())
                .map(|offered| {
                    let accelerator = (*offered == frame)
                        .then(|| Accelerator::new(Some(Modifiers::META), Code::KeyN));
                    let item = MenuItem::new(
                        format!("New {} Window", offered.display_name()),
                        true,
                        accelerator,
                    );
                    (item, *offered)
                })
                .collect();

            // What the board does when it is running is a script in the repo, so the menu's
            // one autopilot item opens that file rather than offering settings there are none
            // of. The board's own [edit autopilot] link does the same thing.
            let edit_autopilot = MenuItem::new("Edit Autopilot Script", true, None);
            // What it did and why it did nothing: a hook fires unattended, so the only account
            // of its reasoning is the one it wrote down.
            let view_autopilot_log = MenuItem::new("View Autopilot Logs", true, None);
            let autopilot_menu = Submenu::new("Autopilot", true);
            autopilot_menu.append(&edit_autopilot).ok()?;
            autopilot_menu.append(&view_autopilot_log).ok()?;

            let window_menu = Submenu::new("Window", true);
            for (item, _) in &new_windows {
                window_menu.append(item).ok()?;
            }
            window_menu
                .append_items(&[
                    &PredefinedMenuItem::separator(),
                    &new_tab,
                    &close_tab,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::fullscreen(None),
                ])
                .ok()?;

            menu.append_items(&[
                &app_menu,
                &file_menu,
                &view_menu,
                &autopilot_menu,
                &window_menu,
            ])
            .ok()?;
            menu.init_for_nsapp();

            Some(Self {
                _menu: menu,
                open_in_browser: open_in_browser.id().clone(),
                toggle_theme: toggle_theme.id().clone(),
                command_palette: command_palette.id().clone(),
                open_file: open_file.id().clone(),
                new_tab: new_tab.id().clone(),
                close_tab: close_tab.id().clone(),
                new_windows: new_windows
                    .iter()
                    .map(|(item, frame)| (item.id().clone(), *frame))
                    .collect(),
                install_launchers: install_launchers.id().clone(),
                edit_autopilot: edit_autopilot.id().clone(),
                view_autopilot_log: view_autopilot_log.id().clone(),
            })
        }

        /// Everything the menu was asked for since the last frame.
        pub(crate) fn drain(&self) -> Vec<MenuAction> {
            let mut actions = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let action = if event.id == self.open_in_browser {
                    MenuAction::OpenInBrowser
                } else if event.id == self.toggle_theme {
                    MenuAction::ToggleTheme
                } else if event.id == self.command_palette {
                    MenuAction::OpenCommandPalette
                } else if event.id == self.open_file {
                    MenuAction::OpenFile
                } else if event.id == self.new_tab {
                    MenuAction::NewTab
                } else if event.id == self.close_tab {
                    MenuAction::CloseTab
                } else if event.id == self.install_launchers {
                    MenuAction::InstallLaunchers
                } else if event.id == self.edit_autopilot {
                    MenuAction::EditAutopilot
                } else if event.id == self.view_autopilot_log {
                    MenuAction::ViewAutopilotLog
                } else if let Some((_, frame)) =
                    self.new_windows.iter().find(|(id, _)| *id == event.id)
                {
                    MenuAction::NewWindow(*frame)
                } else {
                    continue;
                };
                actions.push(action);
            }
            actions
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::MenuAction;

    /// No system-wide menu bar to install into; the command palette carries these actions.
    pub(crate) struct NativeMenu;

    impl NativeMenu {
        pub(crate) fn install(
            _serves_web: bool,
            _picks_files: bool,
            _frame: crate::cli::Frame,
        ) -> Option<Self> {
            None
        }

        pub(crate) fn drain(&self) -> Vec<MenuAction> {
            Vec::new()
        }
    }
}

pub(crate) use platform::NativeMenu;
