//! Turning egui key events into the physical keys libghostty encodes from.
//!
//! libghostty's `Key` is a physical key code (a W3C `KeyboardEvent.code`), and it owns all
//! the encoding rules — application cursor keys, the Kitty keyboard protocol, `modifyOtherKeys`
//! and so on. So this file does one narrow job and makes no encoding decisions of its own.

use egui::Key as UiKey;
use libghostty_vt::key::{Key as VtKey, Mods};

pub(crate) fn vt_key(key: UiKey) -> Option<VtKey> {
    Some(match key {
        UiKey::ArrowDown => VtKey::ArrowDown,
        UiKey::ArrowLeft => VtKey::ArrowLeft,
        UiKey::ArrowRight => VtKey::ArrowRight,
        UiKey::ArrowUp => VtKey::ArrowUp,
        UiKey::Escape => VtKey::Escape,
        UiKey::Tab => VtKey::Tab,
        UiKey::Backspace => VtKey::Backspace,
        UiKey::Enter => VtKey::Enter,
        UiKey::Space => VtKey::Space,
        UiKey::Insert => VtKey::Insert,
        UiKey::Delete => VtKey::Delete,
        UiKey::Home => VtKey::Home,
        UiKey::End => VtKey::End,
        UiKey::PageUp => VtKey::PageUp,
        UiKey::PageDown => VtKey::PageDown,
        UiKey::Copy => VtKey::Copy,
        UiKey::Cut => VtKey::Cut,
        UiKey::Paste => VtKey::Paste,

        // Punctuation. egui reports some of these as the shifted glyph, but libghostty wants
        // the unshifted key on that physical spot, which is what these map back to.
        UiKey::Colon | UiKey::Semicolon => VtKey::Semicolon,
        UiKey::Comma => VtKey::Comma,
        UiKey::Backslash | UiKey::Pipe => VtKey::Backslash,
        UiKey::Slash | UiKey::Questionmark => VtKey::Slash,
        UiKey::Exclamationmark => VtKey::Digit1,
        UiKey::OpenBracket | UiKey::OpenCurlyBracket => VtKey::BracketLeft,
        UiKey::CloseBracket | UiKey::CloseCurlyBracket => VtKey::BracketRight,
        UiKey::Backtick => VtKey::Backquote,
        UiKey::Minus => VtKey::Minus,
        UiKey::Period => VtKey::Period,
        UiKey::Plus | UiKey::Equals => VtKey::Equal,
        UiKey::Quote => VtKey::Quote,
        UiKey::IntlBackslash => VtKey::IntlBackslash,

        UiKey::Num0 => VtKey::Digit0,
        UiKey::Num1 => VtKey::Digit1,
        UiKey::Num2 => VtKey::Digit2,
        UiKey::Num3 => VtKey::Digit3,
        UiKey::Num4 => VtKey::Digit4,
        UiKey::Num5 => VtKey::Digit5,
        UiKey::Num6 => VtKey::Digit6,
        UiKey::Num7 => VtKey::Digit7,
        UiKey::Num8 => VtKey::Digit8,
        UiKey::Num9 => VtKey::Digit9,

        UiKey::A => VtKey::A,
        UiKey::B => VtKey::B,
        UiKey::C => VtKey::C,
        UiKey::D => VtKey::D,
        UiKey::E => VtKey::E,
        UiKey::F => VtKey::F,
        UiKey::G => VtKey::G,
        UiKey::H => VtKey::H,
        UiKey::I => VtKey::I,
        UiKey::J => VtKey::J,
        UiKey::K => VtKey::K,
        UiKey::L => VtKey::L,
        UiKey::M => VtKey::M,
        UiKey::N => VtKey::N,
        UiKey::O => VtKey::O,
        UiKey::P => VtKey::P,
        UiKey::Q => VtKey::Q,
        UiKey::R => VtKey::R,
        UiKey::S => VtKey::S,
        UiKey::T => VtKey::T,
        UiKey::U => VtKey::U,
        UiKey::V => VtKey::V,
        UiKey::W => VtKey::W,
        UiKey::X => VtKey::X,
        UiKey::Y => VtKey::Y,
        UiKey::Z => VtKey::Z,

        UiKey::F1 => VtKey::F1,
        UiKey::F2 => VtKey::F2,
        UiKey::F3 => VtKey::F3,
        UiKey::F4 => VtKey::F4,
        UiKey::F5 => VtKey::F5,
        UiKey::F6 => VtKey::F6,
        UiKey::F7 => VtKey::F7,
        UiKey::F8 => VtKey::F8,
        UiKey::F9 => VtKey::F9,
        UiKey::F10 => VtKey::F10,
        UiKey::F11 => VtKey::F11,
        UiKey::F12 => VtKey::F12,
        UiKey::F13 => VtKey::F13,
        UiKey::F14 => VtKey::F14,
        UiKey::F15 => VtKey::F15,
        UiKey::F16 => VtKey::F16,
        UiKey::F17 => VtKey::F17,
        UiKey::F18 => VtKey::F18,
        UiKey::F19 => VtKey::F19,
        UiKey::F20 => VtKey::F20,
        UiKey::F21 => VtKey::F21,
        UiKey::F22 => VtKey::F22,
        UiKey::F23 => VtKey::F23,
        UiKey::F24 => VtKey::F24,
        UiKey::F25 => VtKey::F25,

        UiKey::ShiftLeft => VtKey::ShiftLeft,
        UiKey::ShiftRight => VtKey::ShiftRight,
        UiKey::ControlLeft => VtKey::ControlLeft,
        UiKey::ControlRight => VtKey::ControlRight,
        UiKey::AltLeft => VtKey::AltLeft,
        UiKey::AltRight => VtKey::AltRight,
        UiKey::SuperLeft => VtKey::MetaLeft,
        UiKey::SuperRight => VtKey::MetaRight,

        _ => return None,
    })
}

pub(crate) fn vt_mods(modifiers: egui::Modifiers) -> Mods {
    let mut mods = Mods::empty();
    if modifiers.shift {
        mods |= Mods::SHIFT;
    }
    if modifiers.alt {
        mods |= Mods::ALT;
    }
    if modifiers.ctrl {
        mods |= Mods::CTRL;
    }
    if modifiers.command && !modifiers.ctrl {
        mods |= Mods::SUPER;
    }
    mods
}

/// The unshifted character on a physical key, which the Kitty protocol reports alongside
/// the key itself.
pub(crate) fn unshifted_codepoint(key: VtKey) -> Option<char> {
    Some(match key {
        VtKey::A => 'a',
        VtKey::B => 'b',
        VtKey::C => 'c',
        VtKey::D => 'd',
        VtKey::E => 'e',
        VtKey::F => 'f',
        VtKey::G => 'g',
        VtKey::H => 'h',
        VtKey::I => 'i',
        VtKey::J => 'j',
        VtKey::K => 'k',
        VtKey::L => 'l',
        VtKey::M => 'm',
        VtKey::N => 'n',
        VtKey::O => 'o',
        VtKey::P => 'p',
        VtKey::Q => 'q',
        VtKey::R => 'r',
        VtKey::S => 's',
        VtKey::T => 't',
        VtKey::U => 'u',
        VtKey::V => 'v',
        VtKey::W => 'w',
        VtKey::X => 'x',
        VtKey::Y => 'y',
        VtKey::Z => 'z',
        VtKey::Digit0 => '0',
        VtKey::Digit1 => '1',
        VtKey::Digit2 => '2',
        VtKey::Digit3 => '3',
        VtKey::Digit4 => '4',
        VtKey::Digit5 => '5',
        VtKey::Digit6 => '6',
        VtKey::Digit7 => '7',
        VtKey::Digit8 => '8',
        VtKey::Digit9 => '9',
        VtKey::Space => ' ',
        VtKey::Minus => '-',
        VtKey::Equal => '=',
        VtKey::BracketLeft => '[',
        VtKey::BracketRight => ']',
        VtKey::Backslash => '\\',
        VtKey::Semicolon => ';',
        VtKey::Quote => '\'',
        VtKey::Backquote => '`',
        VtKey::Comma => ',',
        VtKey::Period => '.',
        VtKey::Slash => '/',
        _ => return None,
    })
}

/// Modifier-only presses carry no text and must not be sent as keystrokes of their own
/// unless the running program asked for release events.
pub(crate) fn is_modifier_key(key: VtKey) -> bool {
    matches!(
        key,
        VtKey::ShiftLeft
            | VtKey::ShiftRight
            | VtKey::ControlLeft
            | VtKey::ControlRight
            | VtKey::AltLeft
            | VtKey::AltRight
            | VtKey::MetaLeft
            | VtKey::MetaRight
            | VtKey::CapsLock
            | VtKey::NumLock
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_their_physical_key() {
        assert_eq!(vt_key(UiKey::C), Some(VtKey::C));
        assert_eq!(unshifted_codepoint(VtKey::C), Some('c'));
    }

    #[test]
    fn shifted_punctuation_maps_back_to_its_unshifted_key() {
        assert_eq!(vt_key(UiKey::Questionmark), Some(VtKey::Slash));
        assert_eq!(vt_key(UiKey::Pipe), Some(VtKey::Backslash));
        assert_eq!(vt_key(UiKey::Colon), Some(VtKey::Semicolon));
        assert_eq!(vt_key(UiKey::Plus), Some(VtKey::Equal));
    }

    #[test]
    fn command_is_super_unless_it_is_already_control() {
        let command = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        assert!(vt_mods(command).contains(Mods::SUPER));

        // On Linux and Windows egui sets both for the same physical Ctrl press.
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        assert!(vt_mods(ctrl).contains(Mods::CTRL));
        assert!(!vt_mods(ctrl).contains(Mods::SUPER));
    }

    #[test]
    fn modifier_presses_are_recognised_as_such() {
        assert!(is_modifier_key(VtKey::ControlLeft));
        assert!(!is_modifier_key(VtKey::A));
    }
}
