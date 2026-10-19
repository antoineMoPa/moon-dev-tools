//! moon_toggle_switch - a pill-shaped segmented switch for [egui](https://github.com/emilk/egui).
//!
//! One rounded track holds a word for each choice, and a thumb sits under the chosen one.
//! Every half is as wide as the widest word, so the thumb is the same size under any of them.
//! Neither the track nor the thumb has a border: the two grounds make the switch.
//!
//! ```no_run
//! # egui::__run_test_ui(|ui| {
//! use moon_toggle_switch::{ToggleSwitch, ToggleSwitchStyle};
//!
//! #[derive(Clone, Copy, PartialEq)]
//! enum Mode { Files, Content }
//! let mut mode = Mode::Files;
//!
//! let switched = ToggleSwitch::new(&[(Mode::Files, "files"), (Mode::Content, "content")], mode)
//!     .style(ToggleSwitchStyle::of(ui))
//!     .show(ui);
//! if let Some(to) = switched {
//!     mode = to;
//! }
//! # });
//! ```

use egui::{Color32, CornerRadius, FontId, Galley, Rect, Response, Sense, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

/// The colours and lettering of a switch.
#[derive(Clone, Debug, PartialEq)]
pub struct ToggleSwitchStyle {
    /// The ground of the whole switch.
    pub track: Color32,
    /// The ground under the chosen word.
    pub thumb: Color32,
    /// The chosen word.
    pub ink: Color32,
    /// The other words.
    pub muted: Color32,
    /// The words' font.
    pub font: FontId,
    /// The room round a word inside its half of the track.
    pub padding: Vec2,
}

impl ToggleSwitchStyle {
    /// A style read off the ui's own: a faint track, the panel's ground for the thumb, and
    /// the small text style for the words.
    pub fn of(ui: &Ui) -> Self {
        let visuals = ui.visuals();
        Self {
            track: visuals.widgets.inactive.weak_bg_fill,
            thumb: visuals.panel_fill,
            ink: visuals.strong_text_color(),
            muted: visuals.weak_text_color(),
            font: egui::TextStyle::Small.resolve(ui.style()),
            padding: vec2(4.0, 2.0),
        }
    }
}

/// A switch between a few choices, with the chosen one marked. It says which choice was
/// clicked, if it is another one; the caller keeps the choice.
pub struct ToggleSwitch<'a, T> {
    choices: &'a [(T, &'a str)],
    chosen: T,
    style: Option<ToggleSwitchStyle>,
}

impl<'a, T: Copy + PartialEq> ToggleSwitch<'a, T> {
    /// A switch over these choices, each with the word that stands for it, on `chosen`.
    pub fn new(choices: &'a [(T, &'a str)], chosen: T) -> Self {
        assert!(
            !choices.is_empty(),
            "a switch needs something to switch between"
        );
        Self {
            choices,
            chosen,
            style: None,
        }
    }

    /// Draw it in these colours and this font, rather than the ui's own.
    pub fn style(mut self, style: ToggleSwitchStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// How much room the switch takes: every half as wide as the widest word.
    pub fn size(&self, ui: &Ui) -> Vec2 {
        let style = self.style_or_uis(ui);
        self.size_of(&self.galleys(ui, &style), &style)
    }

    /// Draw it where the ui is, taking the room it needs. The choice clicked, if it is not
    /// the chosen one.
    pub fn show(self, ui: &mut Ui) -> Option<T> {
        let size = self.size(ui);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        self.show_in(ui, rect)
    }

    /// Draw it centred in a rect the caller already has - over the end of a text line, say -
    /// without taking room of its own. The choice clicked, if it is not the chosen one.
    pub fn show_in(self, ui: &mut Ui, within: Rect) -> Option<T> {
        let style = self.style_or_uis(ui);
        let galleys = self.galleys(ui, &style);
        let size = self.size_of(&galleys, &style);
        let track = Rect::from_center_size(within.center(), size);
        let half = size.x / self.choices.len() as f32;
        let round = CornerRadius::same((size.y / 2.0).ceil() as u8);
        ui.painter().rect_filled(track, round, style.track);

        let mut switched = None;
        for (index, ((choice, word), galley)) in self.choices.iter().zip(galleys).enumerate() {
            let on = *choice == self.chosen;
            let rect = Rect::from_min_size(
                pos2(track.min.x + index as f32 * half, track.min.y),
                vec2(half, size.y),
            );
            let response = half_response(ui, rect, word);
            if on {
                ui.painter().rect_filled(rect, round, style.thumb);
            }
            let ink = if on { style.ink } else { style.muted };
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, ink);
            if response.clicked() && !on {
                switched = Some(*choice);
            }
        }
        switched
    }

    fn style_or_uis(&self, ui: &Ui) -> ToggleSwitchStyle {
        self.style
            .clone()
            .unwrap_or_else(|| ToggleSwitchStyle::of(ui))
    }

    fn galleys(&self, ui: &Ui, style: &ToggleSwitchStyle) -> Vec<Arc<Galley>> {
        self.choices
            .iter()
            .map(|(_, word)| {
                ui.painter().layout_no_wrap(
                    (*word).to_string(),
                    style.font.clone(),
                    Color32::PLACEHOLDER,
                )
            })
            .collect()
    }

    fn size_of(&self, galleys: &[Arc<Galley>], style: &ToggleSwitchStyle) -> Vec2 {
        let widest = galleys
            .iter()
            .map(|galley| galley.size().x)
            .fold(0.0, f32::max);
        let tallest = galleys
            .iter()
            .map(|galley| galley.size().y)
            .fold(0.0, f32::max);
        vec2(
            (widest + 2.0 * style.padding.x) * self.choices.len() as f32,
            tallest + 2.0 * style.padding.y,
        )
    }
}

/// One half of the track as something to click, named after its word so it can be found by
/// it - by a screen reader, or a test.
fn half_response(ui: &mut Ui, rect: Rect, word: &str) -> Response {
    let response = ui.interact(rect, ui.id().with(("toggle switch", word)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, word));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable as _};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Mode {
        Files,
        Content,
    }

    /// Clicking the other word is the switch; clicking the chosen one is nothing.
    #[test]
    fn clicking_the_other_word_switches_and_the_chosen_one_does_not() {
        let mode = Arc::new(Mutex::new(Mode::Files));
        let mode_in_ui = Arc::clone(&mode);
        let switches = Arc::new(Mutex::new(0));
        let switches_in_ui = Arc::clone(&switches);
        let mut harness = Harness::new_ui(move |ui| {
            let chosen = *mode_in_ui.lock().unwrap();
            let switched = ToggleSwitch::new(
                &[(Mode::Files, "files"), (Mode::Content, "content")],
                chosen,
            )
            .show(ui);
            if let Some(to) = switched {
                *mode_in_ui.lock().unwrap() = to;
                *switches_in_ui.lock().unwrap() += 1;
            }
        });
        harness.run();

        harness.get_by_label("files").click();
        harness.run();
        assert_eq!(*mode.lock().unwrap(), Mode::Files);
        assert_eq!(*switches.lock().unwrap(), 0);

        harness.get_by_label("content").click();
        harness.run();
        assert_eq!(*mode.lock().unwrap(), Mode::Content);
        assert_eq!(*switches.lock().unwrap(), 1);
    }
}
