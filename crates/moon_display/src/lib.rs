//! Virtual X display - starts an Xvfb server, captures what programs draw on it, and injects
//! pointer and keyboard input.
//!
//! [`Display::start`] starts Xvfb, an X server that draws into memory, and a thread that
//! captures its screen. The captured pixels are sent as [`Patch`]es, along with the number of
//! open windows, on the channel `start` returns. [`Display::send`] moves the pointer and
//! presses keys on the display. Any program started with `DISPLAY` set to [`Display::name`]
//! opens its windows there.
//!
//! Xvfb cannot resize its screen once started, so the screen is created at [`LARGEST`], the
//! largest size a view can be, and only its top left corner is captured: the view, at the size
//! given by the last [`Input::Resized`]. A program's window is resized to exactly the view -
//! see `screen`, which is the display's window manager as well as its capture loop - so the
//! program lays itself out as if the view were the whole screen.
//!
//! A display from [`Display::start`] has no access control beyond X's default for a server
//! started by hand: anyone with an account on the machine can open a window on it, and read its
//! screen. One from [`Display::start_private`] is kept to one user of the machine: its server
//! runs as them, and lets in only a program that shows the cookie in a file they alone read.

mod clipboard;
mod keys;
mod screen;

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender},
    },
};

use anyhow::{Context, bail};

/// The largest a view of the display can be, which is the size its screen is started at.
pub const LARGEST: Size = Size {
    width: 3840,
    height: 2160,
};

/// The program that is the display: an X server with a screen in memory.
const XVFB: &str = "Xvfb";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

/// What a program shows a private display to be let onto it: sixteen bytes nobody can guess,
/// which whoever starts the display makes up.
pub type Cookie = [u8; 16];

/// What keeps a display to one user of the machine - see [`Display::start_private`].
pub struct Private {
    /// `sh`, as that user runs it, with nothing after it yet: the display's server is run
    /// under it, and so runs as them.
    pub shell: Command,
    /// The file the server reads who it lets in from, and a program of the user's is pointed
    /// at with `XAUTHORITY`. Written before the display is started, with [`authority_of`] the
    /// cookie in it, for that user alone to read.
    pub authority: PathBuf,
    /// The cookie in that file, which this process shows to connect to the display itself.
    pub cookie: Cookie,
}

/// The X authorization protocol in which a program is let in for showing a cookie.
pub(crate) const COOKIE_PROTOCOL: &[u8] = b"MIT-MAGIC-COOKIE-1";

/// The address family an authority file has for an entry that is about any address: Xlib's
/// `FamilyWild`.
const ANY_ADDRESS: u16 = 0xFFFF;

/// What an authority file holds for `cookie` to be what a display lets a program in for: one
/// entry, about any address and any display number.
///
/// The file is the display's alone, so there is no other display for the entry to be told
/// from, and its number is not known until the server reading the file has started. A server
/// takes every cookie in the file it is given, whatever the entry says it is about; a program
/// takes the entry for the display it is opening when the entry is about any.
///
/// An entry is its address family, then its address, display number, protocol and data, each
/// behind its length; every number is two bytes, most significant first.
pub fn authority_of(cookie: &Cookie) -> Vec<u8> {
    let (any_address, any_number): (&[u8], &[u8]) = (&[], &[]);
    let mut entry = ANY_ADDRESS.to_be_bytes().to_vec();
    for field in [any_address, any_number, COOKIE_PROTOCOL, cookie] {
        entry.extend((field.len() as u16).to_be_bytes());
        entry.extend(field);
    }
    entry
}

/// The modifier keys held down on the keyboard that is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
}

/// What a person watching the display did.
#[derive(Debug, Clone)]
pub enum Input {
    /// The pointer is here, in pixels from the top left of the view.
    PointerMoved { x: i16, y: i16 },
    /// A pointer button went down or came up, by its X number: 1 left, 2 middle, 3 right.
    Button { button: u8, pressed: bool },
    /// The wheel turned by this many clicks.
    Scrolled { right: i32, down: i32 },
    /// One key pressed and let go, by its X keysym - a key that types nothing, or a letter
    /// pressed for a chord with what is [`Input::Holding`].
    Stroke { keysym: u32 },
    /// Text typed, each character of it pressed and let go.
    Typed(String),
    /// Which modifier keys are down from now on.
    Holding(Held),
    /// The view is this size from now on.
    Resized(Size),
    /// An explicit clipboard action, answered only to its requester.
    Clipboard(ClipboardRequest),
}

/// Text clipboard action. Copy waits for a fresh selection; paste owns the selection before
/// issuing Ctrl+V. No clipboard is read in the absence of a request.
#[derive(Debug, Clone)]
pub struct ClipboardRequest {
    pub action: ClipboardAction,
    pub reply: Sender<Result<Option<String>, String>>,
}

#[derive(Debug, Clone)]
pub enum ClipboardAction {
    Paste(String),
    Copy { cut: bool },
}

/// Maximum text size accepted in either direction, in UTF-8 bytes.
pub const CLIPBOARD_LIMIT: usize = 64 * 1024;

/// A rectangle of the view that changed, as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// The size of the whole view, which a patch after a resize is the first to say.
    pub view: Size,
    pub x: u16,
    pub y: u16,
    pub size: Size,
    /// The rectangle's pixels, row after row, three bytes each: red, green, blue.
    pub rgb: Vec<u8>,
}

/// What happened on the display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Something was drawn: this rectangle of the view is as it is now.
    Drawn(Patch),
    /// A program opened a window, or one closed: this many are open now. None is a display
    /// with nothing on it - which it also is before its first program has opened anything.
    WindowsOpen(usize),
}

/// The view as it was last looked at - see [`Display::shown`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Shown {
    pub(crate) size: Option<Size>,
    /// Four bytes a pixel as X hands them out: blue, green, red, and one unused.
    pub(crate) bgrx: Vec<u8>,
}

pub struct Display {
    number: u32,
    server: Child,
    inputs: Sender<Input>,
    shown: Arc<Mutex<Shown>>,
}

impl Display {
    /// Start a display with a view of `size`, and watch it. The events are what happens
    /// there from now on; the channel closes when the display is gone.
    pub fn start(size: Size) -> anyhow::Result<(Self, Receiver<Event>)> {
        Self::start_kept_to(None, size)
    }

    /// Start a display as [`Display::start`] does, kept to one user of the machine: the
    /// server runs as them, and only a program that shows the cookie is let onto it - one of
    /// theirs, which reads it from the authority file, and this process, which holds it.
    pub fn start_private(size: Size, private: Private) -> anyhow::Result<(Self, Receiver<Event>)> {
        Self::start_kept_to(Some(private), size)
    }

    fn start_kept_to(
        private: Option<Private>,
        size: Size,
    ) -> anyhow::Result<(Self, Receiver<Event>)> {
        let cookie = private.as_ref().map(|private| private.cookie);
        let (number, server) = start_the_server(private)?;
        let (inputs, taking_inputs) = std::sync::mpsc::channel();
        let (saying, patches) = std::sync::mpsc::channel();
        let shown = Arc::new(Mutex::new(Shown::default()));
        screen::watch_on_a_thread(
            number,
            cookie,
            size,
            taking_inputs,
            saying,
            Arc::clone(&shown),
        )?;
        Ok((
            Self {
                number,
                server,
                inputs,
                shown,
            },
            patches,
        ))
    }

    /// The display's number, which no other display of the machine has while this one runs.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// What `DISPLAY` is set to for a program to open its windows here: `:7`.
    pub fn name(&self) -> String {
        format!(":{}", self.number)
    }

    pub fn send(&self, input: Input) {
        // A send that fails is a display that is gone, which the patches closing already said.
        let _ = self.inputs.send(input);
    }

    /// The whole view as it is now, as one patch: what somebody who starts watching is shown
    /// first. `None` until the display has been looked at once.
    pub fn shown(&self) -> Option<Patch> {
        let shown = self
            .shown
            .lock()
            .expect("the display's view lock is poisoned");
        let size = shown.size?;
        Some(Patch {
            view: size,
            x: 0,
            y: 0,
            size,
            rgb: screen::rgb_of(&shown.bgrx),
        })
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        // Letting go of the server's input is what ends it - see `UNTIL_NOBODY_HOLDS_IT` -
        // and the server going is what ends the thread watching it, and every program
        // drawing on it.
        drop(self.server.stdin.take());
        let _ = self.server.wait();
    }
}

/// The shell the server is run under, which ends it when nobody holds the shell's input any
/// more: `cat` reads that input until it closes, and then the server is stopped. The input is
/// a pipe from this process, so it closes when the display is dropped - and when this process
/// ends in any way at all, killed included, which nothing run at exit would be told of.
///
/// The server is the only one left holding the shell's output, so that it closing without a
/// number on it is the server having ended before it had a display.
const UNTIL_NOBODY_HOLDS_IT: &str = r#"exec 3>&1 >/dev/null; "$@" >&3 3>&- & server=$!; exec 3>&-; cat; kill "$server" 2>/dev/null"#;

/// Start Xvfb on whichever display number is free, which it says on the descriptor it is told
/// to - its own standard output, here - once it is ready for programs to connect.
///
/// A private display's server is run by the shell it was handed, and so as the user that
/// shell is run as, and is told the file to read who it lets in from. A server that reads a
/// cookie there lets in whoever shows it and nobody else: not the other users of the machine,
/// which one told of no such file lets in.
fn start_the_server(private: Option<Private>) -> anyhow::Result<(u32, Child)> {
    let (mut shell, authority) = match private {
        Some(private) => (private.shell, Some(private.authority)),
        None => (Command::new("sh"), None),
    };
    shell
        .args(["-c", UNTIL_NOBODY_HOLDS_IT, "moon-display", XVFB])
        .args(["-displayfd", "1"])
        .args([
            "-screen",
            "0",
            &format!("{}x{}x24", LARGEST.width, LARGEST.height),
        ])
        // Reached through its socket on this machine and no other way; and kept running when
        // the last program on it closes, which is otherwise when an X server starts over.
        .args(["-nolisten", "tcp", "-noreset"]);
    if let Some(authority) = &authority {
        shell.arg("-auth").arg(authority);
    }
    let mut server = shell
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("no shell to start the display's server with")?;
    let said = server.stdout.take().expect("the server's output is piped");
    let mut number = String::new();
    BufReader::new(said)
        .read_line(&mut number)
        .context("the display did not say which number it took")?;
    let Ok(number) = number.trim().parse() else {
        drop(server.stdin.take());
        let _ = server.wait();
        bail!("{XVFB} ended before it had a display to offer - a display needs it installed");
    };
    Ok((number, server))
}
