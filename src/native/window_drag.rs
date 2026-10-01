//! Moving the window by its tab strip, on a window that has no title bar.
//!
//! With the content running under the title bar, macOS moves the window on a press anywhere in
//! the strip's height - which is where the tabs are, so dragging a tab dragged the whole window.
//! Two halves undo that. [`install`] tells the system that a press in the window's view cannot
//! move the window; [`follow`] then moves it on purpose, when a press that began on an empty
//! part of the strip is dragged; a double click there zooms the window.

use std::sync::Once;

use egui::ViewportCommand;
use objc2::{
    Encode,
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

/// What the strip's press so far has done, kept from frame to frame: a press moves the window
/// once, however long it is dragged.
#[derive(Default)]
pub(crate) struct WindowDrag {
    started: bool,
}

impl WindowDrag {
    /// Start moving the window when a press that began in `strip` - an area of the window with
    /// nothing in it that reacts to a press - is dragged. `busy` is whether anything else has
    /// the drag: a tab being moved, a widget being dragged.
    pub(crate) fn follow(
        &mut self,
        ctx: &egui::Context,
        strip: Option<egui::Rect>,
        busy: bool,
        over_a_tab: impl Fn(egui::Pos2) -> bool,
    ) {
        // A double click on an empty part of the strip zooms the window, or puts it back, the
        // way a double click on a title bar does.
        let double_clicked_at = ctx.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        if let (Some(strip), Some(at)) = (strip, double_clicked_at)
            && strip.contains(at)
            && !over_a_tab(at)
            && !busy
        {
            let zoomed = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!zoomed));
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
        if let (Some(strip), Some(origin)) = (strip, origin)
            && strip.contains(origin)
        {
            self.started = true;
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }
}
