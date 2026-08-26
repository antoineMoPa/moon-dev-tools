//! Filling the gaps in egui's bundled fonts with whatever the machine already has.
//!
//! egui ships a subset of Hack and a subset of Noto Emoji. Between them they miss most of
//! what command line programs draw with: the box-drawing characters that make a table, the
//! block elements that make a bar, and the braille patterns that make a spinner. All of those
//! render as empty boxes, which is what a shell in moonreview looked like whenever a tool
//! tried to draw anything.
//!
//! Rather than bundle a whole font for it, this borrows one the operating system already
//! ships. The candidates are tried in order and every one that loads is appended, so a
//! character missing from the first is still found in the next.

use std::{fs, sync::Arc};

use egui::{FontData, FontFamily};

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

/// Append the system fonts that are there to both font families. Returns the ones it added,
/// which is what the tests assert coverage against.
///
/// Appended rather than inserted: egui's own fonts stay in front, so the app keeps the look
/// it was designed with and the borrowed font only answers for characters nothing else has.
pub(crate) fn install(ctx: &egui::Context) -> Vec<String> {
    let mut definitions = egui::FontDefinitions::default();
    let mut added = Vec::new();

    for (name, path) in SYSTEM_FONTS {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        definitions
            .font_data
            .insert((*name).to_string(), Arc::new(FontData::from_owned(bytes)));
        added.push((*name).to_string());
    }

    if added.is_empty() {
        return added;
    }
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        definitions
            .families
            .entry(family)
            .or_default()
            .extend(added.iter().cloned());
    }
    ctx.set_fonts(definitions);
    added
}
