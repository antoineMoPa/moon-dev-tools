//! The X11 side: one connection, one thread, and the rules of being the session's window
//! manager.
//!
//! What makes a client "the window manager" is `SubstructureRedirect` on the root window.

use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, RecvTimeoutError, Sender},
    time::Duration,
};

use anyhow::{Context, anyhow};
use x11rb::{
    CURRENT_TIME,
    connection::Connection,
    protocol::{
        Event as XEvent,
        xproto::{
            AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux,
            ConnectionExt, CreateWindowAux, EventMask, GrabMode, InputFocus, ModMask, PropMode,
            StackMode, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{Bounds, Client, Command, Event};

/// How long the thread waits on the window's commands before looking at X again. X wakes it
/// early through its own socket only in a `wait_for_event`, which cannot also wait on a
/// channel - so both are polled, at a rate well under a frame.
const POLL: Duration = Duration::from_millis(4);

/// The keysym of `p`, which with control and shift is the one chord the launcher takes for
/// itself - see [`Session::grab_the_way_back`].
const KEYSYM_P: u32 = 0x70;

/// The modifiers that mean nothing about which chord was pressed and everything about what the
/// keyboard's lights are doing: caps lock, num lock, and the two together. A grab is for exact
/// modifiers, so the chord is grabbed once for each of them or it stops working the moment
/// somebody leaves caps lock on.
fn meaningless_modifiers() -> [u16; 4] {
    let lock = u16::from(ModMask::LOCK);
    let num_lock = u16::from(ModMask::M2);
    [0, lock, num_lock, lock | num_lock]
}

/// What `WM_STATE` says about a window that is managed and not withdrawn - ICCCM's
/// `NormalState`.
const NORMAL_STATE: u32 = 1;

/// Where a window sits while it has no pane: inside the container, but past its right edge.
const OFF_SCREEN: Bounds = Bounds {
    x: 10_000,
    y: 10_000,
    width: 640,
    height: 480,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        WM_STATE,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_ACTIVE_WINDOW,
        _NET_SUPPORTED,
        _NET_SUPPORTING_WM_CHECK,
        _NET_WM_STATE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_CLOSE_WINDOW,
        UTF8_STRING,
    }
}

/// A window this session is managing.
struct Managed {
    /// Where the window last went, so a frame that asks for the same place again costs nothing.
    bounds: Option<Bounds>,
    /// How many `UnmapNotify`s to let pass as this thread's own doing. Taking a pane off the
    /// screen unmaps its client, and X says so in the same way it says a window was withdrawn.
    unmaps_of_our_own: u32,
}

/// What the launcher calls when it has something to say, so that the window draws and reads it.
pub(crate) type Wake = Box<dyn Fn() + Send>;

pub(crate) fn run_on_a_thread(
    container: u32,
    commands: Receiver<Command>,
    events: Sender<Event>,
    wake: Wake,
) -> anyhow::Result<()> {
    let session = Session::take(container)?;
    std::thread::Builder::new()
        .name("moon launcher".to_string())
        .spawn(move || session.run(&commands, &Said { events, wake }))
        .context("the launcher's thread would not start")?;
    Ok(())
}

/// Where what happened goes: the window's channel, and the nudge that makes it draw and read
/// the channel.
struct Said {
    events: Sender<Event>,
    wake: Wake,
}

impl Said {
    fn say(&self, event: Event) {
        // A send that fails is a window that has gone; there is nothing to do about it here.
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

struct Session {
    conn: RustConnection,
    root: u32,
    container: u32,
    atoms: Atoms,
    managed: HashMap<Client, Managed>,
}

impl Session {
    /// Connect to the display the window is on and become its window manager.
    fn take(container: u32) -> anyhow::Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("no X display to manage")?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn)?.reply()?;

        // The one call that decides it: X gives substructure redirection to a single client, so
        // this is refused when a window manager is already running.
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY),
        )?
        .check()
        .map_err(|_| anyhow!("another window manager is already running on this display"))?;

        let session = Self {
            conn,
            root,
            container,
            atoms,
            managed: HashMap::new(),
        };
        session.say_there_is_a_window_manager()?;
        session.grab_the_way_back()?;
        session.conn.flush()?;
        Ok(session)
    }

    /// The properties a toolkit looks at to decide whether the session has a window manager at
    /// all. Chromium and GTK both go a different, worse way without them - drawing their own
    /// frames, or refusing to start maximized - so moon says plainly that it is one.
    fn say_there_is_a_window_manager(&self) -> anyhow::Result<()> {
        let checker = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            checker,
            self.root,
            -100,
            -100,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1),
        )?;
        for window in [checker, self.root] {
            self.conn.change_property32(
                PropMode::REPLACE,
                window,
                self.atoms._NET_SUPPORTING_WM_CHECK,
                AtomEnum::WINDOW,
                &[checker],
            )?;
        }
        self.conn.change_property8(
            PropMode::REPLACE,
            checker,
            self.atoms._NET_WM_NAME,
            self.atoms.UTF8_STRING,
            b"moon",
        )?;
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms._NET_SUPPORTED,
            AtomEnum::ATOM,
            &[
                self.atoms._NET_SUPPORTING_WM_CHECK,
                self.atoms._NET_ACTIVE_WINDOW,
                self.atoms._NET_WM_NAME,
                self.atoms._NET_CLOSE_WINDOW,
            ],
        )?;
        Ok(())
    }

    /// The one chord the launcher keeps for itself, whatever has the keyboard: the palette's.
    ///
    /// A grab on the root window is how a window manager keeps a chord of its own: X hands this
    /// one here instead, wherever the keyboard was pointed.
    fn grab_the_way_back(&self) -> anyhow::Result<()> {
        let setup = self.conn.setup();
        let mapping = self
            .conn
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)?
            .reply()?;
        let per_keycode = mapping.keysyms_per_keycode as usize;

        for (at, keysyms) in mapping.keysyms.chunks(per_keycode).enumerate() {
            if !keysyms.contains(&KEYSYM_P) {
                continue;
            }
            let keycode = setup.min_keycode + at as u8;
            for meaningless in meaningless_modifiers() {
                self.conn.grab_key(
                    // The key press comes here and nowhere else.
                    false,
                    self.root,
                    ModMask::from(
                        u16::from(ModMask::CONTROL) | u16::from(ModMask::SHIFT) | meaningless,
                    ),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?;
            }
        }
        Ok(())
    }

    fn run(mut self, commands: &Receiver<Command>, events: &Said) {
        loop {
            match self.step(commands, events) {
                Ok(true) => {}
                // Stopped, or X went away with the session.
                Ok(false) | Err(_) => return,
            }
        }
    }

    /// One turn of the loop: everything X has said, then everything the window has asked for.
    /// `false` when the session is over.
    fn step(&mut self, commands: &Receiver<Command>, events: &Said) -> anyhow::Result<bool> {
        while let Some(event) = self.conn.poll_for_event()? {
            self.heard(event, events)?;
        }
        match commands.recv_timeout(POLL) {
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => return Ok(false),
            Ok(command) => self.asked(command)?,
            Err(RecvTimeoutError::Timeout) => {}
        }
        self.conn.flush()?;
        Ok(true)
    }

    fn heard(&mut self, event: XEvent, events: &Said) -> anyhow::Result<()> {
        match event {
            // A program wants its window on the screen. It goes inside moon's window instead,
            // off screen, until a pane is opened for it.
            XEvent::MapRequest(request) => self.adopt(request.window, events)?,
            XEvent::DestroyNotify(notify) => {
                if self.managed.remove(&notify.window).is_some() {
                    events.say(Event::Gone {
                        client: notify.window,
                    });
                }
            }
            // Either the program withdrew its window, or this thread took the pane off the
            // screen a moment ago - and X says both the same way.
            XEvent::UnmapNotify(notify) => {
                if let Some(managed) = self.managed.get_mut(&notify.window) {
                    if managed.unmaps_of_our_own > 0 {
                        managed.unmaps_of_our_own -= 1;
                    } else {
                        self.managed.remove(&notify.window);
                        events.say(Event::Gone {
                            client: notify.window,
                        });
                    }
                }
            }
            // A window asking to be moved or resized. A managed one is told again what the
            // pane says; the answer is never the program's own idea of its size.
            XEvent::ConfigureRequest(request) => {
                match self.managed.get(&request.window).map(|held| held.bounds) {
                    Some(bounds) => self.put(request.window, bounds.unwrap_or(OFF_SCREEN))?,
                    None => {
                        self.conn.configure_window(
                            request.window,
                            &ConfigureWindowAux::from_configure_request(&request),
                        )?;
                    }
                }
            }
            // The one chord that is the launcher's own - see `grab_the_way_back`. Whatever the
            // keyboard was pointed at, this is the way back to the window.
            XEvent::KeyPress(_) => events.say(Event::AskedForTheWindow),
            XEvent::PropertyNotify(notify) => {
                let renamed = notify.atom == self.atoms._NET_WM_NAME
                    || notify.atom == u32::from(AtomEnum::WM_NAME);
                if renamed && self.managed.contains_key(&notify.window) {
                    events.say(Event::Retitled {
                        client: notify.window,
                        title: self.title_of(notify.window),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Take a window into the session: make it a child of moon's window, off screen, and say
    /// so. It is mapped only once a pane has told it where to go.
    fn adopt(&mut self, client: Client, events: &Said) -> anyhow::Result<()> {
        // moon's own window is never put in one of its own panes.
        if client == self.container {
            self.conn.map_window(client)?;
            return Ok(());
        }
        if self.managed.contains_key(&client) {
            return Ok(());
        }
        self.conn.change_window_attributes(
            client,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )?;
        self.conn.reparent_window(
            client,
            self.container,
            OFF_SCREEN.x as i16,
            OFF_SCREEN.y as i16,
        )?;
        self.managed.insert(
            client,
            Managed {
                bounds: None,
                unmaps_of_our_own: 0,
            },
        );
        self.put(client, OFF_SCREEN)?;
        // A toolkit that waits to be told it is managed sits and waits without `WM_STATE`.
        self.conn.change_property32(
            PropMode::REPLACE,
            client,
            self.atoms.WM_STATE,
            self.atoms.WM_STATE,
            &[NORMAL_STATE, x11rb::NONE],
        )?;
        self.conn.flush()?;
        events.say(Event::Appeared {
            client,
            title: self.title_of(client),
            program: self.program_of(client),
        });
        Ok(())
    }

    fn asked(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::Place { client, bounds } => self.place(client, bounds)?,
            Command::Focus { client } => {
                if self.managed.contains_key(&client) {
                    self.conn
                        .set_input_focus(InputFocus::PARENT, client, CURRENT_TIME)?;
                    self.conn.change_property32(
                        PropMode::REPLACE,
                        self.root,
                        self.atoms._NET_ACTIVE_WINDOW,
                        AtomEnum::WINDOW,
                        &[client],
                    )?;
                }
            }
            Command::TakeBackTheKeyboard => {
                self.conn
                    .set_input_focus(InputFocus::PARENT, self.container, CURRENT_TIME)?;
            }
            Command::Close { client } => self.ask_it_to_close(client)?,
            Command::Stop => {}
        }
        Ok(())
    }

    /// Where a client goes. `None` takes it off the screen - a pane in a tab that is not in
    /// front, or a window whose pane has been closed.
    fn place(&mut self, client: Client, bounds: Option<Bounds>) -> anyhow::Result<()> {
        let Some(managed) = self.managed.get_mut(&client) else {
            return Ok(());
        };
        if managed.bounds == bounds {
            return Ok(());
        }
        let was_shown = managed.bounds.is_some();
        managed.bounds = bounds;
        match bounds {
            Some(bounds) => {
                self.put(client, bounds)?;
                if !was_shown {
                    self.conn.map_window(client)?;
                }
            }
            None => {
                if was_shown {
                    // The unmap this causes is this thread's doing, not the program's.
                    if let Some(managed) = self.managed.get_mut(&client) {
                        managed.unmaps_of_our_own += 1;
                    }
                    self.conn.unmap_window(client)?;
                }
            }
        }
        Ok(())
    }

    fn put(&self, client: Client, bounds: Bounds) -> anyhow::Result<()> {
        self.conn.configure_window(
            client,
            &ConfigureWindowAux::new()
                .x(bounds.x)
                .y(bounds.y)
                .width(bounds.width.max(1))
                .height(bounds.height.max(1))
                .border_width(0)
                .stack_mode(StackMode::ABOVE),
        )?;
        Ok(())
    }

    /// The way a title bar's × asks: `WM_DELETE_WINDOW` when the program says it understands
    /// it, so a browser gets to ask about the form you were typing in. A program that does not
    /// is killed, which is what every window manager does with it.
    fn ask_it_to_close(&self, client: Client) -> anyhow::Result<()> {
        let understands = self
            .conn
            .get_property(
                false,
                client,
                self.atoms.WM_PROTOCOLS,
                AtomEnum::ATOM,
                0,
                u32::MAX,
            )?
            .reply()
            .ok()
            .and_then(|property| property.value32().map(|atoms| atoms.collect::<Vec<_>>()))
            .is_some_and(|atoms| atoms.contains(&self.atoms.WM_DELETE_WINDOW));

        if understands {
            let asking = ClientMessageEvent::new(
                32,
                client,
                self.atoms.WM_PROTOCOLS,
                [self.atoms.WM_DELETE_WINDOW, CURRENT_TIME, 0, 0, 0],
            );
            self.conn
                .send_event(false, client, EventMask::NO_EVENT, asking)?;
        } else {
            self.conn.kill_client(client)?;
        }
        Ok(())
    }

    /// What a window calls itself: `_NET_WM_NAME` for anything written this century, and
    /// `WM_NAME` for the rest.
    fn title_of(&self, client: Client) -> String {
        for (atom, kind) in [
            (self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING),
            (u32::from(AtomEnum::WM_NAME), u32::from(AtomEnum::STRING)),
        ] {
            let said = self
                .conn
                .get_property(false, client, atom, kind, 0, 1024)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|property| String::from_utf8_lossy(&property.value).to_string())
                .unwrap_or_default();
            if !said.is_empty() {
                return said;
            }
        }
        String::new()
    }

    /// The program that opened the window, out of `WM_CLASS` - two strings, the instance and
    /// the class, of which the instance is the closer to the command that was run.
    fn program_of(&self, client: Client) -> String {
        self.conn
            .get_property(
                false,
                client,
                u32::from(AtomEnum::WM_CLASS),
                u32::from(AtomEnum::STRING),
                0,
                256,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|property| {
                String::from_utf8_lossy(&property.value)
                    .split('\0')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_default()
    }
}
