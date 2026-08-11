//! Small pieces of chrome the review UI reuses: pills, quiet buttons, section headers.

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Layout, Response, RichText, Sense, Stroke, Ui, vec2,
};

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

/// The cursor anything clickable shows, the way the web frontend's `cursor: pointer` does.
/// Everything the pointer can act on goes through here, so the two frontends feel the same.
pub(crate) fn clickable(response: Response) -> Response {
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A button with no frame until it is hovered, which is what the dense toolbars use.
pub(crate) fn quiet_button(ui: &mut Ui, text: &str) -> Response {
    clickable(ui.add(egui::Button::new(text).frame(false)))
}

/// The close mark's box, matching the one on a tab.
pub(crate) const CLOSE_MARK_SIZE: f32 = 12.0;

/// The thin cross a tab carries to close it, as a button of its own.
///
/// Drawn rather than typeset for the reason `egui_frames` gives: a font's close glyph is a
/// heavy emoji, and this wants the hairline cross a browser tab draws.
pub(crate) fn close_button(ui: &mut Ui, palette: &Palette) -> Response {
    close_mark(ui, palette, true)
}

/// The same, for a mark that is there to be found and explained rather than pressed — a
/// column that still holds cards has one, because a mark that vanishes is a mark nobody
/// learns about, but it must not light up as though the press would do something.
pub(crate) fn close_mark(ui: &mut Ui, palette: &Palette, enabled: bool) -> Response {
    const SIZE: f32 = CLOSE_MARK_SIZE;

    let (rect, response) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
    if ui.is_rect_visible(rect) {
        let ink = if response.hovered() && enabled {
            palette.warn
        } else {
            palette.muted
        };
        let reach = SIZE * 0.27;
        let stroke = Stroke::new(1.0, ink);
        let center = rect.center();
        ui.painter().line_segment(
            [center + vec2(-reach, -reach), center + vec2(reach, reach)],
            stroke,
        );
        ui.painter().line_segment(
            [center + vec2(reach, -reach), center + vec2(-reach, reach)],
            stroke,
        );
    }
    clickable(response)
}

pub(crate) fn quiet_button_colored(ui: &mut Ui, text: &str, ink: Color32) -> Response {
    clickable(ui.add(egui::Button::new(RichText::new(text).color(ink)).frame(false)))
}

/// Text laid out to fit a row of fixed height: cut short with an ellipsis rather than wrapped.
///
/// A row that cannot grow has to say no to text that does not fit. Wrapping it instead is what
/// makes a long commit subject run over whatever is drawn on the line below it. The whole text
/// belongs on the row's hover instead.
pub(crate) fn cut_to_fit(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// What a confirmation came back with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Confirmed {
    /// Still being asked.
    Waiting,
    /// Go ahead.
    Yes,
    /// Never mind.
    No,
}

/// The second half of a destructive action: what it is about to do, and the way out.
///
/// Drawn in place of the control that armed it, so the question is asked where the click was
/// rather than over the whole window — nothing is nailed down behind it, and reading the
/// answer is one `match`. Discarding a hunk, discarding a file and deleting a task are all
/// this same two-press shape.
///
/// The caller keeps which thing is armed, because only the caller knows what it is asking
/// about; this draws the question.
pub(crate) fn confirm(ui: &mut Ui, palette: &Palette, question: &str, hover: &str) -> Confirmed {
    let mut answer = Confirmed::Waiting;

    if quiet_button_colored(ui, question, palette.warn)
        .on_hover_text(hover)
        .clicked()
    {
        answer = Confirmed::Yes;
    }
    if quiet_button(ui, "[keep]")
        .on_hover_text("leave it alone")
        .clicked()
    {
        answer = Confirmed::No;
    }
    answer
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
