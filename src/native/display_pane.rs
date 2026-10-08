//! Desktop pane - shows the server's desktop: a virtual X screen on the server, with the
//! applications started on it. The pane draws their windows and sends them pointer and keyboard
//! input. See `crate::display` for the server side and [`crate::api::display`] for the
//! websocket between the two.
//!
//! The desktop is drawn by egui, as a texture: the server sends the rectangles that changed and
//! the pane paints them into it. So it behaves like any other pane - the palette opens over it,
//! it can sit behind another tab - in a browser's window and in a native one alike.
//!
//! One pixel of the desktop is drawn as one physical pixel of the screen this window is on -
//! not one point, which on most screens is not a whole number of pixels: a picture scaled by
//! any other factor has its rows and columns doubled or dropped unevenly. The desktop's view is
//! kept at the size of the pane in physical pixels, so the applications on it see a screen of
//! that size and lay themselves out for it. The desktop is started with the scale of the window
//! that starts it, so that applications draw their text as large as this window draws its own.
//!
//! The desktop exists from the moment an application is started - from `moon › Applications`,
//! or the palette - until its last application exits, and so does the pane: it opens with the
//! first application and closes after the last. The desktop belongs to the server, like a
//! shell, so closing the pane leaves it running, and a window opened while a desktop is running
//! shows it. That lets an agent on the server and a person at this window use the same browser.

mod clipboard;
mod keys;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, mpsc::TryRecvError},
    time::Duration,
};

use egui::{
    Color32, ColorImage, Event, MouseWheelUnit, Pos2, Rect, Sense, TextureHandle, TextureOptions,
    Ui, Vec2, pos2, vec2,
};
use egui_frames::PaneId;

use crate::{
    api::display::{DisplayClipboard, DisplayInput, DisplayPatch, StartApplicationRequest},
    backend::Socket,
    native::{
        app::App,
        panes::{OpenPaneRequest, Pane},
        theme,
    },
};

/// The view a desktop is started with, in points, before any pane has said how much room it
/// has: a pane showing it asks for its own size as soon as it is drawn.
const VIEW_TO_START_WITH: Vec2 = vec2(1280.0, 800.0);

/// How often a pane looks at its socket while nothing else is making the window draw: as
/// often as the server looks at the display.
const BETWEEN_LOOKS: Duration = Duration::from_millis(33);

/// How far a wheel turned in points is one click of a wheel on the display, which X only
/// knows clicks of.
const POINTS_A_CLICK: f32 = 40.0;
/// And how many clicks a wheel that reports whole pages turned.
const CLICKS_A_PAGE: f32 = 10.0;
/// The most clicks sent for one frame's turning: a flick of a trackpad is hundreds of points.
const MOST_CLICKS_A_FRAME: f32 = 20.0;

/// The keys that with ⌘ or Ctrl held are cut, copy and paste, which the window says as
/// events of their own - see `Watched::type_event`.
const CLIPBOARD_KEYS: [egui::Key; 3] = [egui::Key::X, egui::Key::C, egui::Key::V];

/// One display pixel is one texel, and stays one: a display shown at twice its size is its
/// pixels doubled, not smeared.
const TEXTURE: TextureOptions = TextureOptions::NEAREST;

/// The server's desktop as this window is watching it.
#[derive(Default)]
pub(crate) struct Displays {
    /// Each display pane's view of the desktop, once its socket is open.
    watched: HashMap<PaneId, Watched>,
    /// The sockets that opened on a worker, until the frame that makes them a pane's.
    opened: Arc<Mutex<Vec<Opened>>>,
    /// Why a pane's socket did not open, said on the pane.
    refused: HashMap<PaneId, String>,
    /// How many pixels of its screen a point of this window was on the last frame drawn,
    /// which is the scale a desktop started from here is given.
    pixels_per_point: f32,
}

/// Where the desktop's pixels fall on this window's: the pane, taken in to whole pixels of
/// the screen, so that a pixel of the desktop is drawn on exactly one of them.
#[derive(Clone, Copy)]
struct OnScreen {
    /// The top left pixel wholly inside the pane, in pixels from the window's top left.
    first_pixel: Vec2,
    /// How many whole pixels the pane has room for from there.
    room: [u16; 2],
    pixels_per_point: f32,
}

impl OnScreen {
    fn of(pane: Rect, pixels_per_point: f32) -> Self {
        let first_pixel = (pane.min.to_vec2() * pixels_per_point).ceil();
        let past_the_last = (pane.max.to_vec2() * pixels_per_point).floor();
        let room = (past_the_last - first_pixel).max(Vec2::ZERO);
        Self {
            first_pixel,
            room: [room.x as u16, room.y as u16],
            pixels_per_point,
        }
    }

    /// The pixel of the desktop under a place in the window.
    fn pixel_at(&self, at: Pos2) -> [i16; 2] {
        let pixel = (at.to_vec2() * self.pixels_per_point - self.first_pixel).floor();
        [pixel.x as i16, pixel.y as i16]
    }

    /// Where a view of this many pixels is painted, in points.
    fn rect_of(&self, view: [u16; 2]) -> Rect {
        Rect::from_min_size(
            (self.first_pixel / self.pixels_per_point).to_pos2(),
            vec2(f32::from(view[0]), f32::from(view[1])) / self.pixels_per_point,
        )
    }
}

struct Opened {
    pane_id: PaneId,
    socket: anyhow::Result<Socket>,
}

/// The modifier keys the display was last told are down.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Holding {
    shift: bool,
    control: bool,
    alt: bool,
}

impl Holding {
    fn of(modifiers: egui::Modifiers) -> Self {
        Self {
            shift: modifiers.shift,
            // ⌘ on a Mac is what Ctrl is everywhere else, which is where the display is.
            control: modifiers.ctrl || modifiers.command,
            alt: modifiers.alt,
        }
    }
}

/// The desktop, as one pane is watching it.
struct Watched {
    socket: Socket,
    clipboard: clipboard::Clipboard,
    /// The display's view, once its first patch has said how large that is.
    picture: Option<(TextureHandle, [u16; 2])>,
    /// The size the display was last asked to make its view.
    asked_for: Option<[u16; 2]>,
    holding: Holding,
    /// The pointer buttons that went down over the display, by their X numbers, which it is
    /// owed the release of wherever the pointer is by then.
    pressed: HashSet<u8>,
    /// Where the display was last told the pointer is.
    pointer: Option<[i16; 2]>,
    /// The turn of the wheel not yet worth a click.
    wheel: Vec2,
    /// Whether the socket has closed: the display is over, or the server is gone.
    over: bool,
}

impl Watched {
    fn new(socket: Socket) -> Self {
        Self {
            socket,
            clipboard: clipboard::Clipboard::default(),
            picture: None,
            asked_for: None,
            holding: Holding::default(),
            pressed: HashSet::new(),
            pointer: None,
            wheel: Vec2::ZERO,
            over: false,
        }
    }

    fn say(&mut self, input: &DisplayInput) {
        let said = serde_json::to_string(input).expect("an input is plain data");
        if self.socket.said.say(said).is_err() {
            self.over = true;
        }
    }

    /// Paint in every patch that has arrived since the last frame.
    fn hear(&mut self, ctx: &egui::Context) -> anyhow::Result<()> {
        loop {
            match self.socket.heard.try_recv() {
                Ok(message) => match DisplayClipboard::from_message(&message)? {
                    Some(reply) => self.clipboard.received(ctx, reply),
                    None => self.paint(ctx, DisplayPatch::from_message(&message)?),
                },
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    self.over = true;
                    return Ok(());
                }
            }
        }
    }

    fn paint(&mut self, ctx: &egui::Context, patch: DisplayPatch) {
        let size = patch.size.map(usize::from);
        let patched = ColorImage::from_rgb(size, &patch.rgb);
        match &mut self.picture {
            Some((texture, view)) if *view == patch.view => {
                texture.set_partial(patch.at.map(usize::from), patched, TEXTURE);
            }
            // The first patch, or the first of a view of another size: either is all of it.
            _ => {
                assert_eq!(
                    (patch.at, patch.size),
                    ([0, 0], patch.view),
                    "the first patch of a view is the whole of it"
                );
                let texture = ctx.load_texture("the server's desktop", patched, TEXTURE);
                self.picture = Some((texture, patch.view));
            }
        }
    }

    /// Ask the display to make its view the size of the pane, once the pane has stopped
    /// changing size: a divider being dragged is a new size every frame, and each one asked
    /// for is every window on the display laid out again.
    fn fit(&mut self, wanted: [u16; 2], settled: bool) {
        if settled && self.asked_for != Some(wanted) && wanted[0] > 0 && wanted[1] > 0 {
            self.asked_for = Some(wanted);
            self.say(&DisplayInput::Resized {
                width: wanted[0],
                height: wanted[1],
            });
        }
    }

    /// Send the display what the pointer did over it this frame.
    fn point(&mut self, ui: &Ui, on_screen: OnScreen, over_it: bool) {
        let events = ui.input(|input| input.events.clone());
        for event in events {
            match event {
                Event::PointerMoved(at) if over_it || !self.pressed.is_empty() => {
                    self.move_pointer(at, on_screen);
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    ..
                } if over_it => {
                    self.move_pointer(pos, on_screen);
                    let button = keys::x_button(button);
                    self.pressed.insert(button);
                    self.say(&DisplayInput::Button {
                        button,
                        pressed: true,
                    });
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    ..
                } if self.pressed.remove(&keys::x_button(button)) => {
                    self.move_pointer(pos, on_screen);
                    self.say(&DisplayInput::Button {
                        button: keys::x_button(button),
                        pressed: false,
                    });
                }
                Event::MouseWheel { unit, delta, .. } if over_it => self.turn_wheel(unit, delta),
                _ => {}
            }
        }
    }

    fn move_pointer(&mut self, at: Pos2, on_screen: OnScreen) {
        let at = on_screen.pixel_at(at);
        if self.pointer != Some(at) {
            self.pointer = Some(at);
            self.say(&DisplayInput::PointerMoved { x: at[0], y: at[1] });
        }
    }

    fn turn_wheel(&mut self, unit: MouseWheelUnit, delta: Vec2) {
        let clicks = match unit {
            MouseWheelUnit::Point => delta / POINTS_A_CLICK,
            MouseWheelUnit::Line => delta,
            MouseWheelUnit::Page => delta * CLICKS_A_PAGE,
        };
        self.wheel += clicks;
        let whole = self
            .wheel
            .clamp(
                Vec2::splat(-MOST_CLICKS_A_FRAME),
                Vec2::splat(MOST_CLICKS_A_FRAME),
            )
            .round();
        if whole == Vec2::ZERO {
            return;
        }
        // What the clamp cut off is dropped with what was sent, not kept to scroll on after
        // the hand has stopped.
        self.wheel = (self.wheel - whole).clamp(Vec2::splat(-1.0), Vec2::splat(1.0));
        // egui reports how the content moves; X wheel buttons give the scroll direction, which
        // is the opposite.
        self.say(&DisplayInput::Scrolled {
            right: -whole.x as i32,
            down: -whole.y as i32,
        });
    }

    /// Send the display what was typed this frame, while it has the keyboard - and let go of
    /// every modifier key the moment it has not, or a Ctrl held while the keyboard left would
    /// be held there for good.
    ///
    /// Each key is sent with the modifiers its own event says were down, not the ones down
    /// when the frame is drawn: a chord pressed and let go within one frame is still a chord.
    fn type_into(&mut self, ui: &Ui, has_keyboard: bool) {
        let (events, modifiers) = ui.input(|input| (input.events.clone(), input.modifiers));
        if has_keyboard {
            for event in events {
                self.type_event(event);
            }
        }
        self.hold(match has_keyboard {
            true => Holding::of(modifiers),
            false => Holding::default(),
        });
    }

    fn type_event(&mut self, event: Event) {
        match event {
            // Text is the characters themselves, shift and all: with Ctrl or Alt still held
            // from a chord before it, it would be typed as chords.
            Event::Text(text) => {
                self.hold(Holding {
                    shift: self.holding.shift,
                    control: false,
                    alt: false,
                });
                self.say(&DisplayInput::Typed { text });
            }
            Event::Copy | Event::Cut => {
                let cut = matches!(event, Event::Cut);
                if let Some(id) = self.clipboard.begin() {
                    self.say(&DisplayInput::Copy { id, cut });
                }
            }
            Event::Paste(text) => {
                if let Some(id) = self.clipboard.pasted(&text) {
                    self.say(&DisplayInput::Paste { id, text });
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // A browser's window is told of the key as well as of what it meant, and
                // that would be the chord twice.
                if modifiers.command && CLIPBOARD_KEYS.contains(&key) {
                    return;
                }
                let holding = Holding::of(modifiers);
                let chorded = holding.control || holding.alt;
                let keysym = keys::keysym_of_named(key)
                    .or_else(|| chorded.then(|| keys::keysym_of_chorded(key)).flatten());
                if let Some(keysym) = keysym {
                    self.hold(holding);
                    self.say(&DisplayInput::Stroke { keysym });
                }
            }
            _ => {}
        }
    }

    /// Tell the display which modifier keys are down, when that is not what it was last told.
    fn hold(&mut self, holding: Holding) {
        if holding == self.holding {
            return;
        }
        self.holding = holding;
        self.say(&DisplayInput::Holding {
            shift: holding.shift,
            control: holding.control,
            alt: holding.alt,
        });
    }
}

/// The desktop, over the whole of its pane: nothing of the pane's own is drawn beside it. Its
/// name on the server is said on the pane's tab - see `PaneView::tab` - and it is ended from
/// the palette.
pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    let palette = app.palette_of();
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let says = |ui: &Ui, text: &str, color: Color32| {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(theme::UI_SIZE),
            color,
        );
    };

    if let Some(why) = app.displays.refused.get(&pane_id) {
        says(ui, why, palette.muted);
        return;
    }
    let Some(watched) = app.displays.watched.get_mut(&pane_id) else {
        says(ui, "opening the server's desktop", palette.muted);
        return;
    };
    if let Err(error) = watched.hear(ui.ctx()) {
        app.displays.refused.insert(pane_id, format!("{error:#}"));
        app.displays.watched.remove(&pane_id);
        return;
    }
    // Closed with its pane once the workspace is drawn - see `App::settle_displays`.
    if watched.over {
        return;
    }

    if app.pane_taking_keyboard == Some(pane_id) {
        app.pane_taking_keyboard = None;
        response.request_focus();
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.has_focus() {
        // Tab, the arrows and Escape are the application's, not egui's to move the focus with.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    let on_screen = OnScreen::of(rect, ui.ctx().pixels_per_point());
    let settled = !ui.input(|input| input.pointer.any_down());
    watched.fit(on_screen.room, settled);
    watched.point(ui, on_screen, response.contains_pointer());
    watched.type_into(ui, response.has_focus());

    if let Some((texture, view)) = &watched.picture {
        // Clipped to the pane: a view on its way to a smaller size is still the larger one.
        ui.painter_at(rect).image(
            texture.id(),
            on_screen.rect_of(*view),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    watched.clipboard.draw(ui.ctx());
    ui.ctx().request_repaint_after(BETWEEN_LOOKS);
}

impl App {
    /// Start an application with windows - what `moon › Applications` and the palette ask
    /// for. On the server's desktop, which is started with it when there is none, and shown
    /// in a pane; or, in a window that is its machine's own desktop - `moon desktop` - on
    /// the screen that window is on, where the session puts it in a pane of its own.
    pub(crate) fn start_application(&mut self, command: String) {
        if self.manages_the_session {
            self.start_application_on_this_screen(&command);
            return;
        }
        let session_id = self.model.root_session_id.clone();
        let pixels_per_point = self.displays.pixels_per_point;
        let view = (VIEW_TO_START_WITH * pixels_per_point).round();
        let request = StartApplicationRequest {
            command,
            width: view.x as u16,
            height: view.y as u16,
            // Applications scale by whole numbers, and never down.
            scale: pixels_per_point.round().max(1.0) as u8,
        };
        self.tasks.spawn(
            move |backend| backend.start_application(&session_id, &request),
            |model, started| match started {
                Ok(display) => {
                    model.server_display = Some(display);
                    model.display_wants_showing = true;
                }
                Err(error) => {
                    model.error(format!("could not start the application: {error:#}"));
                }
            },
        );
    }

    /// Start a program on the screen this window is on. Its window is the session's to find
    /// and put in a pane - see [`crate::native::application_pane`].
    ///
    /// One that ends in failure as it starts is said in the window, with what it said - see
    /// [`crate::display::started`].
    #[cfg(target_os = "linux")]
    fn start_application_on_this_screen(&mut self, command: &str) {
        use crate::display::started::{self, Said, Started};

        let (ending, ended) = std::sync::mpsc::channel();
        let started = Started::start(
            std::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdin(std::process::Stdio::null()),
            // The session's log is where an application's own words went, and still go.
            Said::PassedOn,
        )
        // Nobody left to hear is the wait below being over: it ended later than soon.
        .and_then(|program| {
            program.wait(move |ended| {
                let _ = ending.send(ended);
            })
        });
        match started {
            Ok(()) => {
                let command = command.to_owned();
                // On a worker, which is what waits while the window goes on drawing: for the
                // program to end, or `SOON`, whichever is first.
                self.tasks.spawn(
                    move |_backend| {
                        let ended = ended.recv_timeout(started::SOON).ok();
                        Ok(ended.and_then(|ended| ended.failure_of(&command)))
                    },
                    |model, failed| {
                        if let Ok(Some(failure)) = failed {
                            model.error(failure);
                        }
                    },
                );
            }
            Err(error) => self
                .model
                .error(format!("could not start `{command}`: {error:#}")),
        }
    }

    /// Only a window on Linux is ever its machine's desktop - see `moon desktop`.
    #[cfg(not(target_os = "linux"))]
    fn start_application_on_this_screen(&mut self, _command: &str) {
        unreachable!("no window off Linux manages the session it is in");
    }

    /// Ask the server whether its desktop is running, and show it when that is news to this
    /// window: started by another window, by a `moon launch` typed in one of the server's
    /// shells, or by this window before the page was loaded again.
    ///
    /// Asked as the window opens and then on its clock - see `App::poll_running_shells`.
    /// Nothing tells a window what happened on the server without it asking, and a desktop
    /// an agent started there is one no window asked for.
    ///
    /// A desktop this window already knows of is not shown again: closing its pane leaves it
    /// running, and the pane is not to come back a second later.
    pub(crate) fn show_display_if_running(&mut self) {
        let known_when_asked = self.model.server_display.clone();
        self.tasks.spawn_keyed(
            Some("display-running".to_owned()),
            |backend| backend.display(),
            move |model, running| {
                // What this window knows changed while the answer was on its way - a start of
                // its own was answered, or its pane's socket closed - and that is the later
                // news. A failed answer leaves the last one standing, as on every clock here.
                let Ok(running) = running else { return };
                if model.server_display != known_when_asked {
                    return;
                }
                if running.is_some() && running != known_when_asked {
                    model.display_wants_showing = true;
                }
                model.server_display = running;
            },
        );
    }

    /// Stop every application on the server's desktop, which ends it - what the palette's
    /// `end desktop` asks for.
    pub(crate) fn end_display(&mut self) {
        self.tasks.spawn(
            |backend| backend.end_display(),
            |model, ended| {
                // The pane closes when its socket does, which is how every window watching
                // the desktop hears.
                if let Err(error) = ended {
                    model.error(format!("could not end the server's desktop: {error:#}"));
                }
            },
        );
    }

    /// Open the display pane when the desktop wants showing, open the socket of a display
    /// pane that has none, and close the panes whose desktop is over - which every
    /// application on it having ended makes it.
    ///
    /// After the workspace is drawn: the layout is lent out to the draw until then.
    pub(crate) fn settle_displays(&mut self, ctx: &egui::Context) {
        self.displays.pixels_per_point = ctx.pixels_per_point();
        if std::mem::take(&mut self.model.display_wants_showing) {
            self.open_pane(OpenPaneRequest::Display);
        }

        let over: Vec<PaneId> = self
            .displays
            .watched
            .iter()
            .filter_map(|(pane_id, watched)| watched.over.then_some(*pane_id))
            .collect();
        for pane_id in over {
            self.model.layout.close_pane(pane_id);
            self.model.server_display = None;
        }

        let open: HashSet<PaneId> = self
            .model
            .layout
            .panes()
            .filter_map(|(pane_id, pane)| matches!(pane, Pane::Display).then_some(pane_id))
            .collect();
        self.displays
            .watched
            .retain(|pane_id, _| open.contains(pane_id));
        self.displays
            .refused
            .retain(|pane_id, _| open.contains(pane_id));

        let opened = std::mem::take(
            &mut *self
                .displays
                .opened
                .lock()
                .expect("the opened displays lock is poisoned"),
        );
        for Opened { pane_id, socket } in opened {
            // A pane closed while its socket was opening.
            if !open.contains(&pane_id) {
                continue;
            }
            match socket {
                Ok(socket) => {
                    self.displays.watched.insert(pane_id, Watched::new(socket));
                }
                Err(error) => {
                    self.displays.refused.insert(pane_id, format!("{error:#}"));
                }
            }
        }

        for pane_id in open {
            if self.displays.watched.contains_key(&pane_id)
                || self.displays.refused.contains_key(&pane_id)
            {
                continue;
            }
            let inbox = Arc::clone(&self.displays.opened);
            // One socket a pane at a time, however many frames are drawn while it opens.
            self.tasks.spawn_keyed(
                Some(format!("display-socket-{pane_id:?}")),
                |backend| Ok(backend.attach_display()),
                move |_model, socket| {
                    inbox
                        .lock()
                        .expect("the opened displays lock is poisoned")
                        .push(Opened {
                            pane_id,
                            socket: socket.and_then(|socket| socket),
                        });
                },
            );
        }
    }
}
