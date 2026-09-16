//! A pane a web page is drawn in, by the system's own webview - WKWebView on macOS.
//!
//! The page is not drawn by egui. A webview is a native view of its own, put in the window as
//! a child of the view eframe draws into, and laid over the pane: egui only keeps the room for
//! it. So the two halves of a pane meet once a frame, after the workspace is drawn - the pane
//! writes down the rect it was given, and [`App::place_webviews`] moves the webview onto it,
//! hides the ones whose pane was not drawn, and lets go of the ones whose pane is gone.
//!
//! A native view is drawn over the whole wgpu surface, whatever egui paints there after it.
//! That is why the webviews are hidden while the palette or the find bar is up: those are
//! drawn over the panes, and would be drawn under the page.
//!
//! This is the first step of showing an agent's visualizations beside its shell. Nothing opens
//! the pane yet, and the one page it shows is a test page bundled with the window.

use std::collections::HashMap;

use egui::{Rect, Sense, Ui};
use egui_frames::PaneId;

use crate::native::app::App;

/// The page a webview pane opens on until it is given one of its own: something to see, a button whose count says scripts
/// run and clicks arrive, and an input to type in.
#[cfg(target_os = "macos")]
const TEST_PAGE: &str = include_str!("../../assets/webview/test_page.html");

/// The webviews of the window, and where their panes were drawn this frame.
#[derive(Default)]
pub(crate) struct Webviews {
    /// The rect each webview pane was drawn in this frame, in points. Emptied as the
    /// webviews are placed, so a pane missing from it on the next frame was not drawn.
    drawn: HashMap<PaneId, Rect>,
    /// Each webview pane's webview, with the rect it was last put at - a webview is only
    /// moved when its pane has moved.
    #[cfg(target_os = "macos")]
    views: HashMap<PaneId, (wry::WebView, Rect)>,
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    // Only what can be seen of the pane: a webview is not clipped by the frame around it.
    let rect = ui.max_rect().intersect(ui.clip_rect());
    ui.allocate_rect(rect, Sense::hover());
    if cfg!(target_os = "macos") {
        app.webviews.drawn.insert(pane_id, rect);
    } else {
        ui.label("web pages are only shown on macOS for now");
    }
}

impl App {
    /// Put each webview where its pane was drawn this frame, make the webview of a pane just
    /// opened, hide the ones whose pane was not drawn - a tab behind another - and drop the
    /// ones whose pane is closed.
    ///
    /// After the workspace is drawn, since that is where the rects come from, and with the
    /// window's handle, which is only lent out to `eframe::App::ui`.
    #[cfg(target_os = "macos")]
    pub(crate) fn place_webviews(&mut self, window: &eframe::Frame, ctx: &egui::Context) {
        let drawn = std::mem::take(&mut self.webviews.drawn);
        let open: Vec<PaneId> = self
            .model
            .layout
            .panes()
            .filter_map(|(pane_id, pane)| {
                matches!(pane, crate::native::panes::Pane::Webview).then_some(pane_id)
            })
            .collect();
        // Dropping a webview takes it out of the window.
        self.webviews
            .views
            .retain(|pane_id, _| open.contains(pane_id));

        // Drawn over the panes: a page left showing would cover them.
        let covered = self.model.palette.open || self.model.find.is_some();
        let pixels_per_point = ctx.pixels_per_point();
        for (pane_id, (view, _)) in &self.webviews.views {
            if covered || !drawn.contains_key(pane_id) {
                view.set_visible(false)
                    .expect("a webview can always be hidden");
            }
        }
        for (pane_id, rect) in drawn {
            let bounds = physical_bounds(rect, pixels_per_point);
            let (view, placed) = self.webviews.views.entry(pane_id).or_insert_with(|| {
                let view = wry::WebViewBuilder::new()
                    .with_html(TEST_PAGE)
                    // The first click on a window in the background lands on what it clicked,
                    // as it does on the rest of the window's panes, rather than only bringing
                    // the window forward.
                    .with_accept_first_mouse(true)
                    .with_bounds(bounds)
                    .build_as_child(window)
                    .expect("the window is an AppKit window, which a webview is made a child of");
                (view, rect)
            });
            if *placed != rect {
                view.set_bounds(bounds)
                    .expect("a child webview can always be moved");
                *placed = rect;
            }
            if !covered {
                view.set_visible(true)
                    .expect("a webview can always be shown");
            }
        }
    }
}

/// A rect of the window in points, as the webview is placed: in physical pixels from the
/// window's top left. wry turns it into the flipped, logical coordinates AppKit wants.
#[cfg(target_os = "macos")]
fn physical_bounds(rect: Rect, pixels_per_point: f32) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(
            (rect.min.x * pixels_per_point).round() as i32,
            (rect.min.y * pixels_per_point).round() as i32,
        )
        .into(),
        size: wry::dpi::PhysicalSize::new(
            (rect.width() * pixels_per_point).round() as u32,
            (rect.height() * pixels_per_point).round() as u32,
        )
        .into(),
    }
}
