//! The application menu.
//!
//! On macOS the menu bar belongs to the system, not to the window, so it is built with the
//! platform API. Everywhere else there is no system-wide bar to put these in, and the same
//! actions are reached from the command palette - which is also where macOS users can find
//! them, so nothing lives only in the menu.

/// Something the menu asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MenuAction {
    ToggleTheme,
    OpenCommandPalette,
    /// Ask the OS which file of the repo to open for editing.
    OpenFile,
    /// Open the palette on the file finder, where what is typed is a file name.
    FindFile,
    /// Open the palette on the content search, where what is typed is looked for in the
    /// text of the repo's files.
    SearchContent,
    NewTab,
    CloseTab,
    /// The submodule hub: every submodule of the repo, and the changed ones' reviews.
    OpenSubmodules,
    /// Bring this window's own review forward, opening it if it is not open.
    OpenReview,
    /// Run one of the project's own commands in a shell.
    RunProject(crate::project::ProjectCommand),
    /// Open the pane those commands are set in.
    OpenProject,
    /// Open another window of one of the three programs, on its launch screen.
    NewWindow(crate::cli::Frame),
    /// Start this program again on the repo this window is on, and close this window.
    RestartWindow,
    InstallLaunchers,
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
        project::ProjectCommand,
    };

    /// The menu, kept alive for as long as the window: dropping it would take the bar with it.
    pub(crate) struct NativeMenu {
        _menu: Menu,
        toggle_theme: MenuId,
        command_palette: MenuId,
        open_file: MenuId,
        find_file: MenuId,
        search_content: MenuId,
        new_tab: MenuId,
        close_tab: MenuId,
        open_review: MenuId,
        open_submodules: MenuId,
        /// One per command the Project menu runs, in the order the menu has them.
        project_commands: Vec<(MenuId, ProjectCommand)>,
        open_project: MenuId,
        /// One per program that is installed, in [`NEW_WINDOW_FRAMES`] order.
        new_windows: Vec<(MenuId, Frame)>,
        restart_window: MenuId,
        install_launchers: MenuId,
    }

    impl NativeMenu {
        /// Install the menu bar. `picks_files` decides whether Open File is offered at all,
        /// since it needs the repo to be on this machine for the OS picker to reach it.
        /// `frame` is which of the three programs this window is - the one whose new window
        /// takes ⌘N.
        pub(crate) fn install(picks_files: bool, frame: Frame) -> Option<Self> {
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
            // way to a file - clicking it in the sidebar - opens the same tab.
            let open_file = MenuItem::new(
                "Open File…",
                picks_files,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyO)),
            );
            // The two searches that open a file without knowing where it is: by its name, and
            // by text inside it. Both carry the chords their keyboard bindings have, which
            // from here on is what answers them - macOS hands a chord to the menu bar before
            // the window sees it - so both items do what those bindings did.
            let find_file = MenuItem::new(
                "Find File…",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyP)),
            );
            let search_content = MenuItem::new(
                "Search Contents…",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::META | Modifiers::SHIFT),
                    Code::KeyF,
                )),
            );
            let file_menu = Submenu::new("File", true);
            file_menu
                .append_items(&[
                    &open_file,
                    &PredefinedMenuItem::separator(),
                    &find_file,
                    &search_content,
                ])
                .ok()?;

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
                .append_items(&[&toggle_theme, &command_palette])
                .ok()?;

            // There is deliberately no Edit menu.
            //
            // The predefined Copy, Paste and Select All items carry ⌘C, ⌘V and ⌘A as their key
            // equivalents, and macOS hands a chord to the menu bar before the window ever sees
            // it. Those items act on the `copy:` and `paste:` selectors, which nothing in a
            // winit window implements, so the chord was swallowed and nothing happened -
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

            // A window is a process, so the executable it is running is the one it started
            // with: a rebuilt one is only picked up by starting again. Restart does that
            // without a trip to the terminal - the new instance opens on this window's repo.
            let restart_window = MenuItem::new("Restart", true, None);

            // The project's own two commands, and the pane they are set in.
            //
            // Both items are always there and always enabled. The bar is built as the window
            // opens, before any repo has been read, and macOS gives no way to grow it later -
            // so a command the project has not set says so when it is picked, which is what
            // the palette avoids by only listing the ones that are set.
            let project_commands: Vec<(MenuItem, ProjectCommand)> = [
                (
                    ProjectCommand::Build,
                    Accelerator::new(Some(Modifiers::META | Modifiers::SHIFT), Code::KeyB),
                ),
                (
                    ProjectCommand::Run,
                    Accelerator::new(Some(Modifiers::META), Code::KeyR),
                ),
                (
                    ProjectCommand::BuildAndRun,
                    // Off `R` and beside `build`, because `cmd+alt+R` and the review's
                    // `cmd+shift+R` were one hand's reach and one letter apart.
                    Accelerator::new(Some(Modifiers::META | Modifiers::ALT), Code::KeyB),
                ),
            ]
            .into_iter()
            .map(|(which, accelerator)| {
                let mut label = which.label().to_string();
                label[..1].make_ascii_uppercase();
                (MenuItem::new(label, true, Some(accelerator)), which)
            })
            .collect();
            let open_project = MenuItem::new("Project Settings…", true, None);
            let project_menu = Submenu::new("Project", true);
            for (item, _) in &project_commands {
                project_menu.append(item).ok()?;
            }
            project_menu
                .append_items(&[&PredefinedMenuItem::separator(), &open_project])
                .ok()?;

            // A Tools menu for the review itself and for what the window can open on the repo
            // beside it.
            //
            // Review carries the chord `Action::OpenReview` has in the keyboard table. macOS
            // hands a chord to the menu bar before the window sees it, so from here on it is
            // this item that answers cmd+shift+R rather than the binding - which is why the
            // item has to do what the binding did, and does: `App::open_root_review`.
            let open_review = MenuItem::new(
                "Review",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::META | Modifiers::SHIFT),
                    Code::KeyR,
                )),
            );
            let open_submodules = MenuItem::new(
                "Submodule Status",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::META | Modifiers::SHIFT),
                    Code::KeyS,
                )),
            );
            let tools_menu = Submenu::new("Tools", true);
            tools_menu
                .append_items(&[&open_review, &open_submodules])
                .ok()?;

            let window_menu = Submenu::new("Window", true);
            for (item, _) in &new_windows {
                window_menu.append(item).ok()?;
            }
            window_menu
                .append_items(&[
                    &PredefinedMenuItem::separator(),
                    &restart_window,
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
                &project_menu,
                &tools_menu,
                &window_menu,
            ])
            .ok()?;
            menu.init_for_nsapp();

            Some(Self {
                _menu: menu,
                toggle_theme: toggle_theme.id().clone(),
                command_palette: command_palette.id().clone(),
                open_file: open_file.id().clone(),
                find_file: find_file.id().clone(),
                search_content: search_content.id().clone(),
                new_tab: new_tab.id().clone(),
                close_tab: close_tab.id().clone(),
                open_review: open_review.id().clone(),
                open_submodules: open_submodules.id().clone(),
                project_commands: project_commands
                    .iter()
                    .map(|(item, which)| (item.id().clone(), *which))
                    .collect(),
                open_project: open_project.id().clone(),
                new_windows: new_windows
                    .iter()
                    .map(|(item, frame)| (item.id().clone(), *frame))
                    .collect(),
                restart_window: restart_window.id().clone(),
                install_launchers: install_launchers.id().clone(),
            })
        }

        /// Everything the menu was asked for since the last frame.
        pub(crate) fn drain(&self) -> Vec<MenuAction> {
            let mut actions = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let action = if event.id == self.toggle_theme {
                    MenuAction::ToggleTheme
                } else if event.id == self.command_palette {
                    MenuAction::OpenCommandPalette
                } else if event.id == self.open_file {
                    MenuAction::OpenFile
                } else if event.id == self.find_file {
                    MenuAction::FindFile
                } else if event.id == self.search_content {
                    MenuAction::SearchContent
                } else if event.id == self.new_tab {
                    MenuAction::NewTab
                } else if event.id == self.close_tab {
                    MenuAction::CloseTab
                } else if event.id == self.open_review {
                    MenuAction::OpenReview
                } else if event.id == self.open_submodules {
                    MenuAction::OpenSubmodules
                } else if event.id == self.open_project {
                    MenuAction::OpenProject
                } else if let Some((_, which)) =
                    self.project_commands.iter().find(|(id, _)| *id == event.id)
                {
                    MenuAction::RunProject(*which)
                } else if event.id == self.restart_window {
                    MenuAction::RestartWindow
                } else if event.id == self.install_launchers {
                    MenuAction::InstallLaunchers
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
        pub(crate) fn install(_picks_files: bool, _frame: crate::cli::Frame) -> Option<Self> {
            None
        }

        pub(crate) fn drain(&self) -> Vec<MenuAction> {
            Vec::new()
        }
    }
}

pub(crate) use platform::NativeMenu;
