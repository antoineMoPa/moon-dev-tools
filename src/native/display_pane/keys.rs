//! Key and button mapping - translates egui keys and pointer buttons to X keysyms and buttons.

use egui::{Key, PointerButton};

/// The keys that type nothing, and the X keysym of each. A key that types is not here: what
/// it typed is sent as text, which is the character whatever layout this keyboard has.
const KEYSYMS_OF_NAMED_KEYS: &[(Key, u32)] = &[
    (Key::Backspace, 0xff08),
    (Key::Tab, 0xff09),
    (Key::Enter, 0xff0d),
    (Key::Escape, 0xff1b),
    (Key::Home, 0xff50),
    (Key::ArrowLeft, 0xff51),
    (Key::ArrowUp, 0xff52),
    (Key::ArrowRight, 0xff53),
    (Key::ArrowDown, 0xff54),
    (Key::PageUp, 0xff55),
    (Key::PageDown, 0xff56),
    (Key::End, 0xff57),
    (Key::Insert, 0xff63),
    (Key::Delete, 0xffff),
    (Key::F1, 0xffbe),
    (Key::F2, 0xffbf),
    (Key::F3, 0xffc0),
    (Key::F4, 0xffc1),
    (Key::F5, 0xffc2),
    (Key::F6, 0xffc3),
    (Key::F7, 0xffc4),
    (Key::F8, 0xffc5),
    (Key::F9, 0xffc6),
    (Key::F10, 0xffc7),
    (Key::F11, 0xffc8),
    (Key::F12, 0xffc9),
];

/// The X number of each pointer button.
const X_BUTTONS: &[(PointerButton, u8)] = &[
    (PointerButton::Primary, 1),
    (PointerButton::Middle, 2),
    (PointerButton::Secondary, 3),
    (PointerButton::Extra1, 8),
    (PointerButton::Extra2, 9),
];

/// The keysym of a key that types nothing.
pub(super) fn keysym_of_named(key: Key) -> Option<u32> {
    KEYSYMS_OF_NAMED_KEYS
        .iter()
        .find(|(named, _)| *named == key)
        .map(|(_, keysym)| *keysym)
}

/// The keysym of a key pressed for a chord - the `c` of Ctrl C - which is the character the
/// key has on it, in its lower case: a keysym of Latin-1 is its code point. `None` for a key
/// with a name where a character would be, which is no chord a program on the display has.
pub(super) fn keysym_of_chorded(key: Key) -> Option<u32> {
    if key == Key::Space {
        return Some(u32::from(' '));
    }
    let mut characters = key.symbol_or_name().chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if character.is_ascii() => {
            Some(u32::from(character.to_ascii_lowercase()))
        }
        _ => None,
    }
}

pub(super) fn x_button(button: PointerButton) -> u8 {
    X_BUTTONS
        .iter()
        .find(|(known, _)| *known == button)
        .map(|(_, number)| *number)
        .expect("every pointer button has an X number")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_s_key_is_the_character_on_it_and_a_named_key_its_x_name() {
        assert_eq!(keysym_of_chorded(Key::C), Some(0x63));
        assert_eq!(keysym_of_chorded(Key::Num1), Some(0x31));
        assert_eq!(keysym_of_chorded(Key::Space), Some(0x20));
        assert_eq!(keysym_of_chorded(Key::Enter), None);
        assert_eq!(keysym_of_named(Key::Enter), Some(0xff0d));
        assert_eq!(keysym_of_named(Key::C), None);
    }
}
