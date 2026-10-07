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
//! Only a server on Linux has a desktop: the display is Xvfb, which is installed on Linux.

mod routes;
#[cfg(target_os = "linux")]
mod running;

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
    /// of `view` and applications drawn at `scale`, when there is none.
    pub(crate) fn start_application(
        &self,
        command: &str,
        folder: &Path,
        view: [u16; 2],
        scale: u8,
    ) -> Result<DisplayView> {
        let mut running = self.running.lock().expect("the display lock is poisoned");
        // One whose applications have all ended is over, and only not yet let go of.
        let desktop = match running.as_ref().filter(|desktop| !desktop.is_over()) {
            Some(desktop) => std::sync::Arc::clone(desktop),
            None => {
                let started = running::Running::start(view, scale)?;
                *running = Some(std::sync::Arc::clone(&started));
                started
            }
        };
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
