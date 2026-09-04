//! The faces the app draws code in, and the system fonts that fill in what they lack.
//!
//! egui ships Hack Regular as its monospace font and no other weight of it, so nothing in
//! the app could be set in bold or italic: a terminal's bold had to be faked as brighter
//! ink. This bundles Hack Bold and Hack Italic - the same release of Hack egui's regular
//! comes from, so every face has the same advance and a bold word sits in the same columns
//! as the regular one around it - each as a font family of its own. See [`bold`] and
//! [`italic`].
//!
//! Between them, Hack and egui's other bundled fonts miss most of what command line programs
//! draw with: the box-drawing characters that make a table, the block elements that make a
//! bar, and the braille patterns that make a spinner. All of those render as empty boxes,
//! which is what a shell in moonreview looked like whenever a tool tried to draw anything.
//! Rather than bundle a whole font for it, this borrows one the operating system already
//! ships. The candidates are tried in order and every one that loads is appended to every
//! family, so a character missing from the first is still found in the next.

use std::{fs, sync::Arc};

use egui::{FontData, FontFamily};

/// Hack Bold, as a family: the bold face first, then everything the monospace family falls
/// back on, so a symbol the bold face lacks is drawn from the same place a regular run
/// draws it.
pub(crate) fn bold() -> FontFamily {
    FontFamily::Name(BOLD_NAME.into())
}

/// Hack Italic, as a family, with the same fallbacks as [`bold`].
pub(crate) fn italic() -> FontFamily {
    FontFamily::Name(ITALIC_NAME.into())
}

const BOLD_NAME: &str = "hack-bold";
const ITALIC_NAME: &str = "hack-italic";

/// The faces shipped with the app, under the names their families are listed by. Both are
/// Hack v3.003, the release egui's `Hack-Regular.ttf` is taken from byte for byte;
/// `assets/fonts/Hack-LICENSE.md` is the licence they ship under.
const BUNDLED_FACES: &[(&str, &[u8])] = &[
    (BOLD_NAME, include_bytes!("../../assets/fonts/Hack-Bold.ttf")),
    (ITALIC_NAME, include_bytes!("../../assets/fonts/Hack-Italic.ttf")),
];

/// Where each platform keeps a font with the drawing characters in it, best first. Every one
/// that is there and loads gets appended, so a character missing from the first is still
/// found in a later one; a path that is not there is simply skipped.
///
/// Only plain `.ttf` files: egui reads a font as one face, and the `.ttc` collections macOS
/// keeps its terminal fonts in (Menlo, PT Mono) load without yielding a single glyph.
#[cfg(target_os = "macos")]
const SYSTEM_FONTS: &[(&str, &str)] = &[
    // Braille - which is what most command line spinners are made of - and a good deal of
    // the miscellaneous symbols tools reach for.
    ("apple-symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
];

#[cfg(target_os = "linux")]
const SYSTEM_FONTS: &[(&str, &str)] = &[
    // DejaVu carries the braille, the box drawing and the block elements between them.
    ("dejavu-sans-mono", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
    ("dejavu-sans-mono-fedora", "/usr/share/fonts/dejavu/DejaVuSansMono.ttf"),
    ("noto-sans-mono", "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf"),
];

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const SYSTEM_FONTS: &[(&str, &str)] = &[
    ("consolas", "C:\\Windows\\Fonts\\consola.ttf"),
    ("cascadia-mono", "C:\\Windows\\Fonts\\CascadiaMono.ttf"),
];

/// Load the bundled faces under their families and append the system fonts that are there
/// to every family, egui's two included. Returns the system fonts it borrowed, which is what
/// the tests assert coverage against.
///
/// The system fonts are appended rather than inserted: egui's own fonts stay in front, so
/// the app keeps the look it was designed with and the borrowed font only answers for
/// characters nothing else has.
pub(crate) fn install(ctx: &egui::Context) -> Vec<String> {
    let mut definitions = egui::FontDefinitions::default();

    let mut borrowed = Vec::new();
    for (name, path) in SYSTEM_FONTS {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        definitions
            .font_data
            .insert((*name).to_string(), Arc::new(FontData::from_owned(bytes)));
        borrowed.push((*name).to_string());
    }
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        definitions
            .families
            .get_mut(&family)
            .expect("egui's default font definitions list both of its families")
            .extend(borrowed.iter().cloned());
    }

    // Each bundled face heads a family of its own, ahead of everything the monospace family
    // falls back on - egui's regular Hack included, so a glyph the face has no version of
    // is still drawn as the regular face draws it.
    let monospace = definitions.families[&FontFamily::Monospace].clone();
    for (name, bytes) in BUNDLED_FACES {
        definitions
            .font_data
            .insert((*name).to_string(), Arc::new(FontData::from_static(bytes)));
        let mut family = Vec::with_capacity(monospace.len() + 1);
        family.push((*name).to_string());
        family.extend(monospace.iter().cloned());
        definitions
            .families
            .insert(FontFamily::Name((*name).into()), family);
    }

    ctx.set_fonts(definitions);
    borrowed
}
