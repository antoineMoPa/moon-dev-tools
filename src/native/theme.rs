//! The web app's palette, as Rust. Both frontends are the same product, so the colors are
//! kept in step with `web/src/style/base.css` rather than reinvented here.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// Every color the native UI draws with. One struct per mode, chosen once per frame, so no
/// widget has to know which theme is active.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    /// Which theme this is. Terminal panes need it: their colors come from the emulator
    /// rather than from here, so they have to be told which way round to set them.
    pub(crate) mode: ThemeMode,
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) ink: Color32,
    pub(crate) muted: Color32,
    pub(crate) line: Color32,
    pub(crate) accent: Color32,
    pub(crate) accent_2: Color32,
    pub(crate) warn: Color32,
    pub(crate) header_bg: Color32,
    pub(crate) control_bg: Color32,
    pub(crate) control_active_bg: Color32,
    pub(crate) code_bg: Color32,
    pub(crate) composer_bg: Color32,
    pub(crate) batch_bg: Color32,
    pub(crate) diff_gutter_bg: Color32,
    pub(crate) diff_gutter_line: Color32,
    pub(crate) diff_added_bg: Color32,
    pub(crate) diff_removed_bg: Color32,
    pub(crate) line_target_bg: Color32,
    pub(crate) hunk_active_bg: Color32,
    pub(crate) inline_comment_border: Color32,
    pub(crate) inline_comment_bg: Color32,
    pub(crate) status_open_bg: Color32,
    pub(crate) status_resolved_bg: Color32,
    pub(crate) status_failed_bg: Color32,
    pub(crate) status_neutral_bg: Color32,
    pub(crate) added: Color32,
    pub(crate) added_word_bg: Color32,
    pub(crate) removed: Color32,
    pub(crate) removed_word_bg: Color32,
    pub(crate) staged: Color32,
    pub(crate) unstaged: Color32,
    pub(crate) partial: Color32,
    pub(crate) snoozed: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

fn rgba(hex: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
        alpha,
    )
}

fn light() -> Palette {
    Palette {
        mode: ThemeMode::Light,
        bg: rgb(0xf3efe6),
        panel: rgb(0xfffaf2),
        ink: rgb(0x1d1a16),
        muted: rgb(0x6a6156),
        line: rgb(0xd6c9b7),
        accent: rgb(0xb7522d),
        accent_2: rgb(0x275d52),
        warn: rgb(0x8f3526),
        header_bg: rgb(0xf7f2e9),
        control_bg: rgb(0xfffdf9),
        control_active_bg: rgb(0xf2ece2),
        code_bg: rgb(0xfffdf9),
        composer_bg: rgb(0xffffff),
        batch_bg: rgb(0xeee4d6),
        diff_gutter_bg: rgba(0x312a22, 10),
        diff_gutter_line: rgba(0x312a22, 20),
        diff_added_bg: rgba(0x247045, 20),
        diff_removed_bg: rgba(0xa12d22, 20),
        line_target_bg: rgba(0xb7522d, 20),
        hunk_active_bg: rgb(0x6380f9),
        inline_comment_border: rgba(0xb7522d, 89),
        inline_comment_bg: rgba(0xb7522d, 15),
        status_open_bg: rgba(0xb7522d, 26),
        status_resolved_bg: rgba(0x275d52, 26),
        status_failed_bg: rgba(0x8f3526, 31),
        status_neutral_bg: rgba(0x275d52, 20),
        added: rgb(0x247045),
        added_word_bg: rgba(0x00aa00, 51),
        removed: rgb(0xa12d22),
        removed_word_bg: rgba(0xbe0000, 41),
        staged: rgb(0x1f4d34),
        unstaged: rgb(0x9d2f24),
        partial: rgb(0x9a6c12),
        snoozed: rgb(0x2b5fad),
    }
}

fn dark() -> Palette {
    Palette {
        mode: ThemeMode::Dark,
        bg: rgb(0x10141c),
        panel: rgb(0x171d27),
        ink: rgb(0xedf3fb),
        muted: rgb(0xa9b6c4),
        line: rgb(0x303b4b),
        accent: rgb(0xee8d68),
        accent_2: rgb(0x7ed0c2),
        warn: rgb(0xff8d83),
        header_bg: rgb(0x10141c),
        control_bg: rgb(0x121822),
        control_active_bg: rgb(0x202938),
        code_bg: rgb(0x0d121a),
        composer_bg: rgb(0x1b2330),
        batch_bg: rgb(0x202938),
        diff_gutter_bg: rgba(0xedf3fb, 11),
        diff_gutter_line: rgba(0xedf3fb, 20),
        diff_added_bg: rgba(0x72d89c, 31),
        diff_removed_bg: rgba(0xff8b82, 31),
        line_target_bg: rgba(0xee8d68, 41),
        hunk_active_bg: rgb(0x567489),
        inline_comment_border: rgba(0xee8d68, 112),
        inline_comment_bg: rgba(0xee8d68, 31),
        status_open_bg: rgba(0xee8d68, 41),
        status_resolved_bg: rgba(0x7ed0c2, 36),
        status_failed_bg: rgba(0xff8d83, 41),
        status_neutral_bg: rgba(0x7ed0c2, 28),
        added: rgb(0x72d89c),
        added_word_bg: rgba(0x00d26e, 61),
        removed: rgb(0xff8b82),
        removed_word_bg: rgba(0xff5a50, 61),
        staged: rgb(0x78d09a),
        unstaged: rgb(0xff897b),
        partial: rgb(0xe7bd58),
        snoozed: rgb(0x88aef1),
    }
}

impl Palette {
    pub(crate) fn of(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => light(),
            ThemeMode::Dark => dark(),
        }
    }

    /// Foreground for a diff line, by its leading character.
    pub(crate) fn diff_line_ink(&self, prefix: char) -> Color32 {
        match prefix {
            '+' => self.added,
            '-' => self.removed,
            _ => self.ink,
        }
    }

    pub(crate) fn diff_line_bg(&self, prefix: char) -> Option<Color32> {
        match prefix {
            '+' => Some(self.diff_added_bg),
            '-' => Some(self.diff_removed_bg),
            _ => None,
        }
    }
}

impl Palette {
    /// The palette as the workspace widget wants it, so frames and tabs are drawn in the same
    /// colors as everything else.
    pub(crate) fn frames_style(&self) -> egui_frames::FramesStyle {
        egui_frames::FramesStyle {
            background: self.bg,
            frame_fill: self.panel,
            border: self.line,
            active_border: self.accent,
            tab_fill: self.control_bg,
            active_tab_fill: self.control_active_bg,
            text: self.ink,
            inactive_text: self.muted,
            accent: self.accent,
            close_hover: self.warn,
            font: FontId::proportional(SMALL_SIZE + 1.0),
            ..egui_frames::FramesStyle::default()
        }
    }

    /// The same, for a shell. A terminal's own colors come from the emulator rather than from
    /// here, so all it takes from the palette is which way round they run.
    pub(crate) fn terminal_style(&self) -> egui_tty::TerminalStyle {
        egui_tty::TerminalStyle {
            font: FontId::monospace(CODE_SIZE),
            scheme: match self.mode {
                ThemeMode::Light => egui_tty::ColorScheme::Light,
                ThemeMode::Dark => egui_tty::ColorScheme::Dark,
            },
            cursor: self.accent,
            bold_ink: self.ink,
            notice_ink: self.muted,
            error_ink: self.warn,
            ..egui_tty::TerminalStyle::default()
        }
    }
}

/// Text roles the app uses. `TextStyle::Name` keeps them addressable from any widget.
pub(crate) const CODE_SIZE: f32 = 12.0;
pub(crate) const UI_SIZE: f32 = 13.0;
pub(crate) const SMALL_SIZE: f32 = 11.0;

pub(crate) fn tiny_style() -> TextStyle {
    TextStyle::Name("tiny".into())
}

pub(crate) fn strong_style() -> TextStyle {
    TextStyle::Name("strong".into())
}

pub(crate) fn apply(ctx: &egui::Context, mode: ThemeMode) {
    let palette = Palette::of(mode);
    let mut style = egui::Style::default();

    style.text_styles = [
        (TextStyle::Small, FontId::proportional(SMALL_SIZE)),
        (TextStyle::Body, FontId::proportional(UI_SIZE)),
        (TextStyle::Button, FontId::proportional(UI_SIZE)),
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Monospace, FontId::monospace(CODE_SIZE)),
        (tiny_style(), FontId::proportional(10.0)),
        (
            strong_style(),
            FontId::new(UI_SIZE, FontFamily::Proportional),
        ),
    ]
    .into();

    let mut visuals = match mode {
        ThemeMode::Light => Visuals::light(),
        ThemeMode::Dark => Visuals::dark(),
    };
    visuals.panel_fill = palette.bg;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.code_bg;
    visuals.faint_bg_color = palette.control_active_bg;
    visuals.override_text_color = Some(palette.ink);
    visuals.window_stroke = Stroke::new(1.0, palette.line);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.ink);
    visuals.widgets.inactive.bg_fill = palette.control_bg;
    visuals.widgets.inactive.weak_bg_fill = palette.control_bg;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.ink);
    visuals.widgets.hovered.bg_fill = palette.control_active_bg;
    visuals.widgets.hovered.weak_bg_fill = palette.control_active_bg;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.ink);
    visuals.widgets.active.bg_fill = palette.control_active_bg;
    visuals.widgets.active.weak_bg_fill = palette.control_active_bg;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.ink);
    visuals.selection.bg_fill = palette.accent.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);

    let radius = CornerRadius::same(4);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style.spacing.interact_size.y = 20.0;
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.floating = true;

    // moonreview picks its own palette, so the same style applies whichever theme the
    // desktop reports - the app's light/dark switch is the only thing that changes it.
    ctx.all_styles_mut(|target| *target = style.clone());
    apply_window_theme(ctx, mode);
}

/// Tell the OS which side of the switch the window is on: it draws the title bar, and
/// without this a dark window sits under a light mac header.
///
/// Called on every theme switch, and again over the first few frames - a command sent while
/// the window is still being set up is lost on macOS, which left the header on the desktop's
/// theme until the next switch.
pub(crate) fn apply_window_theme(ctx: &egui::Context, mode: ThemeMode) {
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match mode {
        ThemeMode::Light => egui::SystemTheme::Light,
        ThemeMode::Dark => egui::SystemTheme::Dark,
    }));
}
