//! What right-clicking a file in the review sidebar can do to the whole of it.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use super::{app_for, seeded_fixture, settle};
use crate::native::theme::ThemeMode;

/// Press and release the secondary button at a position, then let the UI settle.
fn right_click_at(harness: &mut Harness<'_>, at: egui::Pos2) {
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();
    harness.run_steps(2);
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Staging {
    hunks: usize,
    staged: usize,
}

#[test]
fn the_file_menu_stages_and_discards_the_whole_file() {
    let fixture = seeded_fixture("file-menu");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    let staging = Arc::new(Mutex::new(Staging::default()));
    let staging_in_ui = Arc::clone(&staging);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            let of_file = review
                .hunks()
                .iter()
                .filter(|hunk| hunk.file_path == "src/lib.rs")
                .fold(Staging::default(), |mut seen, hunk| {
                    seen.hunks += 1;
                    seen.staged += usize::from(hunk.staged);
                    seen
                });
            *staging_in_ui.lock().expect("poisoned") = of_file;
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    let dot = harness
        .ctx
        .read_response(crate::native::review::sidebar::stage_dot_id("src/lib.rs"))
        .expect("expected the file row's staging dot to have been drawn")
        .rect;
    let row = egui::pos2(dot.center().x + 70.0, dot.center().y);

    right_click_at(&mut harness, row);
    let menu = harness.query_by_label("stage the whole file").is_some();
    assert!(menu, "right-clicking a file should open its menu");

    harness.get_by_label("stage the whole file").click();
    harness.run_steps(2);
    let staged = settle(&mut harness, || {
        let seen = *staging.lock().expect("poisoned");
        seen.hunks > 0 && seen.staged == seen.hunks
    });
    assert!(
        staged,
        "the menu should have staged the whole file, saw {:?}",
        *staging.lock().expect("poisoned")
    );

    right_click_at(&mut harness, row);
    assert!(
        harness.query_by_label("discard the whole file").is_some(),
        "the menu should offer discarding the file"
    );
    harness.get_by_label("discard the whole file").click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label("[really discard the whole file]")
            .is_some(),
        "arming the discard should ask for confirmation in the open menu"
    );

    harness.get_by_label("[really discard the whole file]").click();
    let discarded = settle(&mut harness, || {
        std::fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("failed to read")
            .contains("pub fn count")
            .eq(&false)
    });
    assert!(discarded, "confirming should have reverted the file");
}
