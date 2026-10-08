//! Window drag - moves and zooms the window by its tab strips, on a window that has no title bar.
//!
//! With the content running under the title bar, macOS moves the window on a press anywhere in
//! the strip's height - which is where the tabs are, so dragging a tab dragged the whole window.
//! Two halves undo that. [`install`] tells the system that a press in the window's view cannot
//! move the window; [`WindowDrag::follow`] then moves it on purpose, when a press that began on
//! an empty part of a strip is dragged. Every frame along the top of the window has such a
//! strip, one per column.
//!
//! A double click on an empty part of a strip zooms the window, or puts it back, the way one on
//! a title bar does. [`WindowDrag::follow`] only notes that it was asked;
//! [`WindowDrag::zoom_window_if_asked`] asks it of the window itself, which knows whether it is
//! zoomed.

use std::sync::Once;

use egui::ViewportCommand;
use objc2::{
    Encode, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Bool, Sel},
};

/// The class winit gives the content view of each of its windows.
const WINIT_VIEW_CLASS: &std::ffi::CStr = c"WinitView";

/// Make the window's view refuse to be a handle for moving the window. Called once the window
/// is open, since winit registers its view class when it builds the first window.
pub(crate) fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let class = AnyClass::get(WINIT_VIEW_CLASS).expect("winit's view class is registered");
        // `BOOL` is a different type on Intel and on Apple silicon, so the encoding is asked of
        // the type rather than written down.
        let encoding = std::ffi::CString::new(format!("{}@:", Bool::ENCODING))
            .expect("an encoding has no nul byte");
        // SAFETY: the replacement has the signature of the method it stands in for. It is put
        // on the subclass, so no other view is touched.
        unsafe {
            objc2::ffi::class_replaceMethod(
                std::ptr::from_ref(class).cast_mut().cast(),
                objc2::sel!(mouseDownCanMoveWindow),
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(&AnyObject, Sel) -> Bool,
                    unsafe extern "C-unwind" fn(),
                >(cannot_move_window),
                encoding.as_ptr(),
            );
        }
    });
}

unsafe extern "C-unwind" fn cannot_move_window(_view: &AnyObject, _cmd: Sel) -> Bool {
    Bool::NO
}

/// What the strips' presses have done so far, kept from frame to frame.
#[derive(Default)]
pub(crate) struct WindowDrag {
    /// A press moves the window once, however long it is dragged.
    started: bool,
    /// An empty part of a strip was double clicked, and the window has yet to be zoomed for it.
    pub(crate) zoom_asked: bool,
}

impl WindowDrag {
    /// Start moving the window when a press that began in one of `strips` is dragged, and note
    /// a double click in one of them as a zoom to do. `strips` are the tab strips along the top
    /// of the window, widgets included. `busy` is whether anything else has the drag: a tab
    /// being moved, a widget being dragged.
    pub(crate) fn follow(&mut self, ctx: &egui::Context, strips: &[egui::Rect], busy: bool) {
        let in_a_strip = |at: egui::Pos2| strips.iter().any(|strip| strip.contains(at));

        let (double_clicked_at, fullscreen) = ctx.input(|input| {
            (
                input
                    .pointer
                    .button_double_clicked(egui::PointerButton::Primary)
                    .then(|| input.pointer.interact_pos())
                    .flatten(),
                input.viewport().fullscreen.unwrap_or(false),
            )
        });
        // A click egui gave to a widget is that widget's: a tab being renamed, the button that
        // opens a tab pressed twice in a row. Only the empty part of a strip is left.
        let on_a_widget = ctx.interaction_snapshot(|interaction| interaction.clicked.is_some());
        // A window that fills the screen has no other size to go to.
        if double_clicked_at.is_some_and(in_a_strip) && !on_a_widget && !busy && !fullscreen {
            self.zoom_asked = true;
        }

        let (down, dragging, origin) = ctx.input(|input| {
            (
                input.pointer.primary_down(),
                input.pointer.is_decidedly_dragging(),
                input.pointer.press_origin(),
            )
        });
        if !down {
            self.started = false;
            return;
        }
        if self.started || !dragging || busy {
            return;
        }
        if origin.is_some_and(in_a_strip) {
            self.started = true;
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }

    /// Zoom the window, or put it back, if [`Self::follow`] noted a double click. The window is
    /// asked rather than egui: AppKit picks the direction from the frame the window has now,
    /// where egui's `maximized` is only what egui last asked for, which a window moved or
    /// resized by hand since no longer matches.
    pub(crate) fn zoom_window_if_asked(&mut self, window: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        if !std::mem::take(&mut self.zoom_asked) {
            return;
        }
        let handle = window
            .window_handle()
            .expect("eframe hands `ui` the handle of its window");
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            panic!("a macOS window has an AppKit handle");
        };
        // SAFETY: `ns_view` is winit's content view, alive for as long as the window `ui` is
        // drawing. `-[NSView window]` takes no argument and returns the `NSWindow` the view is
        // in, or nil, which the `Option` holds. `-[NSWindow zoom:]` takes a nullable sender and
        // returns nothing. Both are main thread only, and `ui` runs on the main thread.
        unsafe {
            let view: &AnyObject = handle.ns_view.cast().as_ref();
            let window: Option<Retained<AnyObject>> = msg_send![view, window];
            let window = window.expect("the view being drawn is in a window");
            let _: () = msg_send![&*window, zoom: None::<&AnyObject>];
        }
    }
}
