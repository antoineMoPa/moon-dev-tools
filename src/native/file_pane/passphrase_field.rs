//! Passphrase field - a one-line field that types a passphrase into a buffer which is wiped
//! when it is let go of, showing a dot per character.
//!
//! Not `egui::TextEdit`: that keeps every state of its text in an undo history of its own,
//! which would leave copies of the passphrase in memory nobody wipes. This one keeps nothing
//! but the buffer it is handed, and that buffer never grows - a push that would move it to a
//! larger allocation, leaving the old one behind unwiped, is refused instead.

use egui::{Event, Key, Response, Sense, Ui, vec2};
use zeroize::{Zeroize, Zeroizing};

/// The most bytes a passphrase may have: the buffer is allocated this large once, and never
/// moved.
const PASSPHRASE_CAPACITY: usize = 256;

const FIELD_WIDTH: f32 = 260.0;

/// An empty buffer for a passphrase to be typed into.
pub(super) fn empty_passphrase() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(PASSPHRASE_CAPACITY))
}

/// Wipe what was typed, keeping the buffer for typing again.
pub(super) fn wipe(passphrase: &mut Zeroizing<String>) {
    // Zeroizing a `String` writes over its bytes and its spare capacity, then empties it.
    passphrase.zeroize();
}

/// Draw the field and take what is typed into it while it has the keyboard. The flag is
/// whether Enter was pressed in it.
pub(super) fn passphrase_field(
    ui: &mut Ui,
    passphrase: &mut Zeroizing<String>,
    hint: &str,
) -> (Response, bool) {
    let size = vec2(FIELD_WIDTH, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::Text);
    if response.clicked() {
        response.request_focus();
    }
    let mut entered = false;
    if response.has_focus() {
        ui.input_mut(|input| {
            input.events.retain(|event| match event {
                Event::Text(text) | Event::Paste(text) => {
                    push_without_growing(passphrase, text);
                    false
                }
                Event::Key {
                    key: Key::Backspace,
                    pressed: true,
                    ..
                } => {
                    pop_and_wipe(passphrase);
                    false
                }
                Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    ..
                } => {
                    entered = true;
                    false
                }
                _ => true,
            })
        });
    }

    let visuals = ui.visuals();
    let stroke = match response.has_focus() {
        true => visuals.selection.stroke,
        false => visuals.widgets.inactive.bg_stroke,
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        2.0,
        visuals.extreme_bg_color,
        stroke,
        egui::StrokeKind::Inside,
    );
    let (shown, color) = match passphrase.is_empty() {
        true => (hint.to_string(), visuals.weak_text_color()),
        false => ("•".repeat(passphrase.chars().count()), visuals.text_color()),
    };
    painter.text(
        rect.left_center() + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        shown,
        egui::TextStyle::Body.resolve(ui.style()),
        color,
    );
    (response, entered)
}

fn push_without_growing(passphrase: &mut Zeroizing<String>, text: &str) {
    if passphrase.len() + text.len() <= passphrase.capacity() {
        passphrase.push_str(text);
    }
}

/// Take the last character off, and write over the bytes it was in: `String::pop` only
/// shortens the string, leaving them in its spare capacity.
fn pop_and_wipe(passphrase: &mut Zeroizing<String>) {
    if passphrase.pop().is_none() {
        return;
    }
    // SAFETY: only the spare capacity, past the end of the text, is written to, so the text
    // stays the UTF-8 it was.
    unsafe { passphrase.as_mut_vec() }
        .spare_capacity_mut()
        .zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passphrase_never_moves_to_a_larger_buffer_and_is_wiped_behind_a_backspace() {
        let mut passphrase = empty_passphrase();
        let buffer = passphrase.as_ptr();
        push_without_growing(&mut passphrase, "horsé");
        push_without_growing(&mut passphrase, &"x".repeat(PASSPHRASE_CAPACITY));
        assert_eq!(passphrase.as_str(), "horsé");
        assert_eq!(passphrase.as_ptr(), buffer);

        pop_and_wipe(&mut passphrase);
        assert_eq!(passphrase.as_str(), "hors");
        let behind = unsafe { std::slice::from_raw_parts(buffer.add(4), 2) };
        assert_eq!(behind, [0, 0]);
    }
}
