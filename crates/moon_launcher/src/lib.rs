//! moon window manager

mod wm;

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// A window belonging to another program, by its X id.
pub type Client = u32;

/// Where a client window goes, in physical pixels from the top left of moon's own window - the
/// same corner and the same pixels as the pane it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// What the client windows of the session have done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A program opened a window. It is off screen until [`Launcher::place`] says where it
    /// goes, so the pane it belongs to is opened first and the window follows the pane.
    Appeared {
        client: Client,
        /// What the window calls itself, for the tab.
        title: String,
        /// The program that opened it, out of `WM_CLASS` - `chromium`, `xeyes`.
        program: String,
    },
    /// The window is gone: the program closed it, or ended.
    Gone { client: Client },
    /// The window renamed itself, which a browser does on every page.
    Retitled { client: Client, title: String },
    /// The chord the launcher keeps for itself was pressed - the palette's.
    AskedForTheWindow,
}

/// What the window asks of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// Where a client goes, or `None` to take it off the screen - a pane in a tab that is not
    /// in front, or a window that has no pane yet.
    Place {
        client: Client,
        bounds: Option<Bounds>,
    },
    /// Give this client the keyboard.
    Focus {
        client: Client,
    },
    /// Ask the program to close the window, the way a title bar's × would.
    Close {
        client: Client,
    },
    TakeBackTheKeyboard,
    Stop,
}

/// The window manager of this X session, running on a thread of its own.
pub struct Launcher {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl Launcher {
    /// Take over the session, putting every window that opens from now on inside `container` -
    /// moon's own window.
    ///
    /// `wake` is called whenever something has happened to say: a window opens when a program
    /// feels like opening one, not when the window next draws, and the caller only reads
    /// [`Launcher::events`] as it draws. Without being woken it would draw next when somebody
    /// moved the mouse, and a program started from the palette would sit off screen until then.
    ///
    /// This fails when another window manager already holds the session: X gives
    /// `SubstructureRedirect` on the root window to one client at a time, which is what makes
    /// "the window manager" a thing there is only one of.
    pub fn start(container: u32, wake: impl Fn() + Send + 'static) -> anyhow::Result<Self> {
        let (commands, taking_commands) = std::sync::mpsc::channel();
        let (saying, events) = std::sync::mpsc::channel();
        wm::run_on_a_thread(container, taking_commands, saying, Box::new(wake))?;
        Ok(Self { commands, events })
    }

    /// What the session's windows have done since this was last asked. Nothing is waited for:
    /// this is read once a frame.
    pub fn events(&self) -> Vec<Event> {
        let mut heard = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => heard.push(event),
                // The thread is gone - `Gone` was already said for every client it held.
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return heard,
            }
        }
    }

    /// Where a client's window goes, or `None` to take it off the screen.
    ///
    /// The window is only mapped once it has bounds, so a program whose window has no pane yet
    /// is never seen in the wrong place first.
    pub fn place(&self, client: Client, bounds: Option<Bounds>) {
        self.send(Command::Place { client, bounds });
    }

    /// Give a client the keyboard. The window keeps drawing its own focus either way; this is
    /// what makes typing go to it.
    pub fn focus(&self, client: Client) {
        self.send(Command::Focus { client });
    }

    /// Give the keyboard back to moon's own window - what closing or leaving an application
    /// pane does.
    pub fn take_back_the_keyboard(&self) {
        self.send(Command::TakeBackTheKeyboard);
    }

    /// Ask the program to close this window. It may refuse, or ask its own question first,
    /// exactly as it would with any other window manager.
    pub fn close(&self, client: Client) {
        self.send(Command::Close { client });
    }

    fn send(&self, command: Command) {
        // A send that fails is a thread that has ended, which has already been said in the
        // events; there is nothing for a caller to do about it.
        let _ = self.commands.send(command);
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        self.send(Command::Stop);
    }
}
