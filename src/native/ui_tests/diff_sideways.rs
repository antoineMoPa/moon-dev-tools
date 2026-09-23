//! Scrolling a hunk's code sideways to read a line longer than the card is wide.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{model::LineSelection, theme::ThemeMode};

use super::{Fixture, app_for};

/// What the test reads back out of the app the harness owns.
#[derive(Default)]
struct Seen {
    long_hunk_id: Option<String>,
    short_hunk_id: Option<String>,
    patch: String,
    short_patch: String,
    long_scroll_x: f32,
    short_scroll_x: f32,
    selection: Option<LineSelection>,
}

/// A swipe to the left over a hunk with a line wider than the card scrolls its code, and only
/// its code: a hunk with nothing past its edge stays put, and a double-click on the scrolled
/// row takes the word now under the pointer rather than the one that was there before.
#[test]
fn a_sideways_swipe_scrolls_the_code_of_a_hunk_wider_than_its_card() {
    let fixture = Fixture::new("diff-sideways");
    fixture.write("src/long.rs", "fn keep() {}\n");
    fixture.write("src/short.rs", "fn keep() {}\n");
    fixture.commit("Add the files");
    let long_line: String = (0..80).map(|index| format!("word{index} ")).collect();
    fixture.write("src/long.rs", &format!("fn keep() {{}}\n{long_line}\n"));
    fixture.write("src/short.rs", "fn keep() {}\nfn short() {}\n");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            let hunk_of = |path: &str| review.hunks().iter().find(|hunk| hunk.file_path == path);
            let mut seen = seen_in_ui.lock().expect("poisoned");
            if let (Some(long), Some(short)) = (hunk_of("src/long.rs"), hunk_of("src/short.rs")) {
                seen.long_hunk_id = Some(long.id.clone());
                seen.short_hunk_id = Some(short.id.clone());
                seen.patch = long.patch_preview.clone();
                seen.short_patch = short.patch_preview.clone();
                seen.long_scroll_x = review.code_scroll_x.get(&long.id).copied().unwrap_or(0.0);
                seen.short_scroll_x = review.code_scroll_x.get(&short.id).copied().unwrap_or(0.0);
                ready_in_ui.store(true, Ordering::Relaxed);
            }
            seen.selection = review.selection;
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(3);

    let (long_hunk_id, short_hunk_id, patch, short_patch) = {
        let seen = seen.lock().expect("poisoned");
        (
            seen.long_hunk_id.clone().expect("expected the long hunk"),
            seen.short_hunk_id.clone().expect("expected the short hunk"),
            seen.patch.clone(),
            seen.short_patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let long_index = lines
        .iter()
        .position(|line| line.body() == long_line)
        .expect("expected the long line in the patch");
    let row_of = |harness: &Harness<'_>, hunk_id: &str, index: usize| {
        harness
            .ctx
            .read_response(crate::native::review::hunks::diff_line_id(hunk_id, index))
            .expect("expected the diff line to have been drawn")
            .rect
    };
    let long_row = row_of(&harness, &long_hunk_id, long_index);
    let short_lines = crate::native::review::diff::build_diff_lines(&short_patch);
    let short_index = short_lines
        .iter()
        .position(|line| line.body() == "fn short() {}")
        .expect("expected the short line");
    let short_row = row_of(&harness, &short_hunk_id, short_index);

    // Content moving left is a negative x, as a two-finger swipe to the left sends it.
    let swipe_left_over = |harness: &mut Harness<'_>, at: egui::Pos2| {
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-240.0, 0.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        // The wheel is smoothed over a few frames before all of it has arrived.
        harness.run_steps(30);
    };

    swipe_left_over(&mut harness, short_row.center());
    assert_eq!(
        seen.lock().expect("poisoned").short_scroll_x,
        0.0,
        "a hunk with nothing past its edge has nothing to scroll to"
    );

    swipe_left_over(&mut harness, long_row.center());
    let scrolled = seen.lock().expect("poisoned").long_scroll_x;
    assert!(
        scrolled > 100.0,
        "the long hunk should have scrolled, got {scrolled}"
    );

    // Where the first word used to be; it has slid off to the left since.
    let at = egui::pos2(
        crate::native::review::hunks::body_text_x(long_row, 0.0) + 10.0,
        long_row.center().y,
    );
    let press_and_release = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        press_and_release(true),
        press_and_release(false),
    ]);
    harness.step();
    harness
        .input_mut()
        .events
        .extend([press_and_release(true), press_and_release(false)]);
    harness.run_steps(2);

    let selection = seen
        .lock()
        .expect("poisoned")
        .selection
        .expect("a double-click should have selected a word");
    let (from, to) = selection
        .columns_on(long_index)
        .expect("the word should be on the long line");
    // Unscrolled, ten points in is the first word, which starts the line.
    assert!(
        from > 20,
        "expected a word from further along the line, got columns {from}..{to}"
    );
}
