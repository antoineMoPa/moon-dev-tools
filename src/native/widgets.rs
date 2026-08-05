//! Small pieces of chrome the review UI reuses: pills, quiet buttons, section headers.

use egui::{Align, Color32, CornerRadius, Layout, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::native::theme::{Palette, SMALL_SIZE};

/// A compact label on a tinted background: staged/unstaged counts, dispatch status, and so on.
pub(crate) fn pill(ui: &mut Ui, text: &str, ink: Color32, background: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(SMALL_SIZE),
        ink,
    );
    let padding = vec2(5.0, 2.0);
    let (rect, response) =
        ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), background);
        ui.painter()
            .galley(rect.min + padding, galley, ink);
    }
    response
}

/// A button with no frame until it is hovered, which is what the dense toolbars use.
pub(crate) fn quiet_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(text).frame(false))
}

pub(crate) fn quiet_button_colored(ui: &mut Ui, text: &str, ink: Color32) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(ink)).frame(false))
}

/// A glyph on a filled disc — the tab strip's "new shell" button.
pub(crate) fn round_button(ui: &mut Ui, glyph: &str, diameter: f32, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, ink) = if response.hovered() {
            (palette.control_active_bg, palette.ink)
        } else {
            (palette.control_bg, palette.muted)
        };
        ui.painter().circle_filled(rect.center(), diameter / 2.0, fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(diameter * 0.72),
            ink,
        );
    }
    response
}

/// A toggle that reads as a segment of a group rather than a checkbox.
pub(crate) fn segment(ui: &mut Ui, selected: bool, text: &str, palette: &Palette) -> Response {
    let ink = if selected { palette.ink } else { palette.muted };
    let mut button = egui::Button::new(RichText::new(text).color(ink));
    if selected {
        button = button.fill(palette.control_active_bg);
    } else {
        button = button.fill(Color32::TRANSPARENT).frame(false);
    }
    ui.add(button)
}

/// A heading for one section of the sidebar, with an optional action on the right.
pub(crate) fn section_header(
    ui: &mut Ui,
    title: &str,
    palette: &Palette,
    trailing: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), trailing);
    });
}

/// A 1px rule in the palette's line color.
pub(crate) fn divider(ui: &mut Ui, palette: &Palette) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, palette.line),
    );
}

/// Elides the middle of a path so both the directory and the file name stay readable.
pub(crate) fn elide_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    let chars: Vec<char> = path.chars().collect();
    let keep_end = (max_chars * 2) / 3;
    let keep_start = max_chars.saturating_sub(keep_end + 1);
    let start: String = chars[..keep_start].iter().collect();
    let end: String = chars[chars.len() - keep_end..].iter().collect();
    format!("{start}…{end}")
}

/// `1,234` — thousands separated, so large diff counts stay readable.
pub(crate) fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths_are_left_alone() {
        assert_eq!(elide_path("src/main.rs", 40), "src/main.rs");
    }

    #[test]
    fn long_paths_keep_their_file_name() {
        let elided = elide_path("packages/app/src/components/deeply/nested/Thing.tsx", 24);

        assert!(elided.chars().count() <= 25, "got {elided}");
        assert!(elided.ends_with("Thing.tsx"), "got {elided}");
        assert!(elided.starts_with("packa"), "got {elided}");
    }

    #[test]
    fn counts_are_grouped_in_threes() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_234), "1,234");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
