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
    NewTab,
    CloseTab,
    NewTask,
    InstallLaunchers,
}

#[cfg(target_os = "macos")]
mod platform {
    use muda::{
        Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    };

    use super::MenuAction;

    /// The menu, kept alive for as long as the window: dropping it would take the bar with it.
    pub(crate) struct NativeMenu {
        _menu: Menu,
        open_in_browser: MenuId,
        toggle_theme: MenuId,
        command_palette: MenuId,
        new_tab: MenuId,
        close_tab: MenuId,
        new_task: MenuId,
        install_launchers: MenuId,
    }

    impl NativeMenu {
        /// Install the menu bar. `serves_web` decides whether the browser item is offered at
        /// all, since a window with no server behind it has nothing to open.
        pub(crate) fn install(serves_web: bool) -> Option<Self> {
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

            // The board's own item. It sits with the other "new" ones rather than in View,
            // because it makes something rather than showing something: it opens the board if
            // it is not open and starts a card there with the cursor in it.
            let new_task = MenuItem::new(
                "New Moontask",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyN)),
            );

            let window_menu = Submenu::new("Window", true);
            window_menu
                .append_items(&[
                    &new_task,
                    &new_tab,
                    &close_tab,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::fullscreen(None),
                ])
                .ok()?;

            menu.append_items(&[&app_menu, &view_menu, &window_menu])
                .ok()?;
            menu.init_for_nsapp();

            Some(Self {
                _menu: menu,
                open_in_browser: open_in_browser.id().clone(),
                toggle_theme: toggle_theme.id().clone(),
                command_palette: command_palette.id().clone(),
                new_tab: new_tab.id().clone(),
                close_tab: close_tab.id().clone(),
                new_task: new_task.id().clone(),
                install_launchers: install_launchers.id().clone(),
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
                } else if event.id == self.new_tab {
                    MenuAction::NewTab
                } else if event.id == self.close_tab {
                    MenuAction::CloseTab
                } else if event.id == self.new_task {
                    MenuAction::NewTask
                } else if event.id == self.install_launchers {
                    MenuAction::InstallLaunchers
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
        pub(crate) fn install(_serves_web: bool) -> Option<Self> {
            None
        }

        pub(crate) fn drain(&self) -> Vec<MenuAction> {
            Vec::new()
        }
    }
}

pub(crate) use platform::NativeMenu;
