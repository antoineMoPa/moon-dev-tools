//! The minimap beside a review's diff: the whole review down a strip, drawn by the map a
//! file tab has, and the diff scrolled from it.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{
    review::{diff::build_diff_lines, hunks::diff_line_id, minimap},
    theme::ThemeMode,
};

use super::{Fixture, app_for};

/// How many files the review is of, and how many lines each adds: between them a good deal
/// more than a window shows at once.
const FILES: usize = 8;
const LINES_A_FILE: usize = 60;

/// What the test reads back out of the app the harness owns.
#[derive(Default)]
struct Seen {
    session_id: String,
    /// The last line of the last file: the bottom of the review.
    last_line: Option<egui::Id>,
    scrolled: minimap::Scrolled,
    times_folded: usize,
}

/// A review of [`FILES`] new files, in a window `size`, drawn until it has loaded.
fn harness_on_a_tall_review(
    fixture: &Fixture,
    size: egui::Vec2,
) -> (Harness<'static>, Arc<Mutex<Seen>>) {
    fixture.write("README.md", "# fixture\n");
    fixture.commit("Add the readme");
    for file in 0..FILES {
        let lines: String = (0..LINES_A_FILE)
            .map(|line| format!("pub fn function_{file}_{line}() {{}}\n"))
            .collect();
        fixture.write(&format!("src/file_{file}.rs"), &lines);
    }
    let last_path = format!("src/file_{}.rs", FILES - 1);
    let last_body = format!("pub fn function_{}_{}() {{}}", FILES - 1, LINES_A_FILE - 1);

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(size)
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let session_id = app.model.root_session_id.clone();
            let Some(review) = app.model.review_ref(&session_id) else {
                return;
            };
            let mut seen = seen_in_ui.lock().expect("poisoned");
            seen.scrolled = review.minimap.scrolled;
            seen.times_folded = review.minimap.times_folded;
            if let Some(last) = review
                .hunks()
                .iter()
                .find(|hunk| hunk.file_path == last_path)
            {
                let index = build_diff_lines(&last.patch_preview)
                    .iter()
                    .position(|line| line.body() == last_body)
                    .expect("expected the last line of the last file in its patch");
                seen.last_line = Some(diff_line_id(&last.id, index));
                seen.session_id = session_id;
                ready_in_ui.store(true, Ordering::Relaxed);
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    // Laid out, and read as code.
    harness.run_steps(8);
    (harness, seen)
}

fn scrolled(seen: &Arc<Mutex<Seen>>) -> minimap::Scrolled {
    seen.lock().expect("poisoned").scrolled
}

fn strip_of(harness: &Harness<'_>, seen: &Arc<Mutex<Seen>>) -> Option<egui::Rect> {
    let session_id = seen.lock().expect("poisoned").session_id.clone();
    harness
        .ctx
        .read_response(minimap::strip_id(&session_id))
        .map(|response| response.rect)
}

fn pointer_button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The strip runs down the right of the review. A press on it brings the diff to what is
/// drawn there, a drag carries the diff along, and none of that scrolling folds the map
/// over again.
#[test]
fn the_minimap_scrolls_the_diff_to_where_it_is_pressed_and_dragged() {
    let fixture = Fixture::new("review-minimap");
    let window = egui::vec2(1400.0, 880.0);
    let (mut harness, seen) = harness_on_a_tall_review(&fixture, window);

    let strip = strip_of(&harness, &seen).expect("expected the minimap beside the diff");
    let as_wide_as_a_file_tab_s = crate::native::theme::Palette::of(ThemeMode::Dark)
        .editor_style()
        .minimap_width;
    assert!(
        strip.width() == as_wide_as_a_file_tab_s
            && strip.right() > window.x - 20.0
            && strip.height() > 700.0,
        "the strip should run down the right of the window, it is at {strip:?}"
    );

    let at_first = scrolled(&seen);
    assert_eq!(at_first.offset, 0.0);
    let furthest = at_first.content_height - at_first.view_height;
    assert!(
        furthest > 1000.0,
        "the review should be a good deal taller than the pane, it can scroll {furthest}"
    );
    let last_line = seen
        .lock()
        .expect("poisoned")
        .last_line
        .expect("expected the last line");
    assert!(
        harness.ctx.read_response(last_line).is_none(),
        "the bottom of the review should be out of sight to begin with"
    );
    let times_folded = seen.lock().expect("poisoned").times_folded;
    assert!(times_folded > 0, "the strip should have drawn the review");

    // The review is taller than the strip can show at two points a row, so it is fitted to
    // the strip, and the strip's bottom is the review's.
    let bottom = strip.center_bottom() - egui::vec2(0.0, 3.0);
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(bottom),
        pointer_button(bottom, true),
    ]);
    harness.run_steps(2);
    assert!(
        (scrolled(&seen).offset - furthest).abs() < 1.0,
        "a press at the strip's bottom should scroll the diff to its end, it is at {} of {furthest}",
        scrolled(&seen).offset
    );
    let last_row = harness
        .ctx
        .read_response(last_line)
        .expect("the last line of the review should be drawn now")
        .rect;
    assert!(
        last_row.bottom() <= window.y,
        "and on the screen, it is at {last_row:?}"
    );

    // Carried half way up the strip, the diff shows the middle of the review.
    for step in 1..=5 {
        let at = bottom + (strip.center() - bottom) * (step as f32 / 5.0);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.step();
    }
    harness.step();
    let carried = scrolled(&seen).offset;
    assert!(
        (carried - furthest / 2.0).abs() < 20.0,
        "the pointer half way up the strip should leave the diff half way, it is at {carried} of {furthest}"
    );
    // And on past the top of the strip, where the diff stops at its own top.
    let above = strip.center_top() - egui::vec2(0.0, 40.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(above));
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(pointer_button(above, false));
    harness.run_steps(3);
    assert_eq!(scrolled(&seen).offset, 0.0);

    // The wheel over the diff scrolls it, the map's slider with it.
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(egui::pos2(700.0, 500.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -300.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    // The wheel is smoothed over a few frames before all of it has arrived.
    harness.run_steps(30);
    let wheeled = scrolled(&seen).offset;
    assert!(
        wheeled > 200.0,
        "the wheel should scroll the diff, got {wheeled}"
    );

    assert_eq!(
        seen.lock().expect("poisoned").times_folded,
        times_folded,
        "scrolling moves the slider over the map, it does not fold the map again"
    );
}

/// How much ink the strip is drawn in: the green of its pixels, the review being all
/// added lines.
fn ink_on(harness: &mut Harness<'_>, strip: egui::Rect) -> u64 {
    let picture = harness.render().expect("the window should render");
    let mut green = 0;
    for y in strip.top() as u32..strip.bottom() as u32 {
        for x in strip.left() as u32..strip.right() as u32 {
            green += u64::from(picture.get_pixel(x, y).0[1]);
        }
    }
    green
}

/// The strip is the map a file tab has, and like it is a hint at the edge of the page until
/// the pointer is on it.
#[test]
fn the_minimap_is_dimmed_until_the_pointer_is_on_it() {
    let fixture = Fixture::new("review-minimap-dimmed");
    let (mut harness, seen) = harness_on_a_tall_review(&fixture, egui::vec2(1400.0, 880.0));
    let strip = strip_of(&harness, &seen).expect("expected the minimap beside the diff");

    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(700.0, 500.0)));
    harness.run_steps(4);
    let dimmed = ink_on(&mut harness, strip);

    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(strip.center()));
    harness.run_steps(4);
    let lit = ink_on(&mut harness, strip);

    assert!(
        lit as f64 > dimmed as f64 * 1.3,
        "the strip should be brighter under the pointer: {dimmed} away from it, {lit} on it"
    );
}

/// A pane too narrow to spare the strip its width has none.
#[test]
fn a_narrow_review_has_no_minimap() {
    let fixture = Fixture::new("review-minimap-narrow");
    let (harness, seen) = harness_on_a_tall_review(&fixture, egui::vec2(760.0, 880.0));

    assert_eq!(strip_of(&harness, &seen), None);
    assert!(
        scrolled(&seen).content_height > 1000.0,
        "the diff still lays the review out"
    );
}
