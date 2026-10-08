//! Running desktop - holds the Xvfb display `moon_display` started and the applications started
//! on it, and broadcasts its patches to every watcher.

use std::{
    collections::HashMap,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use tokio::sync::{broadcast, watch};

use crate::api::display::{DisplayInput, DisplayPatch, DisplayView};

use super::started::{self, Ended, Said, Started};

/// How many patches a watcher may fall behind by before it is shown the whole display in
/// their place - see `routes::watching`. A patch is a thirtieth of a second of the display,
/// so this is how stale what a slow socket shows may get before it is caught up.
const PATCHES_KEPT: usize = 4;

/// How long a desktop with nothing on it is left running before it is ended. An application
/// that hands itself over to another process is, for a moment, neither a program still running
/// nor a window yet open - and one that fails to start is that for good.
const EMPTY_FOR: Duration = Duration::from_secs(5);

/// What each toolkit reads the scale it draws at from, when it is started: GTK - and
/// Chromium, which asks GTK - and Qt. Whole numbers, which is all GTK takes.
const SCALED_BY: &[&str] = &["GDK_SCALE", "QT_SCALE_FACTOR"];

pub(super) struct Running {
    pub(super) view: DisplayView,
    /// The scale the desktop's applications draw at: the one of the window that started it.
    /// The desktop's own, since two applications on one screen at two scales is no screen.
    scale: u8,
    /// The display, until it is ended: dropping it is what stops Xvfb.
    display: Mutex<Option<moon_display::Display>>,
    occupied: Mutex<Occupied>,
    /// Told whenever a window opens or closes and whenever an application ends, which is
    /// what a start waits on to answer - see `Running::start_application`.
    occupancy_changed: Condvar,
    /// Every patch of the display, as the message its socket carries.
    patches: broadcast::Sender<Arc<Vec<u8>>>,
    /// Whether the desktop is over, for the sockets watching it to hang up on.
    over: watch::Sender<bool>,
}

/// What is on the desktop. With neither, there is nothing left to keep it for.
#[derive(Default)]
struct Occupied {
    /// The applications started here that have not ended, by the process group each was
    /// started as - its own, so that what it started ends with it.
    programs: Vec<u32>,
    /// How many windows are open.
    windows: usize,
    /// The applications whose start is still waited on - see `Running::start_application` -
    /// by process group, each with how it ended once it has.
    starting: HashMap<u32, Option<Ended>>,
}

impl Occupied {
    fn by_nothing(&self) -> bool {
        self.programs.is_empty() && self.windows == 0
    }
}

impl Running {
    /// Start the desktop, with a view of `view`, applications drawn at `scale`, and nothing
    /// on it yet.
    pub(super) fn start(view: [u16; 2], scale: u8) -> Result<Arc<Self>> {
        let (display, events) = moon_display::Display::start(moon_display::Size {
            width: view[0],
            height: view[1],
        })?;
        let (patches, _) = broadcast::channel(PATCHES_KEPT);
        let (over, _) = watch::channel(false);
        let running = Arc::new(Self {
            view: DisplayView {
                name: display.name(),
            },
            scale,
            display: Mutex::new(Some(display)),
            occupied: Mutex::default(),
            occupancy_changed: Condvar::new(),
            patches,
            over,
        });

        let watched = Arc::clone(&running);
        thread::Builder::new()
            .name(format!("moon display {}", running.view.name))
            .spawn(move || {
                // Until the display is gone, which closes the channel.
                for event in events {
                    match event {
                        // Nobody watching is no reason to stop: the next to watch is shown
                        // the display as it is - see `Running::watch`.
                        moon_display::Event::Drawn(patch) => {
                            let _ = watched.patches.send(Arc::new(message_of(patch)));
                        }
                        moon_display::Event::WindowsOpen(open) => {
                            watched.occupied().windows = open;
                            watched.occupancy_changed.notify_all();
                            watched.end_if_left_empty();
                        }
                    }
                }
                watched.end();
            })
            .context("the display's thread would not start")?;
        Ok(running)
    }

    /// Start `command` in `folder` with its windows going to this desktop, and answer once
    /// its start is known to have gone one way or the other: well when a window opens on the
    /// desktop or the program ends well, and with an error saying what it said when it ends
    /// in failure. One that has done none of these [`started::SOON`] after is taken to have
    /// started; one that opens a window and then fails is not reported.
    ///
    /// Any window opening after the start counts: whose it is, the display does not say.
    ///
    /// The answer is how a failure reaches the window that asked because it is the one
    /// message that window is waiting for, and already shows as an error: the desktop's
    /// socket is opened only after it and carries display patches and clipboard replies,
    /// rather than application launch failures.
    pub(super) fn start_application(self: &Arc<Self>, command: &str, folder: &Path) -> Result<()> {
        let program = Started::start(
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(folder)
                .env("DISPLAY", &self.view.name)
                .envs(SCALED_BY.iter().map(|read| (read, self.scale.to_string())))
                // A session of another kind would be found first by a toolkit that looks for
                // one.
                .env_remove("WAYLAND_DISPLAY")
                // The command is found where a shell tab of this server would find it, so
                // that the browser started from the menu is the one typed there by name.
                .env("PATH", crate::shell_path::installed_tools_path())
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null()),
            Said::KeptOnly,
        )
        .with_context(|| format!("`{command}` would not start"))?;
        let group = program.id();
        // Held from here until this is waiting below, so that neither a window opening nor
        // the program ending in between goes unheard.
        let mut occupied = self.occupied();
        occupied.programs.push(group);
        let windows_before = occupied.windows;

        let desktop = Arc::clone(self);
        program.wait(move |ended| {
            {
                let mut occupied = desktop.occupied();
                occupied.programs.retain(|running| *running != group);
                // Kept for a start still waiting to hear, and for no one otherwise.
                if let Some(how) = occupied.starting.get_mut(&group) {
                    *how = Some(ended);
                }
            }
            desktop.occupancy_changed.notify_all();
            desktop.end_if_left_empty();
        })?;
        occupied.starting.insert(group, None);
        let (mut occupied, _) = self
            .occupancy_changed
            .wait_timeout_while(occupied, started::SOON, |occupied| {
                occupied.windows <= windows_before && occupied.starting[&group].is_none()
            })
            .expect("the display's occupancy lock is poisoned");
        let ended = occupied
            .starting
            .remove(&group)
            .expect("a start is waited on until this takes it back");
        drop(occupied);
        match ended.and_then(|ended| ended.failure_of(command)) {
            Some(failure) => bail!(failure),
            None => Ok(()),
        }
    }

    /// Everything drawn on the display from now on, after the whole of it as it is now.
    pub(super) fn watch(&self) -> (Option<Vec<u8>>, broadcast::Receiver<Arc<Vec<u8>>>) {
        // Subscribed first, so nothing drawn in between is missed - a patch that then arrives
        // twice draws the same pixels twice.
        let patches = self.patches.subscribe();
        let shown = self
            .display
            .lock()
            .expect("the display lock is poisoned")
            .as_ref()
            .and_then(moon_display::Display::shown)
            .map(message_of);
        (shown, patches)
    }

    pub(super) fn over(&self) -> watch::Receiver<bool> {
        self.over.subscribe()
    }

    pub(super) fn is_over(&self) -> bool {
        *self.over.borrow()
    }

    pub(super) fn clipboard(
        &self,
        input: DisplayInput,
    ) -> std::sync::mpsc::Receiver<Result<Option<String>, String>> {
        let (reply, received) = std::sync::mpsc::channel();
        if let Some(display) = &*self.display.lock().expect("the display lock is poisoned") {
            let action = match input {
                DisplayInput::Paste { text, .. } => moon_display::ClipboardAction::Paste(text),
                DisplayInput::Copy { cut, .. } => moon_display::ClipboardAction::Copy { cut },
                _ => unreachable!("clipboard action"),
            };
            display.send(moon_display::Input::Clipboard(
                moon_display::ClipboardRequest { action, reply },
            ));
        }
        received
    }

    pub(super) fn did(&self, input: DisplayInput) {
        if let Some(display) = &*self.display.lock().expect("the display lock is poisoned") {
            display.send(input_of(input));
        }
    }

    /// Stop every application and the display. Asked twice - ended, and then found gone by
    /// the thread reading it - the second finds nothing left to stop.
    pub(super) fn end(&self) {
        for group in std::mem::take(&mut self.occupied().programs) {
            // The whole group: a browser is a dozen processes, and the shell that started it
            // is only the first.
            // SAFETY: `killpg` on a process group id and a signal touches no memory of ours.
            unsafe { libc::killpg(group as libc::pid_t, libc::SIGTERM) };
        }
        drop(
            self.display
                .lock()
                .expect("the display lock is poisoned")
                .take(),
        );
        self.over.send_replace(true);
    }

    /// End the desktop if nothing is on it now and nothing is in [`EMPTY_FOR`] from now.
    fn end_if_left_empty(self: &Arc<Self>) {
        if !self.occupied().by_nothing() || self.is_over() {
            return;
        }
        let desktop = Arc::clone(self);
        // A thread for the wait: this is asked by the threads that hear of a window closing
        // and of a program ending, neither of which may stop hearing for five seconds.
        let waiting = thread::Builder::new()
            .name(format!("moon display {} emptied", self.view.name))
            .spawn(move || {
                thread::sleep(EMPTY_FOR);
                if desktop.occupied().by_nothing() {
                    desktop.end();
                }
            });
        if waiting.is_err() {
            // No thread to wait on: a desktop left running empty is the lesser harm.
            eprintln!("[moonreview] could not wait to end an empty display");
        }
    }

    fn occupied(&self) -> std::sync::MutexGuard<'_, Occupied> {
        self.occupied
            .lock()
            .expect("the display's occupancy lock is poisoned")
    }
}

fn message_of(patch: moon_display::Patch) -> Vec<u8> {
    DisplayPatch {
        view: [patch.view.width, patch.view.height],
        at: [patch.x, patch.y],
        size: [patch.size.width, patch.size.height],
        rgb: patch.rgb,
    }
    .to_message()
}

fn input_of(input: DisplayInput) -> moon_display::Input {
    use moon_display::Input;

    match input {
        DisplayInput::Paste { .. } | DisplayInput::Copy { .. } => {
            unreachable!("clipboard actions have a reply")
        }
        DisplayInput::PointerMoved { x, y } => Input::PointerMoved { x, y },
        DisplayInput::Button { button, pressed } => Input::Button { button, pressed },
        DisplayInput::Scrolled { right, down } => Input::Scrolled { right, down },
        DisplayInput::Stroke { keysym } => Input::Stroke { keysym },
        DisplayInput::Typed { text } => Input::Typed(text),
        DisplayInput::Holding {
            shift,
            control,
            alt,
        } => Input::Holding(moon_display::Held {
            shift,
            control,
            alt,
        }),
        DisplayInput::Resized { width, height } => {
            Input::Resized(moon_display::Size { width, height })
        }
    }
}
