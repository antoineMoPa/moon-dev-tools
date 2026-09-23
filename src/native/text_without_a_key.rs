//! Text macOS types into the window without a key press behind it: dictation (the F5
//! microphone), the emoji & symbols picker, an accent picked with the mouse from the popup a
//! held key opens.
//!
//! The system hands such text to the focused view's `insertText:replacementRange:`. winit's
//! view passes that call on only when it ends an IME composition - text typed on the keyboard
//! reaches the app as the key event's own text instead - so text that comes any other way is
//! dropped before egui sees it, in the terminal, the editor and every text box alike.
//!
//! Dictation also asks first where the caret is, with `selectedRange`, and winit's view says
//! there is none - `NSNotFound` - after which dictation listens and never types.
//!
//! Three of the view's methods are wrapped here. `selectedRange` answers an empty caret at the
//! start when no composition is open. `keyDown:` marks that a key press is being interpreted,
//! and an `insertText:` outside of one, with no composition open, is kept and handed to egui as
//! typed text on the next frame - see [`take`]. One that ends a composition - dictation's -
//! is winit's to hand on, and the composition is emptied after it.

use std::{
    cell::Cell,
    sync::{Mutex, Once, OnceLock},
};

use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use objc2_foundation::{NSAttributedString, NSRange, NSString};

/// The class winit gives the content view of each of its windows.
const WINIT_VIEW_CLASS: &std::ffi::CStr = c"WinitView";

type SelectedRange = unsafe extern "C-unwind" fn(&AnyObject, Sel) -> NSRange;
type KeyDown = unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject);
type InsertText = unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, NSRange);

static ORIGINAL_SELECTED_RANGE: OnceLock<SelectedRange> = OnceLock::new();
static ORIGINAL_KEY_DOWN: OnceLock<KeyDown> = OnceLock::new();
static ORIGINAL_INSERT_TEXT: OnceLock<InsertText> = OnceLock::new();
/// Woken when text arrives, since nothing else would draw a frame for it.
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
/// What arrived since the last frame, in order.
static ARRIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    /// Whether the view is interpreting a key press, whose text winit sends with the key.
    static IN_KEY_DOWN: Cell<bool> = const { Cell::new(false) };
}

/// Wrap the view's methods, once per process. Called once the window is open: winit only
/// registers its view class when it builds the first window.
pub(crate) fn install(ctx: &egui::Context) {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        CONTEXT
            .set(ctx.clone())
            .expect("the context is set only here");
        let class = AnyClass::get(WINIT_VIEW_CLASS).expect("winit's view class is registered");
        let selected_range = class
            .instance_method(sel!(selectedRange))
            .expect("winit's view has selectedRange");
        let key_down = class
            .instance_method(sel!(keyDown:))
            .expect("winit's view has keyDown:");
        let insert_text = class
            .instance_method(sel!(insertText:replacementRange:))
            .expect("winit's view has insertText:replacementRange:");
        // SAFETY: each replacement has the signature of the method it replaces, and calls the
        // original with the same arguments.
        unsafe {
            let original = selected_range
                .set_implementation(std::mem::transmute::<SelectedRange, Imp>(
                    selected_range_at_start,
                ));
            ORIGINAL_SELECTED_RANGE
                .set(std::mem::transmute::<Imp, SelectedRange>(original))
                .expect("selectedRange is wrapped once");
            let original =
                key_down.set_implementation(std::mem::transmute::<KeyDown, Imp>(key_down_marked));
            ORIGINAL_KEY_DOWN
                .set(std::mem::transmute::<Imp, KeyDown>(original))
                .expect("keyDown: is wrapped once");
            let original = insert_text
                .set_implementation(std::mem::transmute::<InsertText, Imp>(insert_text_kept));
            ORIGINAL_INSERT_TEXT
                .set(std::mem::transmute::<Imp, InsertText>(original))
                .expect("insertText:replacementRange: is wrapped once");
        }
    });
}

/// The text that arrived since the last call, to be given to egui as typed text.
pub(crate) fn take() -> Vec<String> {
    std::mem::take(
        &mut *ARRIVED
            .lock()
            .expect("the arrived text lock is not poisoned"),
    )
}

unsafe extern "C-unwind" fn selected_range_at_start(view: &AnyObject, cmd: Sel) -> NSRange {
    let original = ORIGINAL_SELECTED_RANGE
        .get()
        .expect("selectedRange is wrapped");
    let composing: bool = unsafe { msg_send![view, hasMarkedText] };
    if composing {
        return unsafe { original(view, cmd) };
    }
    // The text is egui's, which the view knows nothing of: an empty caret is enough for
    // dictation to go ahead, and where it lands is the focused widget's business.
    NSRange::new(0, 0)
}

unsafe extern "C-unwind" fn key_down_marked(view: &AnyObject, cmd: Sel, event: &AnyObject) {
    let original = ORIGINAL_KEY_DOWN.get().expect("keyDown: is wrapped");
    IN_KEY_DOWN.with(|in_key_down| in_key_down.set(true));
    unsafe { original(view, cmd, event) };
    IN_KEY_DOWN.with(|in_key_down| in_key_down.set(false));
}

unsafe extern "C-unwind" fn insert_text_kept(
    view: &AnyObject,
    cmd: Sel,
    string: &AnyObject,
    replacement_range: NSRange,
) {
    let original = ORIGINAL_INSERT_TEXT
        .get()
        .expect("insertText:replacementRange: is wrapped");
    // Asked before the original runs, which ends any composition there is.
    let composing: bool = unsafe { msg_send![view, hasMarkedText] };
    unsafe { original(view, cmd, string, replacement_range) };
    if IN_KEY_DOWN.with(Cell::get) {
        return;
    }
    if composing {
        // winit commits the composition but only lets go of its text inside a key press, so
        // a dictation would leave the view composing for good - and `selectedRange` saying
        // there is no caret, which keeps the next dictation from starting. An empty
        // composition is how the view is told there is none.
        let empty = NSString::new();
        let no_replacement = NSRange::new(objc2_foundation::NSNotFound as usize, 0);
        let _: () = unsafe {
            msg_send![
                view,
                setMarkedText: &*empty,
                selectedRange: NSRange::new(0, 0),
                replacementRange: no_replacement
            ]
        };
        return;
    }
    // Documented to be either an `NSString` or an `NSAttributedString`.
    let text = match string.downcast_ref::<NSAttributedString>() {
        Some(attributed) => attributed.string().to_string(),
        None => string
            .downcast_ref::<NSString>()
            .expect("insertText: is handed an NSString or an NSAttributedString")
            .to_string(),
    };
    ARRIVED
        .lock()
        .expect("the arrived text lock is not poisoned")
        .push(text);
    CONTEXT
        .get()
        .expect("the context is set on install")
        .request_repaint();
}
