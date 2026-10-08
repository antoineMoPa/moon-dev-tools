//! Keyboard input - maps characters to X keycodes and sends fake key presses with XTEST.
//!
//! X has no request for "type this character". It takes a key press by keycode, and every
//! program works out the character from the keyboard layout. So typing a character means
//! finding the keycode the layout has it on, and holding shift when the character is in the
//! key's shifted column. A character the layout has no key for is mapped onto the spare
//! keycode, a key the layout leaves empty, just before it is pressed.

use std::collections::HashMap;

use anyhow::Context;
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ConnectionExt, KEY_PRESS_EVENT, KEY_RELEASE_EVENT},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::Held;

const SHIFT: u32 = 0xffe1;
const CONTROL: u32 = 0xffe3;
const ALT: u32 = 0xffe9;

/// The characters that are typed with a key that has a name rather than with one that has
/// the character on it - what a pasted paragraph has between its words and lines.
const KEYS_OF_CONTROL_CHARACTERS: &[(char, u32)] =
    &[('\n', 0xff0d), ('\r', 0xff0d), ('\t', 0xff09)];

/// The keysyms of Latin-1 are its code points, in these two stretches; every other character's
/// is its code point with this bit set.
const LATIN_1: [std::ops::RangeInclusive<u32>; 2] = [0x20..=0x7e, 0xa0..=0xff];
const UNICODE_KEYSYM: u32 = 0x0100_0000;

/// Where the layout has a keysym.
#[derive(Clone, Copy)]
struct Place {
    keycode: u8,
    /// On the key's upper half, which is typed with shift held.
    shifted: bool,
}

pub(crate) struct Keyboard {
    root: u32,
    places: HashMap<u32, Place>,
    /// A key with nothing on it, lent to a keysym the layout has no key for.
    spare: u8,
    held: Held,
}

impl Keyboard {
    pub(crate) fn read(conn: &RustConnection, root: u32) -> anyhow::Result<Self> {
        let setup = conn.setup();
        let first = setup.min_keycode;
        let mapping = conn
            .get_keyboard_mapping(first, setup.max_keycode - first + 1)?
            .reply()?;
        let per_keycode = mapping.keysyms_per_keycode as usize;

        let mut places = HashMap::new();
        let mut spare = None;
        for (at, keysyms) in mapping.keysyms.chunks(per_keycode).enumerate() {
            let keycode = first + at as u8;
            if keysyms.iter().all(|keysym| *keysym == 0) {
                spare = Some(keycode);
            }
            // The first two columns are the key alone and the key with shift. The first key a
            // keysym is found on keeps it: a layout has `Return` on two.
            for (keysym, shifted) in keysyms.iter().zip([false, true]) {
                if *keysym != 0 {
                    places.entry(*keysym).or_insert(Place { keycode, shifted });
                }
            }
        }
        Ok(Self {
            root,
            places,
            spare: spare.context("the display's keyboard has no key to spare")?,
            held: Held::default(),
        })
    }

    pub(crate) fn held(&self) -> Held {
        self.held
    }

    /// Hold exactly these modifier keys, pressing and letting go of whichever differ.
    pub(crate) fn hold(&mut self, conn: &RustConnection, held: Held) -> anyhow::Result<()> {
        for (keysym, was, is) in [
            (SHIFT, self.held.shift, held.shift),
            (CONTROL, self.held.control, held.control),
            (ALT, self.held.alt, held.alt),
        ] {
            if was != is {
                let keycode = self.place_of(conn, keysym)?.keycode;
                self.key(conn, keycode, is)?;
            }
        }
        self.held = held;
        Ok(())
    }

    /// Press one key and let it go.
    pub(crate) fn stroke(&mut self, conn: &RustConnection, keysym: u32) -> anyhow::Result<()> {
        let place = self.place_of(conn, keysym)?;
        let shift_for_it = place.shifted && !self.held.shift;
        let shift = self.place_of(conn, SHIFT)?.keycode;
        if shift_for_it {
            self.key(conn, shift, true)?;
        }
        self.key(conn, place.keycode, true)?;
        self.key(conn, place.keycode, false)?;
        if shift_for_it {
            self.key(conn, shift, false)?;
        }
        Ok(())
    }

    pub(crate) fn type_text(&mut self, conn: &RustConnection, text: &str) -> anyhow::Result<()> {
        for character in text.chars() {
            self.stroke(conn, keysym_of(character))?;
        }
        Ok(())
    }

    /// The key a keysym is on - the spare one, once it has been put there, for a keysym the
    /// layout has no key for.
    fn place_of(&mut self, conn: &RustConnection, keysym: u32) -> anyhow::Result<Place> {
        if let Some(place) = self.places.get(&keysym) {
            return Ok(*place);
        }
        conn.change_keyboard_mapping(1, self.spare, 1, &[keysym])?;
        // Every program is told the layout changed, and has to have heard before the key is
        // pressed or it reads the press off the old one.
        conn.sync()?;
        Ok(Place {
            keycode: self.spare,
            shifted: false,
        })
    }

    fn key(&self, conn: &RustConnection, keycode: u8, pressed: bool) -> anyhow::Result<()> {
        let event = if pressed {
            KEY_PRESS_EVENT
        } else {
            KEY_RELEASE_EVENT
        };
        conn.xtest_fake_input(event, keycode, x11rb::CURRENT_TIME, self.root, 0, 0, 0)?;
        Ok(())
    }
}

fn keysym_of(character: char) -> u32 {
    if let Some((_, keysym)) = KEYS_OF_CONTROL_CHARACTERS
        .iter()
        .find(|(typed_with_a_key, _)| *typed_with_a_key == character)
    {
        return *keysym;
    }
    let code_point = character as u32;
    if LATIN_1.iter().any(|stretch| stretch.contains(&code_point)) {
        return code_point;
    }
    UNICODE_KEYSYM | code_point
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_has_the_keysym_x_gives_it() {
        assert_eq!(keysym_of('a'), 0x61);
        assert_eq!(keysym_of('é'), 0xe9);
        assert_eq!(keysym_of('\n'), 0xff0d);
        assert_eq!(keysym_of('€'), 0x0100_20ac);
    }
}
