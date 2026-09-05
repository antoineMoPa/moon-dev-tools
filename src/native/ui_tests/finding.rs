//! The find bar: what ⌘F searches, what it marks, and where it scrolls to.
//!
//! Over a review it searches every hunk being shown rather than the lines on screen; over an
//! open file it marks the matches in the text, walks them only while the query box is the
//! thing being typed into, and brings the file to a match below the fold. A file opened at a
//! line a content search found lands on it and is marked the same way, which is why that one
//! is here rather than beside the rest of the opening.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, press_key, seeded_fixture, settle};

/// A file opened at one of the lines a content search found is scrolled to that line rather
/// than left at the top with the match far below the fold, and the text that was searched
/// for is marked there the way the find bar marks it.
#[test]
fn a_file_opened_at_a_match_is_scrolled_to_it_and_marks_it() {
    let fixture = Fixture::new("file-pane-at-a-line");
    let mut text = String::new();
    for line in 1..=200 {
        text.push_str(&format!("pub const LINE_{line}: u32 = {line};\n"));
    }
    fixture.write("src/lines.rs", &text);
    fixture.commit("Add the lines");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_pane(crate::native::panes::OpenPaneRequest::File {
                    session_id,
                    file_path: "src/lines.rs".to_string(),
                    at: Some(crate::native::panes::OpenAt {
                        line: 150,
                        query: "LINE_150".to_string(),
                    }),
                });
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the file tab never opened"
    );

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    harness.snapshot("file-pane-at-a-match");
}

/// ⌘F over a review searches every hunk it is showing, not only the lines on screen, and
/// stepping through the matches moves the current one.
#[test]
fn find_searches_a_whole_review_and_steps_through_the_matches() {
    let fixture = seeded_fixture("find-review");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    /// What the test reads back out of the window each frame.
    #[derive(Default, Clone)]
    struct Seen {
        open: bool,
        query: String,
        total: usize,
        at: usize,
        current_hunk: Option<String>,
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let session_id = app.model.root_session_id.clone();
            let current_hunk = app
                .model
                .review_ref(&session_id)
                .and_then(|review| review.find_match.as_ref())
                .map(|found| found.hunk_id.clone());
            *seen_in_ui.lock().expect("poisoned") = Seen {
                open: app.model.find.is_some(),
                query: app
                    .model
                    .find
                    .as_ref()
                    .map(|find| find.query.clone())
                    .unwrap_or_default(),
                total: app.model.find.as_ref().map(|find| find.total).unwrap_or(0),
                at: app.model.find.as_ref().map(|find| find.at).unwrap_or(0),
                current_hunk,
            };
            ready_in_ui.store(
                app.model
                    .review_ref(&session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    let loaded = settle(&mut harness, || ready.load(Ordering::Relaxed));
    assert!(loaded, "the review never loaded");
    harness.run_steps(2);
    assert!(!seen.lock().expect("poisoned").open, "no bar before ⌘F");

    press_key(&mut harness, egui::Key::F, egui::Modifiers::COMMAND);
    assert!(
        seen.lock().expect("poisoned").open,
        "⌘F should have opened the find bar"
    );

    // Typed into the bar, which took the keyboard when it opened.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("values".to_string()));
    harness.step();
    harness.run_steps(3);

    let after_typing = seen.lock().expect("poisoned").clone();
    assert_eq!(after_typing.query, "values");
    // `values` is all over the fixture's second function, on both sides of the diff.
    assert!(
        after_typing.total >= 2,
        "the search should have found every hunk's matches, got {}",
        after_typing.total
    );
    assert_eq!(after_typing.at, 0, "and started on the first one");
    assert!(
        after_typing.current_hunk.is_some(),
        "the review should know which match it is on"
    );

    // What the bar and the marked matches actually look like over a review.
    harness.snapshot("find-bar");

    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    let after_step = seen.lock().expect("poisoned").clone();
    assert_eq!(after_step.at, 1, "Enter steps to the next match");
    assert_eq!(
        after_step.total, after_typing.total,
        "stepping does not change what was found"
    );

    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(
        !seen.lock().expect("poisoned").open,
        "Escape should have put the bar away"
    );
}

/// A search over an open file marks what it found in the text, and Enter only walks those
/// matches while the query box is the thing being typed into. It used to step on any Enter
/// the window saw, so typing into the file - or the shell in the next split - dragged the
/// search along behind it; and the marks were the editor's own selection, which an unfocused
/// editor does not paint at all, so a search over a file showed nothing.
#[test]
fn find_marks_a_file_and_steps_only_while_the_query_box_has_the_keyboard() {
    let fixture = Fixture::new("file-find");
    fixture.write(
        "src/lib.rs",
        "pub fn one() {}\npub fn two() {}\npub fn three() {}\n",
    );
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);

    /// What the test reads back out of the window each frame.
    #[derive(Default, Clone)]
    struct Seen {
        total: usize,
        at: usize,
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            if let Some((pane_id, _)) = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
            {
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&pane_id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            *seen_in_ui.lock().expect("poisoned") = Seen {
                total: app.model.find.as_ref().map(|find| find.total).unwrap_or(0),
                at: app.model.find.as_ref().map(|find| find.at).unwrap_or(0),
            };
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    press_key(&mut harness, egui::Key::F, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("pub fn".to_string()));
    harness.step();
    harness.run_steps(3);

    let after_typing = seen.lock().expect("poisoned").clone();
    assert_eq!(after_typing.total, 3, "every line of the file matches");
    assert_eq!(after_typing.at, 0, "and the search starts on the first one");

    // Every match marked in the text, the current one more strongly than the rest.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-find-marks");

    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        seen.lock().expect("poisoned").at,
        1,
        "Enter in the query box steps to the next match"
    );

    // The keyboard goes back to the file under the bar - an Enter typed there is a newline,
    // and none of the search's business.
    harness.ctx.memory_mut(|memory| memory.stop_text_input());
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        seen.lock().expect("poisoned").at,
        1,
        "an Enter aimed elsewhere should have left the search where it was"
    );
}

/// A match below the fold brings the file to it. The scroll was asked for from inside the
/// code's own sideways scroll area, which takes both axes' targets and drops the one it has
/// no bar for, so a match off the bottom of the pane was marked where nobody could see it.
#[test]
fn find_scrolls_a_file_to_a_match_below_the_fold() {
    let fixture = Fixture::new("file-find-scroll");
    let mut lines = vec!["pub fn filler() {}".to_string(); 200];
    lines.push("pub fn the_needle() {}".to_string());
    fixture.write("src/lib.rs", &format!("{}\n", lines.join("\n")));
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let total = Arc::new(Mutex::new(0usize));
    let total_in_ui = Arc::clone(&total);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            if let Some((pane_id, _)) = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
            {
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&pane_id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            *total_in_ui.lock().expect("poisoned") =
                app.model.find.as_ref().map(|find| find.total).unwrap_or(0);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    press_key(&mut harness, egui::Key::F, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("the_needle".to_string()));
    harness.step();
    harness.run_steps(6);

    assert_eq!(
        *total.lock().expect("poisoned"),
        1,
        "the needle is in the file exactly once"
    );
    // The one match, on screen rather than two hundred lines below it.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-find-scrolled");
}
