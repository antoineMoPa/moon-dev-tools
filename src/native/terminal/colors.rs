//! The colors a shell draws with, per theme.
//!
//! A terminal's colors do not come from the app's palette: programs ask for them by name —
//! "red", "bright blue", "the default foreground" — and the emulator resolves those against a
//! scheme. Ghostty's built-in scheme assumes a dark background, so on the light theme it
//! renders pale text on cream and is unreadable. This file hands the emulator a scheme that
//! suits whichever theme is on.
//!
//! Both themes are set as explicit colors, including the dark one that started as the
//! emulator's own. Handing the emulator `None` to mean "back to your defaults" does not put
//! them back, so a shell that had been through the light theme once stayed light — which
//! against a dark panel is text the color of its own background.

use libghostty_vt::{
    Terminal,
    render::Colors,
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

/// Everything a terminal has to be told about color.
#[derive(Clone)]
pub(crate) struct Scheme {
    pub(crate) foreground: RgbColor,
    pub(crate) background: RgbColor,
    pub(crate) cursor: RgbColor,
    pub(crate) palette: Palette,
}

/// The scheme the emulator arrived with, read off the render state before anything has
/// changed it. This is what the dark theme is — Ghostty's own colors — kept so they can be
/// put back exactly rather than asked for again.
pub(crate) fn emulator_scheme(colors: &Colors) -> Scheme {
    Scheme {
        foreground: colors.foreground,
        background: colors.background,
        cursor: colors.cursor.unwrap_or(colors.foreground),
        palette: Palette(colors.palette),
    }
}

/// Point the emulator at the scheme this theme calls for.
pub(crate) fn apply(
    terminal: &mut Terminal<'_, '_>,
    emulator: &Scheme,
    mode: ThemeMode,
) -> anyhow::Result<()> {
    let scheme = match mode {
        ThemeMode::Dark => emulator.clone(),
        ThemeMode::Light => light_from(emulator),
    };

    terminal
        .set_default_fg_color(Some(scheme.foreground))
        .and_then(|terminal| terminal.set_default_bg_color(Some(scheme.background)))
        .and_then(|terminal| terminal.set_default_cursor_color(Some(scheme.cursor)))
        .and_then(|terminal| terminal.set_default_color_palette(Some(scheme.palette)))
        .map_err(|error| anyhow::anyhow!("failed to set the terminal's colors: {error}"))?;
    Ok(())
}

/// The light scheme, built by overriding the named colors of whatever the emulator started
/// with. The 240 entries beyond the named sixteen are a fixed color cube either way, so they
/// are left as they were.
fn light_from(emulator: &Scheme) -> Scheme {
    let mut palette = emulator.palette;
    for (index, color) in LIGHT_ANSI {
        palette.0[usize::from(index.0)] = rgb(*color);
    }

    Scheme {
        foreground: rgb(LIGHT_FOREGROUND),
        background: rgb(LIGHT_BACKGROUND),
        cursor: rgb(LIGHT_CURSOR),
        palette,
    }
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
    use libghostty_vt::{TerminalOptions, render::RenderState};

    fn terminal() -> Terminal<'static, 'static> {
        Terminal::new(TerminalOptions {
            cols: 10,
            rows: 4,
            max_scrollback: 10,
        })
        .expect("expected a terminal")
    }

    /// What the pane would actually paint with, which is not the same question as what the
    /// terminal was set to: unset colors are resolved to the emulator's own here.
    fn drawn(terminal: &mut Terminal<'_, '_>) -> Scheme {
        let mut state = RenderState::new().expect("expected a render state");
        let snapshot = state.update(terminal).expect("expected a snapshot");
        emulator_scheme(&snapshot.colors().expect("expected colors"))
    }

    fn same(left: RgbColor, right: RgbColor) -> bool {
        (left.r, left.g, left.b) == (right.r, right.g, right.b)
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
        let emulator = drawn(&mut terminal);
        apply(&mut terminal, &emulator, ThemeMode::Light).expect("expected the light colors");

        let light = drawn(&mut terminal);
        assert!(
            contrast(light.foreground, light.background) >= 4.5,
            "plain text: {:.2}:1",
            contrast(light.foreground, light.background)
        );
        for (index, _) in LIGHT_ANSI {
            let color = light.palette.0[usize::from(index.0)];
            let ratio = contrast(color, light.background);
            assert!(
                ratio >= 4.5,
                "palette entry {} only reaches {ratio:.2}:1",
                index.0
            );
        }
    }

    /// The reported regression: light and then dark again left a shell drawing its text in
    /// the same color as its background, because the emulator does not take `None` to mean
    /// "back to your own colors".
    #[test]
    fn a_round_trip_through_light_puts_the_dark_colors_back_exactly() {
        let mut terminal = terminal();
        let emulator = drawn(&mut terminal);

        apply(&mut terminal, &emulator, ThemeMode::Light).expect("expected the light colors");
        let light = drawn(&mut terminal);
        assert!(
            !same(light.foreground, emulator.foreground),
            "the light theme really did change something"
        );

        apply(&mut terminal, &emulator, ThemeMode::Dark).expect("expected the dark colors back");
        let back = drawn(&mut terminal);

        assert!(same(back.foreground, emulator.foreground), "text");
        assert!(same(back.background, emulator.background), "background");
        assert!(
            same(
                back.palette.0[usize::from(PaletteIndex::RED.0)],
                emulator.palette.0[usize::from(PaletteIndex::RED.0)]
            ),
            "and the palette with them"
        );
        assert!(
            contrast(back.foreground, back.background) >= 4.5,
            "text and background have to stay apart, got {:.2}:1",
            contrast(back.foreground, back.background)
        );
    }
}
