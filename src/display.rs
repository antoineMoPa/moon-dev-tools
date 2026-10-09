//! Server desktop - one virtual X display per server. Applications started from a window open
//! their windows on it, and a pane shows it in a window that may be on another machine. See
//! `moon_display` for the display itself, and [`crate::api::display`] for the messages sent
//! each way on the websocket a pane watches it through.
//!
//! The desktop is created when the first application is started; every application after that
//! is started on the same desktop, with `DISPLAY` set to its name. It belongs to the server,
//! like a shell: it keeps running when the window that started it is closed, and any window of
//! the server can watch it. It ends when its last application exits, or when the server does.
//!
//! On a server that gives each person a Unix user - see [`crate::unix_users`] - there is one
//! desktop per person instead, and none of the server's own. A person's desktop is theirs
//! alone: its X server and its applications run as them, X lets nobody else onto it - see
//! `running` - and every route answers about the desktop of whoever asks.
//!
//! A shell of the server starts an application on it as well, with `moon launch <command>` -
//! see [`crate::instances::server`] for how that reaches the server, and
//! [`VIEW_WITHOUT_A_WINDOW`] for the desktop it is given when there was none.
//!
//! Only a server on Linux has a desktop: the display is Xvfb, which is installed on Linux.

mod routes;
#[cfg(target_os = "linux")]
mod running;
// Started on Linux only; tested wherever the tests run.
#[cfg(any(target_os = "linux", test))]
pub(crate) mod started;

pub(crate) use routes::{end, shown, socket, start_application};

use std::path::Path;
#[cfg(target_os = "linux")]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::Result;

use crate::{
    api::display::DisplayView,
    unix_users::{Person, UnixUser},
};

/// The server's desktops: the one it has, or the one each person has - see the module.
#[derive(Default)]
pub(crate) struct ServerDisplay {
    /// By the name of the Unix user each is kept to, and under `None` the one desktop of a
    /// server that runs everything as its own user.
    #[cfg(target_os = "linux")]
    desktops: Mutex<HashMap<Option<String>, Arc<Desktop>>>,
}

/// One desktop's place: whose it is, and the desktop itself while one is running there.
#[cfg(target_os = "linux")]
struct Desktop {
    /// The person its applications run as, and the only one let onto it. `None` for the one
    /// desktop of a server that runs everything as its own user.
    kept_to: Option<Person>,
    running: Mutex<Option<Arc<running::Running>>>,
}

/// Whose desktop `person` is on: their own, on a server that gives each person a Unix user,
/// and nobody's - the server's one desktop - anywhere else.
///
/// Nobody asking, on a server of the first kind, is refused. There is no desktop of the
/// server's own there to put an application on: it would run as root.
#[cfg(any(target_os = "linux", test))]
fn desktop_owner(
    person: Option<&Person>,
    each_person_has_a_unix_user: bool,
) -> Result<Option<&Person>> {
    match (each_person_has_a_unix_user, person) {
        (false, _) => Ok(None),
        (true, Some(person)) => Ok(Some(person)),
        (true, None) => anyhow::bail!(
            "this was asked for nobody, on a server where every desktop is one person's and \
             its applications run as their own Unix user"
        ),
    }
}

/// The view a desktop is started with for a shell's `moon launch`, in pixels, and the scale
/// its applications draw at. A window that starts a desktop says both of its own screen - see
/// [`crate::api::display::StartApplicationRequest`] - and a shell has no screen to say them
/// of.
///
/// The view is only what the desktop starts with: a pane showing it asks for its own size as
/// soon as it is drawn. The scale stays for as long as the desktop does, so on a screen of
/// twice the density its applications are drawn at half the size a window's own start would
/// have given them, until the desktop is ended and started again from that window.
const VIEW_WITHOUT_A_WINDOW: [u16; 2] = [1280, 800];
const SCALE_WITHOUT_A_WINDOW: u8 = 1;

impl crate::api::AppState {
    /// Start what a `moon launch` typed in a shell asks for, on the desktop of the `person`
    /// whose shell it is - and the desktop first, when there is none. Nobody's, where nobody
    /// has a Unix user of their own: the shell is the server's then, and so is the desktop.
    pub(crate) fn start_for(
        &self,
        person: Option<&Person>,
        command: &str,
        folder: &Path,
    ) -> Result<String> {
        crate::api::mark_activity(&self.last_activity);
        let desktop = self.display.start_application(
            person,
            command,
            folder,
            VIEW_WITHOUT_A_WINDOW,
            SCALE_WITHOUT_A_WINDOW,
        )?;
        let whose = match person {
            Some(_) => "your desktop on the server",
            None => "the server's desktop",
        };
        Ok(format!("{whose}, DISPLAY={}", desktop.name))
    }
}

/// A server starts what a `moon launch` typed in one of its shells asks for on its desktop,
/// as it does what a window asks for.
impl crate::instances::StartsApplications for crate::api::AppState {
    /// Asked by nobody in particular, which is how a server that has the one desktop is
    /// asked. One that gives each person a desktop refuses it - see [`desktop_owner`].
    fn start(&self, command: &str, folder: &Path) -> Result<String> {
        self.start_for(None, command, folder)
    }

    fn start_for_person(&self, person: &Person, command: &str, folder: &Path) -> Result<String> {
        self.start_for(Some(person), command, folder)
    }
}

#[cfg(not(target_os = "linux"))]
const ONLY_ON_LINUX: &str =
    "applications are started on an Xvfb display, which only a server on Linux has";

#[cfg(not(target_os = "linux"))]
impl ServerDisplay {
    pub(crate) fn start_application(
        &self,
        _person: Option<&Person>,
        _command: &str,
        _folder: &Path,
        _view: [u16; 2],
        _scale: u8,
    ) -> Result<DisplayView> {
        anyhow::bail!(ONLY_ON_LINUX)
    }

    pub(crate) fn shown(&self, _person: Option<&Person>) -> Result<Option<DisplayView>> {
        Ok(None)
    }

    pub(crate) fn end(&self, _person: Option<&Person>) -> Result<()> {
        Ok(())
    }

    pub(crate) fn end_for(&self, _user: &UnixUser) {}
}

#[cfg(target_os = "linux")]
impl ServerDisplay {
    /// Start `command` on the desktop of `person`, run in `folder` - and the desktop first,
    /// with a view of `view` and applications drawn at `scale`, when there is none. Answered
    /// once a window opens on the desktop or the application ends - with an error when it
    /// ends in failure - and after [`started::SOON`] at the latest.
    pub(crate) fn start_application(
        &self,
        person: Option<&Person>,
        command: &str,
        folder: &Path,
        view: [u16; 2],
        scale: u8,
    ) -> Result<DisplayView> {
        let owner = desktop_owner(person, crate::unix_users::each_person_has_one())?;
        let desktop = {
            let mut desktops = self.desktops();
            let place = desktops
                .entry(owner.map(|person| person.unix_user.name.clone()))
                .or_insert_with(|| {
                    Arc::new(Desktop {
                        kept_to: owner.cloned(),
                        running: Mutex::default(),
                    })
                });
            Arc::clone(place)
        };
        // With the lock let go of: one person's desktop starting keeps nobody from theirs.
        desktop.start_application(command, folder, view, scale)
    }

    /// The desktop of `person`, when one is running.
    pub(crate) fn shown(&self, person: Option<&Person>) -> Result<Option<DisplayView>> {
        Ok(self.running(person)?.map(|desktop| desktop.view.clone()))
    }

    /// End the desktop of `person` and every application on it.
    pub(crate) fn end(&self, person: Option<&Person>) -> Result<()> {
        if let Some(desktop) = self.desktop_of(person)? {
            desktop.end();
        }
        Ok(())
    }

    /// End the desktop kept to `user`: its X server, and every application on it. What ends
    /// everything a person has on a display, for when they are kicked.
    pub(crate) fn end_for(&self, user: &UnixUser) {
        let theirs = self.desktops().remove(&Some(user.name.clone()));
        if let Some(desktop) = theirs {
            desktop.end();
        }
    }

    /// The desktop of `person` that is running, for a socket to watch.
    fn running(&self, person: Option<&Person>) -> Result<Option<Arc<running::Running>>> {
        Ok(self.desktop_of(person)?.and_then(|desktop| desktop.get()))
    }

    fn desktop_of(&self, person: Option<&Person>) -> Result<Option<Arc<Desktop>>> {
        let owner = desktop_owner(person, crate::unix_users::each_person_has_one())?;
        let whose = owner.map(|person| person.unix_user.name.clone());
        Ok(self.desktops().get(&whose).cloned())
    }

    fn desktops(&self) -> MutexGuard<'_, HashMap<Option<String>, Arc<Desktop>>> {
        self.desktops.lock().expect("the desktops lock is poisoned")
    }
}

#[cfg(target_os = "linux")]
impl Desktop {
    /// Start `command` here - see [`ServerDisplay::start_application`].
    fn start_application(
        &self,
        command: &str,
        folder: &Path,
        view: [u16; 2],
        scale: u8,
    ) -> Result<DisplayView> {
        let desktop = {
            let mut running = self.running.lock().expect("the display lock is poisoned");
            // One whose applications have all ended is over, and only not yet let go of.
            match running.as_ref().filter(|desktop| !desktop.is_over()) {
                Some(desktop) => Arc::clone(desktop),
                None => {
                    let started = running::Running::start(view, scale, self.kept_to.as_ref())?;
                    *running = Some(Arc::clone(&started));
                    started
                }
            }
        };
        // With the lock let go of: this waits to hear how the application's start went, and
        // whoever asks what the desktop is meanwhile is not kept waiting with it.
        desktop.start_application(command, folder)?;
        Ok(desktop.view.clone())
    }

    /// End the desktop and every application on it.
    fn end(&self) {
        if let Some(desktop) = self
            .running
            .lock()
            .expect("the display lock is poisoned")
            .take()
        {
            desktop.end();
        }
    }

    fn get(&self) -> Option<Arc<running::Running>> {
        self.running
            .lock()
            .expect("the display lock is poisoned")
            .as_ref()
            .filter(|desktop| !desktop.is_over())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(login: &str) -> Person {
        Person {
            github_login: login.to_string(),
            unix_user: UnixUser {
                name: format!("moon-{login}"),
                uid: 2001,
                gid: 2000,
                home: format!("/home/moon-{login}").into(),
                shell: "/bin/bash".to_string(),
            },
        }
    }

    #[test]
    fn a_desktop_is_the_servers_one_until_each_person_has_a_unix_user_and_then_only_theirs() {
        let someone = person("someone");

        // A server that runs everything as its own user has the one desktop, whoever asks.
        assert_eq!(desktop_owner(None, false).expect("the server's"), None);
        assert_eq!(
            desktop_owner(Some(&someone), false).expect("the server's"),
            None
        );

        assert_eq!(
            desktop_owner(Some(&someone), true).expect("theirs"),
            Some(&someone)
        );
        let refused = desktop_owner(None, true).expect_err("root has no desktop of its own");
        assert!(
            refused.to_string().contains("asked for nobody"),
            "{refused}"
        );
    }
}
