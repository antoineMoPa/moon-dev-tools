//! The spinner a pane draws while it has nothing to show yet.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;

use crate::native::{theme, widgets};

/// How far from the middle the block may sit and still read as being in it.
const SLACK: f32 = 1.0;

/// A pane waiting on something puts its spinner in the middle of the space it was given, both
/// ways: the docker extension takes a moment to reach the daemon, and a spinner in the top
/// left corner of an empty pane reads as a pane that has gone wrong rather than one that is
/// still working.
#[test]
fn a_waiting_pane_puts_its_spinner_in_the_middle_of_it() {
    let palette = theme::Palette::of(theme::ThemeMode::Dark);
    let size = egui::vec2(600.0, 400.0);
    let seen = Arc::new(Mutex::new((egui::Rect::NOTHING, egui::Rect::NOTHING)));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder().with_size(size).build_ui(move |ui| {
        let given = ui.max_rect();
        let drawn = widgets::centered_wait(ui, &palette, "starting…");
        *seen_in_ui.lock().expect("poisoned") = (given, drawn);
    });
    harness.run_steps(2);
    let (given, drawn) = *seen.lock().expect("poisoned");

    assert!(
        (drawn.center().x - given.center().x).abs() <= SLACK,
        "the spinner and its line should be halfway across the pane: they are at {} of {given:?}",
        drawn.center().x
    );
    // The pair straddles the middle, rather than the spinner alone sitting on it with the
    // line pushed below.
    assert!(
        (drawn.center().y - given.center().y).abs() <= SLACK,
        "the spinner and its line should straddle the middle of the pane: they are at {} of \
         {given:?}",
        drawn.center().y
    );
}
