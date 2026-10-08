//! Applications offered - what `moon › Applications` and the palette offer to start: the
//! applications installed on the server, which it reads from its desktop entries - see
//! `crate::desktop_entries` - and the person's own, out of its settings.
//!
//! The installed ones are asked for when the window opens, and again each time the palette
//! or the bar's `moon` menu is opened. The list is the server's and changes under the
//! window: a package installed in one of its shells is on it the next time it is looked at.

use crate::{
    api::applications::{InstalledApplication, InstalledApplications},
    native::app::App,
    settings::Application,
};

/// Where an application is among the ones offered. A menu's item names its application by
/// this rather than by its command, because what a menu asks for is `Copy` - see
/// [`MenuAction`](crate::native::menu::MenuAction).
#[cfg(any(target_arch = "wasm32", target_os = "linux", test))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplicationPlace {
    /// Its place among the person's own.
    Own(usize),
    /// Its place among the installed ones.
    Installed(usize),
}

/// What there is to start, as far as the window has heard from the server.
#[derive(Clone, Copy)]
pub(crate) struct ApplicationsOffered<'a> {
    /// The person's own, in the order the settings have them.
    pub(crate) own: &'a [Application],
    /// What the server answered when asked what it has installed, once it has.
    pub(crate) installed: Option<&'a InstalledApplications>,
}

impl<'a> ApplicationsOffered<'a> {
    /// The installed applications there are to list, by name. There are none before the
    /// server has answered, and none on a server off Linux.
    pub(crate) fn installed_listed(&self) -> &'a [InstalledApplication] {
        match self.installed {
            Some(InstalledApplications::Listed(installed)) => installed,
            Some(InstalledApplications::NotOnLinux) | None => &[],
        }
    }

    /// The line of shell that starts the application at `place` - a place in the lists the
    /// menu that named it was drawn from, this frame.
    #[cfg(any(target_arch = "wasm32", target_os = "linux", test))]
    pub(crate) fn command_at(&self, place: ApplicationPlace) -> &'a str {
        match place {
            ApplicationPlace::Own(place) => &self.own[place].command,
            ApplicationPlace::Installed(place) => &self.installed_listed()[place].command,
        }
    }
}

impl App {
    /// What `moon › Applications` and the palette offer, as far as the server has said.
    pub(crate) fn applications_offered(&self) -> ApplicationsOffered<'_> {
        ApplicationsOffered {
            own: match &self.model.settings {
                Some(settings) => &settings.applications,
                None => &[],
            },
            installed: self.model.installed_applications.as_ref(),
        }
    }

    /// Ask the server which applications it has installed. Every window asks, whatever it
    /// is on: one with nowhere to start an application has no menu or command to list them
    /// in - see `application_commands` in the palette.
    pub(crate) fn list_installed_applications(&mut self) {
        self.tasks.spawn_keyed(
            Some("installed-applications".to_owned()),
            |backend| backend.installed_applications(),
            |model, installed| match installed {
                Ok(installed) => model.installed_applications = Some(installed),
                Err(error) => {
                    model.error(format!("could not list the server's applications: {error:#}"));
                }
            },
        );
    }

    /// Open the command palette, on the applications the server has installed now.
    pub(crate) fn show_palette(&mut self) {
        self.model.palette.show();
        self.list_installed_applications();
    }
}
