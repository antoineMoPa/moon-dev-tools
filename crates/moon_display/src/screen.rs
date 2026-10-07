//! Window management and screen capture - one thread with one X connection.
//!
//! One connection and one thread, like `moon_launcher`'s, and for the same reason: a window
//! manager's requests and the X events it handles must not be interleaved with anything else.
//!
//! The window manager has one rule. A window a program opens is resized to fill the view - see
//! the crate's own comment for why the view is not the whole screen - and the newest window
//! gets keyboard focus. A window that sets `WM_TRANSIENT_FOR`, a dialog, keeps the size and
//! position it asked for. Menus and tooltips are not managed at all.
//!
//! Screen capture waits for the DAMAGE extension to report that something was drawn, then reads
//! the whole view with GetImage, at most once every [`BETWEEN_LOOKS`], and sends what differs
//! from the previous capture: the one rectangle that bounds every changed pixel.

use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow, bail};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event as XEvent,
        damage::{ConnectionExt as _, ReportLevel},
        xproto::{
            AtomEnum, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ChangeWindowAttributesAux,
            ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask, ImageFormat, ImageOrder,
            InputFocus, MOTION_NOTIFY_EVENT, PropMode, StackMode, WindowClass,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{Event, Input, LARGEST, Patch, Shown, Size, keys::Keyboard};

/// How long the thread waits on what a person did before looking at X again - see
/// `moon_launcher`, whose loop this is.
const POLL: Duration = Duration::from_millis(4);

/// The least time between two looks at the view: thirty a second, which is as often as
/// anything drawn there is worth sending anywhere.
const BETWEEN_LOOKS: Duration = Duration::from_millis(33);

/// How X hands out the pixels of a screen 24 bits deep: four bytes each.
const BYTES_A_PIXEL: usize = 4;

/// The pointer buttons a wheel is, to X: a click of it is one of these pressed and let go.
const WHEEL_UP: u8 = 4;
const WHEEL_DOWN: u8 = 5;
const WHEEL_LEFT: u8 = 6;
const WHEEL_RIGHT: u8 = 7;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_SUPPORTED,
        _NET_SUPPORTING_WM_CHECK,
        _NET_WM_NAME,
        _NET_ACTIVE_WINDOW,
        _NET_WORKAREA,
        _NET_DESKTOP_GEOMETRY,
        _NET_NUMBER_OF_DESKTOPS,
        _NET_CURRENT_DESKTOP,
        UTF8_STRING,
    }
}

/// A window a program opened.
struct Managed {
    window: u32,
    /// Whether it is kept the size of the view, which a dialog is not.
    fills_the_view: bool,
}

/// A rectangle of the view, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

pub(crate) fn watch_on_a_thread(
    number: u32,
    view: Size,
    inputs: Receiver<Input>,
    events: Sender<Event>,
    shown: Arc<Mutex<Shown>>,
) -> anyhow::Result<()> {
    let watcher = Watcher::take(number, view, shown)?;
    std::thread::Builder::new()
        .name("moon display".to_string())
        .spawn(move || watcher.run(&inputs, &events))
        .context("the display's thread would not start")?;
    Ok(())
}

struct Watcher {
    conn: RustConnection,
    root: u32,
    atoms: Atoms,
    keyboard: Keyboard,
    damage: u32,
    view: Size,
    /// In the order they were opened: the last is in front, and has the keyboard.
    windows: Vec<Managed>,
    shown: Arc<Mutex<Shown>>,
    /// Whether anything was drawn since the last look.
    drawn_on: bool,
    looked: Instant,
    /// Whether a window opened or closed since it was last said how many are open.
    windows_changed: bool,
}

impl Watcher {
    /// Connect to the display and become its window manager.
    fn take(number: u32, view: Size, shown: Arc<Mutex<Shown>>) -> anyhow::Result<Self> {
        let (conn, screen) = x11rb::connect(Some(&format!(":{number}")))
            .with_context(|| format!("the display :{number} would not be connected to"))?;
        let setup = conn.setup();
        let root = setup.roots[screen].root;
        let depth = setup.roots[screen].root_depth;
        // What `look` reads the pixels as. Xvfb is started with a screen that is, and a
        // display that is not would be drawn in the wrong colors rather than not at all.
        let four_bytes_blue_first = setup.image_byte_order == ImageOrder::LSB_FIRST
            && setup.pixmap_formats.iter().any(|format| {
                format.depth == depth && usize::from(format.bits_per_pixel) == BYTES_A_PIXEL * 8
            });
        if depth != 24 || !four_bytes_blue_first {
            bail!("the display's pixels are not 24 bits in 4 bytes, blue first");
        }

        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY),
        )?
        .check()
        .map_err(|_| anyhow!("the display :{number} already has a window manager"))?;

        conn.damage_query_version(1, 1)?
            .reply()
            .context("the display has no DAMAGE extension to say what was drawn")?;
        let damage = conn.generate_id()?;
        // One word that something was drawn, and no more until it has been looked at.
        conn.damage_create(damage, root, ReportLevel::NON_EMPTY)?;

        let atoms = Atoms::new(&conn)?.reply()?;
        let keyboard = Keyboard::read(&conn, root)?;
        let watcher = Self {
            conn,
            root,
            atoms,
            keyboard,
            damage,
            view: within_the_screen(view),
            windows: Vec::new(),
            shown,
            // The first look is of a display nothing has drawn on yet: its background.
            drawn_on: true,
            looked: Instant::now() - BETWEEN_LOOKS,
            windows_changed: false,
        };
        watcher.say_there_is_a_window_manager()?;
        watcher.say_how_large_the_view_is()?;
        watcher.conn.flush()?;
        Ok(watcher)
    }

    /// See `moon_launcher`: Chromium and GTK go a different, worse way on a display that does
    /// not say it has a window manager.
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
                self.atoms._NET_WORKAREA,
                self.atoms._NET_DESKTOP_GEOMETRY,
            ],
        )?;
        for (property, value) in [
            (self.atoms._NET_NUMBER_OF_DESKTOPS, 1),
            (self.atoms._NET_CURRENT_DESKTOP, 0),
        ] {
            self.conn.change_property32(
                PropMode::REPLACE,
                self.root,
                property,
                AtomEnum::CARDINAL,
                &[value],
            )?;
        }
        Ok(())
    }

    /// Tell the programs where they may put what they open: within the view, which is all of
    /// the screen anybody sees. A menu opened near the view's edge is kept inside it by this.
    fn say_how_large_the_view_is(&self) -> anyhow::Result<()> {
        let (width, height) = (u32::from(self.view.width), u32::from(self.view.height));
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms._NET_WORKAREA,
            AtomEnum::CARDINAL,
            &[0, 0, width, height],
        )?;
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms._NET_DESKTOP_GEOMETRY,
            AtomEnum::CARDINAL,
            &[width, height],
        )?;
        Ok(())
    }

    fn run(mut self, inputs: &Receiver<Input>, events: &Sender<Event>) {
        loop {
            match self.step(inputs, events) {
                Ok(true) => {}
                // Nobody is watching any more, or the display went away.
                Ok(false) | Err(_) => return,
            }
        }
    }

    /// One turn of the loop: everything X has said, then what a person did, then a look at
    /// the view when it is due one. `false` when there is nobody left to do it for.
    fn step(&mut self, inputs: &Receiver<Input>, events: &Sender<Event>) -> anyhow::Result<bool> {
        while let Some(event) = self.conn.poll_for_event()? {
            self.heard(event)?;
        }
        match inputs.recv_timeout(POLL) {
            Ok(input) => self.did(input)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(false),
        }
        self.conn.flush()?;
        if std::mem::take(&mut self.windows_changed)
            && events.send(Event::WindowsOpen(self.windows.len())).is_err()
        {
            return Ok(false);
        }
        if self.drawn_on && self.looked.elapsed() >= BETWEEN_LOOKS {
            self.drawn_on = false;
            self.looked = Instant::now();
            if let Some(patch) = self.look()? {
                return Ok(events.send(Event::Drawn(patch)).is_ok());
            }
        }
        Ok(true)
    }

    fn heard(&mut self, event: XEvent) -> anyhow::Result<()> {
        match event {
            XEvent::DamageNotify(_) => self.drawn_on = true,
            XEvent::MapRequest(request) => self.adopt(request.window)?,
            XEvent::DestroyNotify(notify) => self.forget(notify.window)?,
            XEvent::UnmapNotify(notify) => self.forget(notify.window)?,
            // A window asking to be moved or resized. One that fills the view is told the view
            // again; any other gets what it asked for.
            XEvent::ConfigureRequest(request) => {
                let fills_the_view = self
                    .windows
                    .iter()
                    .any(|held| held.window == request.window && held.fills_the_view);
                match fills_the_view {
                    true => self.fill_the_view_with(request.window)?,
                    false => {
                        self.conn.configure_window(
                            request.window,
                            &ConfigureWindowAux::from_configure_request(&request),
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A program wants a window on the screen: it is put there, in front, with the keyboard.
    fn adopt(&mut self, window: u32) -> anyhow::Result<()> {
        if !self.windows.iter().any(|held| held.window == window) {
            let belongs_to_another = self
                .conn
                .get_property(
                    false,
                    window,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    0,
                    1,
                )?
                .reply()
                .is_ok_and(|property| property.value_len > 0);
            self.windows.push(Managed {
                window,
                fills_the_view: !belongs_to_another,
            });
            self.windows_changed = true;
            if !belongs_to_another {
                self.fill_the_view_with(window)?;
            }
        }
        self.conn.map_window(window)?;
        self.bring_forward(window)
    }

    /// A window closed, or was withdrawn: the one opened before it has the keyboard again.
    fn forget(&mut self, window: u32) -> anyhow::Result<()> {
        let before = self.windows.len();
        self.windows.retain(|held| held.window != window);
        if self.windows.len() == before {
            return Ok(());
        }
        self.windows_changed = true;
        match self.windows.last() {
            Some(held) => self.bring_forward(held.window),
            None => {
                self.conn.set_input_focus(
                    InputFocus::POINTER_ROOT,
                    InputFocus::POINTER_ROOT,
                    CURRENT_TIME,
                )?;
                Ok(())
            }
        }
    }

    fn fill_the_view_with(&self, window: u32) -> anyhow::Result<()> {
        self.conn.configure_window(
            window,
            &ConfigureWindowAux::new()
                .x(0)
                .y(0)
                .width(u32::from(self.view.width))
                .height(u32::from(self.view.height))
                .border_width(0),
        )?;
        Ok(())
    }

    fn bring_forward(&self, window: u32) -> anyhow::Result<()> {
        self.conn.configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?;
        self.conn
            .set_input_focus(InputFocus::POINTER_ROOT, window, CURRENT_TIME)?;
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            &[window],
        )?;
        Ok(())
    }

    fn did(&mut self, input: Input) -> anyhow::Result<()> {
        match input {
            Input::PointerMoved { x, y } => {
                // Past the view is screen nobody sees, where a pointer would be lost.
                let x = x.clamp(0, self.view.width as i16 - 1);
                let y = y.clamp(0, self.view.height as i16 - 1);
                self.conn.xtest_fake_input(
                    MOTION_NOTIFY_EVENT,
                    0,
                    CURRENT_TIME,
                    self.root,
                    x,
                    y,
                    0,
                )?;
            }
            Input::Button { button, pressed } => self.button(button, pressed)?,
            Input::Scrolled { right, down } => {
                for (clicks, forward, backward) in [
                    (right, WHEEL_RIGHT, WHEEL_LEFT),
                    (down, WHEEL_DOWN, WHEEL_UP),
                ] {
                    let button = if clicks > 0 { forward } else { backward };
                    for _ in 0..clicks.unsigned_abs() {
                        self.button(button, true)?;
                        self.button(button, false)?;
                    }
                }
            }
            Input::Stroke { keysym } => self.keyboard.stroke(&self.conn, keysym)?,
            Input::Typed(text) => self.keyboard.type_text(&self.conn, &text)?,
            Input::Holding(held) => self.keyboard.hold(&self.conn, held)?,
            Input::Resized(view) => {
                self.view = within_the_screen(view);
                self.say_how_large_the_view_is()?;
                for held in self.windows.iter().filter(|held| held.fills_the_view) {
                    self.fill_the_view_with(held.window)?;
                }
                // Nothing may have been drawn, and the view is still another one.
                self.drawn_on = true;
            }
        }
        Ok(())
    }

    fn button(&self, button: u8, pressed: bool) -> anyhow::Result<()> {
        let event = if pressed {
            BUTTON_PRESS_EVENT
        } else {
            BUTTON_RELEASE_EVENT
        };
        self.conn
            .xtest_fake_input(event, button, CURRENT_TIME, self.root, 0, 0, 0)?;
        Ok(())
    }

    /// Read the view off the screen, and say what of it is not as it was at the last look -
    /// all of it, when the view was another size then.
    fn look(&mut self) -> anyhow::Result<Option<Patch>> {
        // Before the pixels are read: whatever is drawn while they are is looked at next time.
        self.conn.damage_subtract(self.damage, NONE, NONE)?;
        let now = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                self.view.width,
                self.view.height,
                !0,
            )?
            .reply()?
            .data;

        let mut shown = self
            .shown
            .lock()
            .expect("the display's view lock is poisoned");
        let whole = Rect {
            x: 0,
            y: 0,
            width: usize::from(self.view.width),
            height: usize::from(self.view.height),
        };
        let changed = match shown.size == Some(self.view) {
            true => changed_between(&shown.bgrx, &now, whole.width),
            false => Some(whole),
        };
        shown.size = Some(self.view);
        shown.bgrx = now;
        Ok(changed.map(|rect| Patch {
            view: self.view,
            x: rect.x as u16,
            y: rect.y as u16,
            size: Size {
                width: rect.width as u16,
                height: rect.height as u16,
            },
            rgb: rgb_of_rect(&shown.bgrx, whole.width, rect),
        }))
    }
}

fn within_the_screen(view: Size) -> Size {
    Size {
        width: view.width.clamp(1, LARGEST.width),
        height: view.height.clamp(1, LARGEST.height),
    }
}

/// The one rectangle around every pixel that differs between two looks at a view `width`
/// pixels wide. `None` when they are the same.
fn changed_between(before: &[u8], now: &[u8], width: usize) -> Option<Rect> {
    let row = width * BYTES_A_PIXEL;
    let rows = || before.chunks_exact(row).zip(now.chunks_exact(row));
    let top = rows().position(|(before, now)| before != now)?;
    let bottom = rows()
        .rposition(|(before, now)| before != now)
        .expect("a row differs, since one was found from the top");

    let (mut left, mut right) = (width, 0);
    for (before, now) in rows().skip(top).take(bottom - top + 1) {
        let pixels = || {
            before
                .chunks_exact(BYTES_A_PIXEL)
                .zip(now.chunks_exact(BYTES_A_PIXEL))
        };
        let Some(first) = pixels().position(|(before, now)| before != now) else {
            continue;
        };
        let last = pixels()
            .rposition(|(before, now)| before != now)
            .expect("a pixel differs, since one was found from the left");
        left = left.min(first);
        right = right.max(last);
    }
    Some(Rect {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    })
}

/// A whole view's pixels, as [`Patch::rgb`] has them.
pub(crate) fn rgb_of(bgrx: &[u8]) -> Vec<u8> {
    bgrx.chunks_exact(BYTES_A_PIXEL)
        .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
        .collect()
}

/// The pixels of one rectangle of a view `width` pixels wide, as [`Patch::rgb`] has them.
fn rgb_of_rect(bgrx: &[u8], width: usize, rect: Rect) -> Vec<u8> {
    bgrx.chunks_exact(width * BYTES_A_PIXEL)
        .skip(rect.y)
        .take(rect.height)
        .flat_map(|row| rgb_of(&row[rect.x * BYTES_A_PIXEL..(rect.x + rect.width) * BYTES_A_PIXEL]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view of one color, `width` by `height`.
    fn view(width: usize, height: usize) -> Vec<u8> {
        vec![0; width * height * BYTES_A_PIXEL]
    }

    fn paint(view: &mut [u8], width: usize, x: usize, y: usize, bgr: [u8; 3]) {
        let at = (y * width + x) * BYTES_A_PIXEL;
        view[at..at + 3].copy_from_slice(&bgr);
    }

    #[test]
    fn what_changed_is_the_rectangle_around_every_pixel_that_did() {
        let before = view(8, 6);
        assert_eq!(changed_between(&before, &before, 8), None);

        let mut now = before.clone();
        paint(&mut now, 8, 2, 1, [1, 2, 3]);
        paint(&mut now, 8, 5, 4, [4, 5, 6]);
        let changed = changed_between(&before, &now, 8).expect("two pixels changed");
        assert_eq!(
            changed,
            Rect {
                x: 2,
                y: 1,
                width: 4,
                height: 4
            }
        );

        // Red, green, blue out of blue, green, red - and only the rectangle's own pixels.
        let rgb = rgb_of_rect(&now, 8, changed);
        assert_eq!(rgb.len(), 4 * 4 * 3);
        assert_eq!(rgb[..3], [3, 2, 1]);
        assert_eq!(rgb[rgb.len() - 3..], [6, 5, 4]);
    }
}
