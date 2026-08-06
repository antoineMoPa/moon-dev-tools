//! The colors a shell draws with, per theme.
//!
//! A terminal's colors do not come from the app's palette: programs ask for them by name —
//! "red", "bright blue", "the default foreground" — and the emulator resolves those against a
//! scheme. Ghostty's built-in scheme assumes a dark background, so on the light theme it
//! renders pale text on cream and is unreadable. This file hands the emulator a scheme that
//! suits whichever theme is on.

use libghostty_vt::{
    Terminal,
    style::{Palette, PaletteIndex, RgbColor},
};

use crate::native::theme::ThemeMode;

/// The light scheme's own colors, dark enough to read on the light theme's paper. `white` is
/// a grey rather than white for the same reason, and `bright white` is the darkest of all:
/// programs reach for it to emphasise, which on a light background means going darker.
const LIGHT_FOREGROUND: u32 = 0x1d1a16;
const LIGHT_BACKGROUND: u32 = 0xfffdf9;
const LIGHT_CURSOR: u32 = 0xb7522d;

const LIGHT_ANSI: &[(PaletteIndex, u32)] = &[
    (PaletteIndex::BLACK, 0x2f2b26),
    (PaletteIndex::RED, 0xa12d22),
    (PaletteIndex::GREEN, 0x247045),
    (PaletteIndex::YELLOW, 0x8a6100),
    (PaletteIndex::BLUE, 0x2b5fad),
    (PaletteIndex::MAGENTA, 0x96468f),
    (PaletteIndex::CYAN, 0x1f6b6b),
    (PaletteIndex::WHITE, 0x6a6156),
    (PaletteIndex::BRIGHT_BLACK, 0x6a6156),
    (PaletteIndex::BRIGHT_RED, 0xc2382b),
    (PaletteIndex::BRIGHT_GREEN, 0x27794a),
    (PaletteIndex::BRIGHT_YELLOW, 0x8f6905),
    (PaletteIndex::BRIGHT_BLUE, 0x3068bd),
    (PaletteIndex::BRIGHT_MAGENTA, 0xa5449d),
    (PaletteIndex::BRIGHT_CYAN, 0x257474),
    (PaletteIndex::BRIGHT_WHITE, 0x1d1a16),
];

/// Point the emulator at the scheme this theme calls for.
///
/// The dark theme is what Ghostty's own defaults were drawn for, so it hands them back rather
/// than restating them: `None` is how the emulator is told to use its own.
pub(crate) fn apply(terminal: &mut Terminal<'_, '_>, mode: ThemeMode) -> anyhow::Result<()> {
    let scheme = match mode {
        ThemeMode::Dark => None,
        ThemeMode::Light => Some(light_scheme(terminal)?),
    };

    let (foreground, background, cursor, palette) = match scheme {
        Some(scheme) => (
            Some(scheme.foreground),
            Some(scheme.background),
            Some(scheme.cursor),
            Some(scheme.palette),
        ),
        None => (None, None, None, None),
    };

    terminal
        .set_default_fg_color(foreground)
        .and_then(|terminal| terminal.set_default_bg_color(background))
        .and_then(|terminal| terminal.set_default_cursor_color(cursor))
        .and_then(|terminal| terminal.set_default_color_palette(palette))
        .map_err(|error| anyhow::anyhow!("failed to set the terminal's colors: {error}"))?;
    Ok(())
}

struct Scheme {
    foreground: RgbColor,
    background: RgbColor,
    cursor: RgbColor,
    palette: Palette,
}

/// The light scheme, built by overriding the named colors of Ghostty's default palette. The
/// 240 entries beyond the named sixteen are a fixed color cube either way, so they are left
/// as they were.
fn light_scheme(terminal: &Terminal<'_, '_>) -> anyhow::Result<Scheme> {
    let mut palette = terminal
        .default_color_palette()
        .map_err(|error| anyhow::anyhow!("failed to read the default palette: {error}"))?;
    for (index, color) in LIGHT_ANSI {
        palette.0[usize::from(index.0)] = rgb(*color);
    }

    Ok(Scheme {
        foreground: rgb(LIGHT_FOREGROUND),
        background: rgb(LIGHT_BACKGROUND),
        cursor: rgb(LIGHT_CURSOR),
        palette,
    })
}

const fn rgb(hex: u32) -> RgbColor {
    RgbColor {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    fn terminal() -> Terminal<'static, 'static> {
        Terminal::new(TerminalOptions {
            cols: 10,
            rows: 4,
            max_scrollback: 10,
        })
        .expect("expected a terminal")
    }

    /// WCAG relative luminance, which is what a contrast ratio is built out of.
    fn luminance(color: RgbColor) -> f64 {
        let channel = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(a: RgbColor, b: RgbColor) -> f64 {
        let (high, low) = {
            let (a, b) = (luminance(a), luminance(b));
            if a > b { (a, b) } else { (b, a) }
        };
        (high + 0.05) / (low + 0.05)
    }

    /// The whole point of the light scheme: everything a program can ask to be drawn in has
    /// to stay legible against the paper it lands on. 4.5:1 is what accessibility guidance
    /// asks of body text.
    #[test]
    fn every_light_color_is_readable_on_the_light_background() {
        let mut terminal = terminal();
        apply(&mut terminal, ThemeMode::Light).expect("expected the colors to be set");

        let background = terminal
            .default_bg_color()
            .expect("expected a background")
            .expect("the light theme sets one");
        let palette = terminal
            .default_color_palette()
            .expect("expected the palette");

        let foreground = terminal
            .default_fg_color()
            .expect("expected a foreground")
            .expect("the light theme sets one");
        assert!(
            contrast(foreground, background) >= 4.5,
            "plain text: {:.2}:1",
            contrast(foreground, background)
        );

        for (index, _) in LIGHT_ANSI {
            let color = palette.0[usize::from(index.0)];
            let ratio = contrast(color, background);
            assert!(ratio >= 4.5, "palette entry {} only reaches {ratio:.2}:1", index.0);
        }
    }

    /// The dark theme is what Ghostty's defaults were made for, so switching back to it has
    /// to hand the emulator's own colors back rather than leave the light ones in place.
    /// What the pane actually paints with, which is not the same question as what the
    /// terminal was set to: unset colors are resolved to the emulator's own here.
    fn drawn_colors(terminal: &mut Terminal<'_, '_>) -> (RgbColor, RgbColor) {
        let mut state = libghostty_vt::render::RenderState::new().expect("expected a state");
        let snapshot = state.update(terminal).expect("expected a snapshot");
        let colors = snapshot.colors().expect("expected colors");
        (colors.foreground, colors.background)
    }

    /// The reported regression: light and then dark again left a shell drawing its text in
    /// the same color as its background.
    #[test]
    fn a_round_trip_through_light_leaves_the_dark_colors_readable() {
        let mut terminal = terminal();
        let before = drawn_colors(&mut terminal);

        apply(&mut terminal, ThemeMode::Light).expect("expected the light colors");
        apply(&mut terminal, ThemeMode::Dark).expect("expected the dark colors back");

        let after = drawn_colors(&mut terminal);
        assert_eq!(
            (after.0.r, after.0.g, after.0.b, after.1.r, after.1.g, after.1.b),
            (before.0.r, before.0.g, before.0.b, before.1.r, before.1.g, before.1.b),
            "dark has to come back to where it started"
        );
        assert!(
            contrast(after.0, after.1) >= 4.5,
            "text and background have to stay apart, got {:.2}:1",
            contrast(after.0, after.1)
        );
    }

    #[test]
    fn switching_back_to_dark_gives_the_emulator_its_own_colors_again() {
        let mut terminal = terminal();
        let default_red = terminal
            .default_color_palette()
            .expect("expected the palette")
            .0[usize::from(PaletteIndex::RED.0)];

        apply(&mut terminal, ThemeMode::Light).expect("expected the light colors");
        let light_red = terminal
            .default_color_palette()
            .expect("expected the palette")
            .0[usize::from(PaletteIndex::RED.0)];
        assert_ne!(
            (light_red.r, light_red.g, light_red.b),
            (default_red.r, default_red.g, default_red.b),
            "the light theme has a red of its own"
        );

        apply(&mut terminal, ThemeMode::Dark).expect("expected the dark colors");
        let dark_red = terminal
            .default_color_palette()
            .expect("expected the palette")
            .0[usize::from(PaletteIndex::RED.0)];
        assert_eq!(
            (dark_red.r, dark_red.g, dark_red.b),
            (default_red.r, default_red.g, default_red.b)
        );
        assert_eq!(
            terminal.default_fg_color().expect("expected a read"),
            None,
            "and no foreground of its own"
        );
    }
}
