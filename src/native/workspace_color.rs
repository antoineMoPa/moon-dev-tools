//! The color a workspace is marked with, so one window is told from another at a glance.
//!
//! A workspace is one window - see [`crate::native::workspace`] - and people keep several
//! open at once. The color names the window, so it has to survive the light/dark switch:
//! what is stored is the identity, `teal`, and each identity carries the ground the light
//! palette wants and the ground the dark palette wants. Switching themes keeps the window
//! the same color; it does not turn a teal window into a beige one.

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::native::theme::ThemeMode;

/// A color a workspace can be marked with. Kept per project in
/// `~/.moonreview/settings.json`: which color a window is belongs to whoever is looking at
/// it, not to the repo.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WorkspaceColor {
    /// Unmarked: the ground the two palettes already have.
    #[default]
    Plain,
    Ember,
    Amber,
    Moss,
    Teal,
    Ocean,
    Indigo,
    Plum,
    Rose,
}

/// The ground one color paints, in each of the two themes.
struct Ground {
    light: u32,
    dark: u32,
}

/// Every color and the two shades it is. Written out rather than blended from a hue: these
/// are values to be looked at and tuned by eye, and a blend function would hide which shade
/// actually ships.
///
/// Each shade sits at about the lightness of the palette's own ground, with a hue lean -
/// enough to name the window, little enough that a diff still reads on top of it.
///
/// Every shade tints *away* from the extreme its palette is against: a light ground only
/// ever takes channels away from the light palette's ground, a dark one only ever adds to
/// the dark palette's. That is what keeps the whole ladder of surfaces moving together -
/// the light palette has surfaces at pure white, and a shade that asked them to go whiter
/// would flatten them against each other. `the_surfaces_keep_their_distance_from_each_other`
/// is the test that holds this.
const GROUNDS: [(WorkspaceColor, Ground); 9] = [
    // The palettes' own `bg`, so an unmarked workspace is exactly what it was.
    (WorkspaceColor::Plain, Ground { light: 0xf3efe6, dark: 0x10141c }),
    (WorkspaceColor::Ember, Ground { light: 0xf3e6dc, dark: 0x28161c }),
    (WorkspaceColor::Amber, Ground { light: 0xf1ecd2, dark: 0x26201c }),
    (WorkspaceColor::Moss, Ground { light: 0xe2eedc, dark: 0x10221c }),
    (WorkspaceColor::Teal, Ground { light: 0xd8ece4, dark: 0x102426 }),
    (WorkspaceColor::Ocean, Ground { light: 0xd6e3e6, dark: 0x101a30 }),
    (WorkspaceColor::Indigo, Ground { light: 0xdedee6, dark: 0x1a1632 }),
    (WorkspaceColor::Plum, Ground { light: 0xece0e6, dark: 0x24142c }),
    (WorkspaceColor::Rose, Ground { light: 0xf3e2e2, dark: 0x2c1422 }),
];

/// The colors a picker offers, in the order it offers them.
pub(crate) const ALL: [WorkspaceColor; 9] = [
    WorkspaceColor::Plain,
    WorkspaceColor::Ember,
    WorkspaceColor::Amber,
    WorkspaceColor::Moss,
    WorkspaceColor::Teal,
    WorkspaceColor::Ocean,
    WorkspaceColor::Indigo,
    WorkspaceColor::Plum,
    WorkspaceColor::Rose,
];

impl WorkspaceColor {
    /// The ground this color paints in the given theme.
    pub(crate) fn bg(self, mode: ThemeMode) -> Color32 {
        let ground = GROUNDS
            .iter()
            .find(|(color, _)| *color == self)
            .map(|(_, ground)| ground)
            .expect("every workspace color has a ground");
        let hex = match mode {
            ThemeMode::Light => ground.light,
            ThemeMode::Dark => ground.dark,
        };
        Color32::from_rgb(
            ((hex >> 16) & 0xff) as u8,
            ((hex >> 8) & 0xff) as u8,
            (hex & 0xff) as u8,
        )
    }

    /// What the color is called, in the command palette and beside its swatch.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Ember => "ember",
            Self::Amber => "amber",
            Self::Moss => "moss",
            Self::Teal => "teal",
            Self::Ocean => "ocean",
            Self::Indigo => "indigo",
            Self::Plum => "plum",
            Self::Rose => "rose",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::theme::Palette;

    /// The contrast a shade has to keep against the text drawn on it. The two palettes' own
    /// grounds are far above this; the floor is here so a tint that was nudged too far in
    /// either direction is caught by a test rather than by squinting at the window.
    const CONTRAST_FLOOR: f32 = 10.0;

    /// WCAG relative luminance.
    fn luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let value = value as f32 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(one: Color32, other: Color32) -> f32 {
        let (high, low) = {
            let (a, b) = (luminance(one), luminance(other));
            if a > b { (a, b) } else { (b, a) }
        };
        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn every_color_has_a_ground_in_both_themes() {
        for color in ALL {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                // `bg` panics on a color the map has no ground for.
                color.bg(mode);
            }
        }
    }

    /// The point of two shades per color: whichever way the theme switch is thrown, the
    /// window is still readable.
    ///
    /// Every surface the color moves is checked, not just the ground - the color re-hues a
    /// whole ladder of them, and a rung that got too close to the ink is a pane nobody can
    /// read.
    #[test]
    fn every_shade_fits_the_palette_it_is_for() {
        for color in ALL {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                let palette = Palette::of_workspace(mode, color);
                let surfaces = [
                    ("the ground", palette.bg),
                    ("a frame", palette.panel),
                    ("a header", palette.header_bg),
                    ("a control", palette.control_bg),
                    ("a held control", palette.control_active_bg),
                    ("a hovered row", palette.row_hover_bg),
                    ("code", palette.code_bg),
                    ("a composer", palette.composer_bg),
                    ("a batch", palette.batch_bg),
                ];

                for (what, surface) in surfaces {
                    let ratio = contrast(surface, palette.ink);

                    assert!(
                        ratio >= CONTRAST_FLOOR,
                        "{what} in {} {} has {ratio:.1}:1 against the ink, under {CONTRAST_FLOOR}:1",
                        color.label(),
                        mode.label(),
                    );
                }
            }
        }
    }

    /// The color moves the surfaces and leaves everything a person reads alone.
    #[test]
    fn a_color_moves_the_surfaces_and_nothing_else() {
        for color in ALL {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                let plain = Palette::of(mode);
                let marked = Palette::of_workspace(mode, color);

                assert_eq!(marked.ink, plain.ink, "{} moved the ink", color.label());
                assert_eq!(marked.muted, plain.muted, "{} moved the muted ink", color.label());
                assert_eq!(marked.accent, plain.accent, "{} moved the accent", color.label());
                assert_eq!(marked.added, plain.added, "{} moved a diff color", color.label());
                assert_eq!(marked.removed, plain.removed, "{} moved a diff color", color.label());
            }
        }
    }

    /// Why the grounds are picked the way they are: a light shade only takes channels away
    /// from the light palette's ground and a dark one only adds to the dark palette's, so no
    /// surface the step is applied to can run off the end of its range.
    #[test]
    fn every_shade_tints_away_from_its_palettes_extreme() {
        for color in ALL {
            let light = color.bg(ThemeMode::Light);
            let plain_light = WorkspaceColor::Plain.bg(ThemeMode::Light);
            let dark = color.bg(ThemeMode::Dark);
            let plain_dark = WorkspaceColor::Plain.bg(ThemeMode::Dark);

            for (channel, marked, plain) in [
                ("red", light.r(), plain_light.r()),
                ("green", light.g(), plain_light.g()),
                ("blue", light.b(), plain_light.b()),
            ] {
                assert!(
                    marked <= plain,
                    "{}'s light shade adds {channel}, which a surface at white cannot take",
                    color.label(),
                );
            }
            for (channel, marked, plain) in [
                ("red", dark.r(), plain_dark.r()),
                ("green", dark.g(), plain_dark.g()),
                ("blue", dark.b(), plain_dark.b()),
            ] {
                assert!(
                    marked >= plain,
                    "{}'s dark shade takes {channel} away, which a surface at black cannot give",
                    color.label(),
                );
            }
        }
    }

    /// The surfaces move together, so a frame stands off its ground by exactly what it did
    /// before the workspace was marked.
    #[test]
    fn the_surfaces_keep_their_distance_from_each_other() {
        for color in ALL {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                let plain = Palette::of(mode);
                let marked = Palette::of_workspace(mode, color);
                let gap = |palette: &Palette| {
                    (
                        palette.panel.r() as i16 - palette.bg.r() as i16,
                        palette.panel.g() as i16 - palette.bg.g() as i16,
                        palette.panel.b() as i16 - palette.bg.b() as i16,
                    )
                };

                assert_eq!(
                    gap(&marked),
                    gap(&plain),
                    "{} in {} changed how far a frame stands off its ground",
                    color.label(),
                    mode.label(),
                );
            }
        }
    }

    /// A light shade belongs to the light palette and a dark one to the dark palette: a
    /// shade that landed on the wrong side would pass the contrast floor and still be wrong.
    #[test]
    fn light_shades_are_light_and_dark_shades_are_dark() {
        for color in ALL {
            let light = luminance(color.bg(ThemeMode::Light));
            let dark = luminance(color.bg(ThemeMode::Dark));

            assert!(light > 0.5, "{} is not a light shade", color.label());
            assert!(dark < 0.05, "{} is not a dark shade", color.label());
        }
    }

    /// An unmarked workspace is exactly the window that shipped before there were colors.
    #[test]
    fn plain_is_the_palette_that_shipped_before_there_were_colors() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(WorkspaceColor::Plain.bg(mode), Palette::of(mode).bg);
            // A palette is not `PartialEq`, so the surfaces the color moves are the ones to
            // check: an unmarked workspace must not have moved any of them.
            let plain = Palette::of(mode);
            let marked = Palette::of_workspace(mode, WorkspaceColor::Plain);

            assert_eq!(marked.bg, plain.bg);
            assert_eq!(marked.panel, plain.panel);
            assert_eq!(marked.header_bg, plain.header_bg);
            assert_eq!(marked.code_bg, plain.code_bg);
        }
    }

    #[test]
    fn a_color_is_stored_under_its_own_name() {
        let encoded = serde_json::to_string(&WorkspaceColor::Teal).expect("expected json");

        assert_eq!(encoded, "\"teal\"");
        assert_eq!(
            serde_json::from_str::<WorkspaceColor>(&encoded).expect("expected a color"),
            WorkspaceColor::Teal
        );
    }
}
