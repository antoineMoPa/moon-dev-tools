//! Desktop API types - the messages the window and the server exchange about the server's
//! desktop: a virtual X screen on the server, which applications are started on and a pane of
//! the window shows. The server side is `crate::display`, built on `moon_display`; the window
//! side is `crate::native::display_pane`.
//!
//! The desktop's websocket carries two kinds of message. From server to window,
//! [`DisplayPatch`]es: a rectangle of the view that changed, with its current pixels, one
//! binary message each. From window to server, [`DisplayInput`]s: what the person watching did,
//! one JSON text message each.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// What `POST /api/session/{id}/display/applications` is asked: an application to start on
/// the server's desktop, in the session's repo.
#[derive(Serialize, Deserialize)]
pub(crate) struct StartApplicationRequest {
    /// A line of shell.
    pub(crate) command: String,
    /// How large a view to start the desktop with, when this is what starts it, in pixels.
    /// A pane showing it asks for its own size as soon as it is drawn.
    pub(crate) width: u16,
    pub(crate) height: u16,
    /// How many pixels of the asking window's screen a point of it is, to the nearest whole
    /// number: the scale the desktop's applications draw at, when this is what starts it. A
    /// pixel of the desktop is shown on one pixel of that screen, so without it an
    /// application on a screen of twice the density would be drawn half the size.
    pub(crate) scale: u8,
}

/// The server's desktop.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub(crate) struct DisplayView {
    /// What `DISPLAY` is on the server for a program to open its windows there: `:7`.
    pub(crate) name: String,
}

/// What a person watching a display did - see `moon_display::Input`, which each of these is
/// one of.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DisplayInput {
    PointerMoved {
        x: i16,
        y: i16,
    },
    Button {
        button: u8,
        pressed: bool,
    },
    Scrolled {
        right: i32,
        down: i32,
    },
    Stroke {
        keysym: u32,
    },
    Typed {
        text: String,
    },
    Holding {
        shift: bool,
        control: bool,
        alt: bool,
    },
    Resized {
        width: u16,
        height: u16,
    },
}

/// A rectangle of a display's view that changed, as it is now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct DisplayPatch {
    /// The size of the whole view, which a patch after a resize is the first to say.
    pub(crate) view: [u16; 2],
    pub(crate) at: [u16; 2],
    pub(crate) size: [u16; 2],
    /// The rectangle's pixels, row after row, three bytes each: red, green, blue.
    pub(crate) rgb: Vec<u8>,
}

/// The six numbers a patch's message starts with, two bytes each, low byte first: the view's
/// width and height, the rectangle's left and top, its width and height. The rest of the
/// message is the pixels, deflated.
const HEADER_NUMBERS: usize = 6;
const HEADER_LENGTH: usize = HEADER_NUMBERS * 2;
const BYTES_A_PIXEL: usize = 3;

/// The fastest setting: a patch is made thirty times a second, and a screen - flat color,
/// text - deflates well at any.
#[cfg(any(target_os = "linux", test))]
const DEFLATE_LEVEL: u8 = 1;

impl DisplayPatch {
    /// The patch as one binary message of a display's socket. Made where displays are: by
    /// a server on Linux.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn to_message(&self) -> Vec<u8> {
        let mut message = Vec::with_capacity(HEADER_LENGTH + self.rgb.len() / 4);
        for number in self.view.iter().chain(&self.at).chain(&self.size) {
            message.extend_from_slice(&number.to_le_bytes());
        }
        message.extend(miniz_oxide::deflate::compress_to_vec(
            &self.rgb,
            DEFLATE_LEVEL,
        ));
        message
    }

    pub(crate) fn from_message(message: &[u8]) -> Result<Self> {
        ensure!(
            message.len() >= HEADER_LENGTH,
            "a display's message is {} bytes, short of the {HEADER_LENGTH} a patch starts with",
            message.len()
        );
        let (header, deflated) = message.split_at(HEADER_LENGTH);
        let mut numbers = header
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        let mut pair = || [numbers.next(), numbers.next()].map(|n| n.expect("six numbers"));
        let (view, at, size) = (pair(), pair(), pair());

        let expected = usize::from(size[0]) * usize::from(size[1]) * BYTES_A_PIXEL;
        let rgb = miniz_oxide::inflate::decompress_to_vec_with_limit(deflated, expected)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("a display's patch would not inflate")?;
        ensure!(
            rgb.len() == expected,
            "a display's patch of {}x{} holds {} bytes of pixels, not {expected}",
            size[0],
            size[1],
            rgb.len()
        );
        Ok(Self {
            view,
            at,
            size,
            rgb,
        })
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_patch_comes_out_of_its_message_as_it_went_in() {
        let patch = DisplayPatch {
            view: [1280, 800],
            at: [300, 20],
            size: [4, 2],
            rgb: (0..24).collect(),
        };
        let message = patch.to_message();
        assert_eq!(DisplayPatch::from_message(&message).unwrap(), patch);

        // Pixels that are not the rectangle's worth are a message gone wrong, not a picture.
        let mut short = patch.clone();
        short.rgb.truncate(20);
        assert!(DisplayPatch::from_message(&short.to_message()).is_err());
        assert!(DisplayPatch::from_message(&message[..5]).is_err());
    }
}
