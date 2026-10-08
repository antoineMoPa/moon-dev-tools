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

use anyhow::Result;

use crate::api::display::DisplayView;

/// The server's desktop, when it has one.
#[derive(Default)]
pub(crate) struct ServerDisplay {
    #[cfg(target_os = "linux")]
    running: std::sync::Mutex<Option<std::sync::Arc<running::Running>>>,
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

/// A server starts what a `moon launch` typed in one of its shells asks for on its desktop,
/// as it does what a window asks for - and the desktop first, when there is none.
impl crate::instances::StartsApplications for crate::api::AppState {
    fn start(&self, command: &str, folder: &Path) -> Result<String> {
        crate::api::mark_activity(&self.last_activity);
        let desktop = self.display.start_application(
            command,
            folder,
            VIEW_WITHOUT_A_WINDOW,
            SCALE_WITHOUT_A_WINDOW,
        )?;
        Ok(format!("the server's desktop, DISPLAY={}", desktop.name))
    }
}

#[cfg(not(target_os = "linux"))]
const ONLY_ON_LINUX: &str =
    "applications are started on an Xvfb display, which only a server on Linux has";

#[cfg(not(target_os = "linux"))]
impl ServerDisplay {
    pub(crate) fn start_application(
        &self,
        _command: &str,
        _folder: &Path,
        _view: [u16; 2],
        _scale: u8,
    ) -> Result<DisplayView> {
        anyhow::bail!(ONLY_ON_LINUX)
    }

    pub(crate) fn shown(&self) -> Option<DisplayView> {
        None
    }

    pub(crate) fn end(&self) {}
}

#[cfg(target_os = "linux")]
impl ServerDisplay {
    /// Start `command` on the desktop, run in `folder` - and the desktop first, with a view
    /// of `view` and applications drawn at `scale`, when there is none. Answered once
    /// a window opens on the desktop or the application ends - with an error when it ends in
    /// failure - and after [`started::SOON`] at the latest.
    pub(crate) fn start_application(
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
                Some(desktop) => std::sync::Arc::clone(desktop),
                None => {
                    let started = running::Running::start(view, scale)?;
                    *running = Some(std::sync::Arc::clone(&started));
                    started
                }
            }
        };
        // With the lock let go of: this waits to hear how the application's start went, and
        // whoever asks what the desktop is meanwhile is not kept waiting with it.
        desktop.start_application(command, folder)?;
        Ok(desktop.view.clone())
    }

    /// The desktop, when one is running.
    pub(crate) fn shown(&self) -> Option<DisplayView> {
        self.get().map(|desktop| desktop.view.clone())
    }

    /// End the desktop and every application on it.
    pub(crate) fn end(&self) {
        if let Some(desktop) = self
            .running
            .lock()
            .expect("the display lock is poisoned")
            .take()
        {
            desktop.end();
        }
    }

    fn get(&self) -> Option<std::sync::Arc<running::Running>> {
        self.running
            .lock()
            .expect("the display lock is poisoned")
            .as_ref()
            .filter(|desktop| !desktop.is_over())
            .cloned()
    }
}
