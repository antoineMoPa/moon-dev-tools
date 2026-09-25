//! The review on a phone: one of the sidebar and the diff at a time, one line number, and the
//! code carried sideways under a finger.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::native::theme::ThemeMode;

use super::{Fixture, app_for, finger};

#[derive(Default)]
struct Seen {
    long_hunk_id: Option<String>,
    patch: String,
    scroll_x: f32,
    selected: bool,
    sidebar_in_front: bool,
}

#[test]
fn a_review_on_a_phone_shows_one_thing_at_a_time_and_scrolls_under_a_finger() {
    let fixture = Fixture::new("review-phone");
    fixture.write("src/long.rs", "fn keep() {}\n");
    fixture.commit("Add the file");
    let long_line: String = (0..80).map(|index| format!("word{index} ")).collect();
    fixture.write("src/long.rs", &format!("fn keep() {{}}\n{long_line}\n"));

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(390.0, 800.0))
        .build_ui(move |ui| {
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            let mut seen = seen_in_ui.lock().expect("poisoned");
            if let Some(long) = review
                .hunks()
                .iter()
                .find(|hunk| hunk.file_path == "src/long.rs")
            {
                seen.long_hunk_id = Some(long.id.clone());
                seen.patch = long.patch_preview.clone();
                seen.scroll_x = review.code_scroll_x.get(&long.id).copied().unwrap_or(0.0);
                ready_in_ui.store(true, Ordering::Relaxed);
            }
            seen.selected = review.selection.is_some();
            seen.sidebar_in_front = review.sidebar_in_front;
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(3);

    let (hunk_id, patch) = {
        let seen = seen.lock().expect("poisoned");
        (
            seen.long_hunk_id.clone().expect("expected the hunk"),
            seen.patch.clone(),
        )
    };
    let index = crate::native::review::diff::build_diff_lines(&patch)
        .iter()
        .position(|line| line.body() == long_line)
        .expect("expected the long line");
    let row = harness
        .ctx
        .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, index))
        .expect("expected the line drawn")
        .rect;

    assert!(
        row.left() < 60.0,
        "the diff should have the pane to itself, its rows start at {}",
        row.left()
    );
    let code_starts = crate::native::review::hunks::body_text_x(row, 0.0) - row.left();
    assert!(
        code_starts < 60.0,
        "one line number leaves the code most of the row, it starts {code_starts} in"
    );

    // A finger carried to the left along the long line.
    let from = row.center();
    let to = from - egui::vec2(150.0, 0.0);
    finger(&mut harness, from, egui::TouchPhase::Start);
    for step in 1..=6 {
        finger(
            &mut harness,
            from + (to - from) * (step as f32 / 6.0),
            egui::TouchPhase::Move,
        );
    }
    finger(&mut harness, to, egui::TouchPhase::End);
    harness.run_steps(2);
    {
        let seen = seen.lock().expect("poisoned");
        assert!(
            seen.scroll_x > 100.0,
            "the code should have scrolled under the finger, got {}",
            seen.scroll_x
        );
        assert!(!seen.selected, "a drag under a finger selects nothing");
    }

    harness.get_by_label("[files]").click();
    harness.run_steps(2);
    assert!(
        seen.lock().expect("poisoned").sidebar_in_front,
        "the files take the pane"
    );
    harness.get_by_label("[diff]").click();
    harness.run_steps(2);
    assert!(
        !seen.lock().expect("poisoned").sidebar_in_front,
        "and give it back"
    );
}
