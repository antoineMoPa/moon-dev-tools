//! A pane another program's window is shown in, when moon is the window manager of the X11
//! session it runs in - see [`moon_launcher`] and `os/` for the machine that boots into one.
//!
//! The window is not drawn by moon. It is a window of X's own, made a child of the window
//! eframe draws into, and laid over the pane: egui only keeps the room for it, the way it keeps
//! room for a webview - see [`crate::native::webview_pane`], whose shape this follows. So the
//! two halves of a pane meet once a frame, after the workspace is drawn: the pane writes down
//! the rect it was given, and [`App::place_application_windows`] tells X to put the window
//! there, takes the ones whose pane is not in front off the screen, and closes the panes whose
//! window is gone.
//!
//! An X window is drawn over the whole wgpu surface, whatever egui paints there after it -
//! which is why they are all taken off the screen while the palette or the find bar is up.
//!
//! The keyboard follows the pane in front: while an application's pane is the active one, X
//! sends what is typed to that program, and moon's own window gets it back the moment another
//! pane is. A click inside the window is the program's own and moon never hears it, so the way
//! to another pane is the tab strip or the keyboard.

use std::collections::HashMap;

use egui::{Rect, Sense, Ui};
use egui_frames::PaneId;

use crate::native::app::App;

/// The application windows of this session, and where their panes were drawn this frame.
#[derive(Default)]
pub(crate) struct Applications {
    /// The rect each application pane was drawn in this frame, in points. Emptied as the
    /// windows are placed, so a pane missing from it on the next frame was not drawn.
    drawn: HashMap<PaneId, Rect>,
    /// The session this window manages, once it has taken it. A window that is not the
    /// session's window manager - one started inside somebody else's desktop - never has one.
    launcher: Option<moon_launcher::Launcher>,
    /// Whether taking the session has been tried and failed, so it is not tried every frame.
    could_not_take_the_session: bool,
    /// Which client window each pane shows, as it was last placed, so a window is only moved
    /// when its pane has moved.
    placed: HashMap<PaneId, moon_launcher::Bounds>,
    /// The client the keyboard was last given to, so it is not given again every frame.
    has_the_keyboard: Option<u32>,
    /// Every window the session has, whether or not it still has a pane - which is how a pane
    /// the person closed is turned into a window the program is asked to close.
    known: std::collections::HashSet<u32>,
    /// The ones already asked. A program takes its time about closing, and is only asked once.
    asked_to_close: std::collections::HashSet<u32>,
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    // Only what can be seen of the pane: an X window is not clipped by the frame around it.
    let rect = ui.max_rect().intersect(ui.clip_rect());
    ui.allocate_rect(rect, Sense::hover());
    app.applications.drawn.insert(pane_id, rect);
    // Nothing is drawn under it: the program's own window is put here within the frame, and
    // whatever moon painted first would show through everything the window leaves see-through.
}

impl App {
    /// Take in what the session's windows have done, then put each one where its pane was
    /// drawn this frame.
    ///
    /// After the workspace is drawn, since that is where the rects come from, and with the
    /// window's handle, which is only lent out to `eframe::App::ui`.
    pub(crate) fn place_application_windows(
        &mut self,
        window: &eframe::Frame,
        ctx: &egui::Context,
    ) {
        use crate::native::panes::Pane;

        let drawn = std::mem::take(&mut self.applications.drawn);
        if !self.manages_the_session {
            return;
        }
        self.take_the_session(window, ctx);
        let Some(launcher) = &self.applications.launcher else {
            return;
        };

        // What the session's windows did since the last frame. Collected first: answering them
        // changes the layout the placing below reads.
        let happened = launcher.events();
        for event in happened {
            self.answer_the_session(event);
        }

        let Some(launcher) = &self.applications.launcher else {
            return;
        };
        // Drawn over the panes: a window left showing would cover them.
        let covered = self.model.palette.open || self.model.find.is_some();
        let pixels_per_point = ctx.pixels_per_point();
        let showing: HashMap<PaneId, u32> = self
            .model
            .layout
            .panes()
            .filter_map(|(pane_id, pane)| match pane {
                Pane::Application { client, .. } => Some((pane_id, *client)),
                _ => None,
            })
            .collect();

        for (pane_id, client) in &showing {
            let bounds = match (covered, drawn.get(pane_id)) {
                (false, Some(rect)) => Some(physical_bounds(*rect, pixels_per_point)),
                // Behind another tab, or under the palette: off the screen until it is not.
                _ => None,
            };
            if self.applications.placed.get(pane_id).copied() != bounds {
                launcher.place(*client, bounds);
                match bounds {
                    Some(bounds) => {
                        self.applications.placed.insert(*pane_id, bounds);
                    }
                    None => {
                        self.applications.placed.remove(pane_id);
                    }
                }
            }
        }
        self.applications
            .placed
            .retain(|pane_id, _| showing.contains_key(pane_id));

        // A pane the person closed is a window nothing shows any more: the program is asked to
        // close it, the way a title bar's x would ask.
        let orphaned: Vec<u32> = self
            .applications
            .known
            .iter()
            .filter(|client| {
                !showing.values().any(|showing| showing == *client)
                    && !self.applications.asked_to_close.contains(client)
            })
            .copied()
            .collect();
        for client in orphaned {
            launcher.close(client);
            self.applications.asked_to_close.insert(client);
        }

        self.hand_the_keyboard_over(&showing, covered);
    }

    /// Become the window manager of the display this window is on, the first time a frame is
    /// drawn. The window has to exist before its windows can be made children of it.
    fn take_the_session(&mut self, window: &eframe::Frame, ctx: &egui::Context) {
        if self.applications.launcher.is_some() || self.applications.could_not_take_the_session {
            return;
        }
        let Some(container) = x_window_of(window) else {
            self.applications.could_not_take_the_session = true;
            self.model
                .error("this window is not an X11 window, so it cannot manage the session");
            return;
        };
        // See `Launcher::start` for why the session wakes the window.
        let waking = ctx.clone();
        match moon_launcher::Launcher::start(container, move || waking.request_repaint()) {
            Ok(launcher) => self.applications.launcher = Some(launcher),
            Err(error) => {
                self.applications.could_not_take_the_session = true;
                self.model
                    .error(format!("moon could not take over the session: {error}"));
            }
        }
    }

    /// A window opened, closed or renamed itself.
    fn answer_the_session(&mut self, event: moon_launcher::Event) {
        use crate::native::panes::{Pane, PaneKind};

        match event {
            moon_launcher::Event::Appeared {
                client,
                title,
                program,
            } => {
                // A window says nothing about itself until it is on screen, so the program out
                // of `WM_CLASS` is the better name to open the tab with.
                let title = match (title.is_empty(), program.is_empty()) {
                    (false, _) => title,
                    (true, false) => program,
                    (true, true) => "application".to_string(),
                };
                let frame = self.frame_for(PaneKind::Application, self.model.layout.active_frame());
                let pane =
                    self.model
                        .layout
                        .add_pane(frame, Pane::Application { client, title }, None);
                self.model.layout.focus_pane(pane);
                self.applications.known.insert(client);
            }
            moon_launcher::Event::Gone { client } => {
                let closing = self
                    .model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::Application { client: held, .. } if *held == client))
                    .map(|(pane_id, _)| pane_id);
                if let Some(pane_id) = closing {
                    self.model.layout.close_pane(pane_id);
                }
                self.applications.known.remove(&client);
                self.applications.asked_to_close.remove(&client);
            }
            // The palette's chord, pressed while a program had the keyboard - the launcher
            // keeps that one for itself. Opening the palette is what it asked for, and the
            // palette covers the panes, which is what takes the windows off the screen and the
            // keyboard back off the program a few lines further down.
            moon_launcher::Event::AskedForTheWindow => self.model.palette.show(),
            moon_launcher::Event::Retitled { client, title } => {
                if title.is_empty() {
                    return;
                }
                let renaming = self
                    .model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::Application { client: held, .. } if *held == client))
                    .map(|(pane_id, _)| pane_id);
                if let Some(Pane::Application { title: held, .. }) =
                    renaming.and_then(|pane_id| self.model.layout.pane_mut(pane_id))
                {
                    *held = title;
                }
            }
        }
    }

    /// The keyboard goes to the program whose pane is in front, and comes back to moon as soon
    /// as another pane is - or as soon as the palette is up over all of them.
    fn hand_the_keyboard_over(&mut self, showing: &HashMap<PaneId, u32>, covered: bool) {
        let Some(launcher) = &self.applications.launcher else {
            return;
        };
        let in_front = match covered {
            true => None,
            false => self
                .model
                .layout
                .active_pane()
                .and_then(|(pane_id, _)| showing.get(&pane_id).copied()),
        };
        if self.applications.has_the_keyboard == in_front {
            return;
        }
        match in_front {
            Some(client) => launcher.focus(client),
            None => launcher.take_back_the_keyboard(),
        }
        self.applications.has_the_keyboard = in_front;
    }
}

/// A rect of the window in points, as X wants it: physical pixels from the window's top left,
/// which is exactly what a child window's geometry is measured in.
fn physical_bounds(rect: Rect, pixels_per_point: f32) -> moon_launcher::Bounds {
    moon_launcher::Bounds {
        x: (rect.min.x * pixels_per_point).round() as i32,
        y: (rect.min.y * pixels_per_point).round() as i32,
        width: (rect.width() * pixels_per_point).round().max(1.0) as u32,
        height: (rect.height() * pixels_per_point).round().max(1.0) as u32,
    }
}

/// The X id of the window eframe drew, which the session's windows are made children of.
fn x_window_of(window: &eframe::Frame) -> Option<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => Some(handle.window as u32),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        // A Wayland window has no such thing: there, the compositor is the window manager and
        // an application's surface is never someone else's child.
        _ => None,
    }
}
