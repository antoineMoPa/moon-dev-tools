//! The toasts: where they stand while they are up.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, seeded_fixture, settle};

const WINDOW: egui::Vec2 = egui::vec2(1200.0, 760.0);

/// A toast stands in the bottom left corner, on the status bar, and the next one stacks up
/// from it. The left rather than the right: the right-hand side is where a pane's controls
/// and the tab strip's `+` are, which is where the hand that just asked for something is.
#[test]
fn toasts_stand_in_the_bottom_left_corner() {
    let fixture = seeded_fixture("toasts");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    // Said once the window is drawing the review rather than while it is still opening it.
    // A toast is up for a few seconds of real time, and the review can take longer than that
    // to load on a busy machine - so one posted before it would have faded by the time there
    // was a window to photograph it in.
    let say = Arc::new(AtomicBool::new(false));
    let say_in_ui = Arc::clone(&say);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let up = Arc::new(AtomicBool::new(false));
    let up_in_ui = Arc::clone(&up);

    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if say_in_ui.swap(false, Ordering::Relaxed) {
                app.model.info("staged the whole of src/lib.rs");
                app.model
                    .error("could not rename the task: a task needs a title");
                // The strip along the bottom reads the last message out with the time it was
                // said, which is a different picture every run. Put away there and the strip
                // is the one it draws when nothing has been said; the toasts are their own
                // list and stay up.
                app.model.messages.dismiss_latest();
            }
            app.draw(ui);
            loaded_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            up_in_ui.store(app.model.toasts.len() == 2, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    say.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || up.load(Ordering::Relaxed)),
        "the window never posted the two toasts"
    );
    harness.run_steps(2);

    let toasts = harness
        .ctx
        .memory(|memory| memory.area_rect("moonreview-toasts"))
        .expect("expected the toasts to have been drawn");
    assert!(
        toasts.left() < 40.0 && toasts.right() < WINDOW.x / 2.0,
        "the toasts should stand at the left edge, not at {toasts:?}"
    );
    assert!(
        toasts.bottom() > WINDOW.y - 60.0,
        "the toasts should stand along the bottom, not at {toasts:?}"
    );

    harness.snapshot("toasts-bottom-left");
}
