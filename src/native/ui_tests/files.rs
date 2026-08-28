//! File tabs: opening, rendering, editing, and finding text in a review.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::{
    api::OpenSessionRequest,
    backend::local::LocalBackend,
    native::{Launch, app::App, panes::Pane, theme::ThemeMode},
};

use super::{Fixture, app_for, harness_with_loaded_review, seeded_fixture, settle, press_key};

/// The file tab: a fringe of line numbers beside the text, and the text editable.
#[test]
fn a_file_opens_in_a_tab_of_its_own() {
    let fixture = Fixture::new("file-pane");
    // One line far wider than the pane: it must scroll sideways rather than wrap, and the
    // line numbers must stay put while it does.
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n\npub fn total(values: &[u32], and_a_very_long_parameter_list: &[u32], so_that_this_line_runs_well_past_the_edge_of_the_pane: bool) -> u32 {\n    values.iter().sum()\n}\n",
    );
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
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
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .any(|(_, pane)| matches!(pane, Pane::File { .. }))
                    && app
                        .model
                        .file_editors
                        .values()
                        .any(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the file tab never opened");

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    harness.snapshot("file-pane");
}

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

/// A markdown file opens on the rendered page, and `[edit]` is the way back to the text.
#[test]
fn a_markdown_file_opens_rendered() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-markdown");
    fixture.write(
        "NOTES.md",
        "# The plan\n\nShip it by *Friday*, with:\n\n- a heading\n- emphasis\n- this list\n",
    );
    fixture.commit("Add the notes");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "NOTES.md");
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

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the file tab never opened");
    harness.run_steps(3);

    // Rendered: the page, not the text of it - so no line-number fringe, and the way back is
    // on screen.
    assert!(
        harness.query_by_label("[edit]").is_some(),
        "a rendered markdown file should offer the way back to the text"
    );
    harness.snapshot("file-pane-markdown");

    harness.get_by_label("[edit]").click();
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    assert!(
        harness.query_by_label("[preview]").is_some(),
        "the text view should offer the rendered page back"
    );
    harness.snapshot("file-pane-markdown-source");
}

/// Editing a file tab writes the file back, and the tab says so until it does.
#[test]
fn editing_a_file_tab_saves_it_to_the_working_tree() {
    let fixture = Fixture::new("file-save");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let pane_id = Arc::new(Mutex::new(None::<PaneId>));
    let pane_in_ui = Arc::clone(&pane_id);
    let dirty = Arc::new(AtomicBool::new(false));
    let dirty_in_ui = Arc::clone(&dirty);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let save = Arc::new(AtomicBool::new(false));
    let save_in_ui = Arc::clone(&save);
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
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if let Some(text) = edit_in_ui.lock().expect("poisoned").take()
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
                && let Some(editor) = app.model.file_editors.get_mut(&id)
            {
                editor.edit_for_test(&text);
            }
            if save_in_ui.swap(false, Ordering::Relaxed)
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
            {
                let session_id = app.model.root_session_id.clone();
                app.save_file_pane(id, &session_id);
            }

            app.draw(ui);

            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(id) = open_pane {
                dirty_in_ui.store(app.file_pane_is_dirty(id), Ordering::Relaxed);
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            *pane_in_ui.lock().expect("poisoned") = open_pane;
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !loaded.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(loaded.load(Ordering::Relaxed), "the file never loaded");
    assert!(
        !dirty.load(Ordering::Relaxed),
        "a freshly opened file is clean"
    );

    *edit.lock().expect("poisoned") = Some("pub fn two() {}\n".to_string());
    harness.run_steps(2);
    assert!(dirty.load(Ordering::Relaxed), "an edit should mark the tab");
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn one() {}\n",
        "nothing should reach the file until it is saved"
    );

    // The tab carries a dot for as long as the edit is not on disk.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-tab-unsaved");

    save.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && dirty.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        !dirty.load(Ordering::Relaxed),
        "saving should clear the mark"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn two() {}\n",
        "the edit should be on disk"
    );
}

/// Pointed at a file nobody has touched, the review shows the file itself rather than an
/// empty diff - `moonreview package.json` is a request to read it.
#[test]
fn an_unchanged_file_opens_as_the_file_itself() {
    let fixture = Fixture::new("unchanged-file");
    fixture.write("package.json", "{\n  \"name\": \"fixture\"\n}\n");
    fixture.commit("Add the manifest");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let launch = Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: Some(crate::api::DiffTarget {
                base: None,
                pathspec: Some("package.json".to_string()),
                comparison: None,
            }),
            active_commit: None,
        }),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);

    let shown = Arc::new(Mutex::new(None::<String>));
    let shown_in_ui = Arc::clone(&shown);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            // The file opens as a tab of its own, so what it is showing is looked for there.
            *shown_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id)
                .and_then(|pane_id| app.model.file_editors.get(&pane_id))
                .and_then(|editor| editor.content_for_test());
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if shown.lock().expect("poisoned").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let content = shown.lock().expect("poisoned").clone();
    assert_eq!(
        content.as_deref(),
        Some("{\n  \"name\": \"fixture\"\n}\n"),
        "the file's own text should be on screen"
    );
}

/// A changed image is shown as before and after pictures. They arrive as `data:` URIs, which
/// egui has no loader for, so this is where a decoding regression turns back into an error box.
#[test]
fn a_changed_image_is_drawn_as_before_and_after() {
    let fixture = Fixture::new("image-diff");
    fixture.write("README.md", "# fixture\n");
    fixture.write_png("logo.png", [200, 90, 40, 255]);
    fixture.commit("Add the logo");
    fixture.write_png("logo.png", [40, 120, 200, 255]);

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);
    // Decoding and uploading the textures takes passes of their own after the diff arrives -
    // and on a machine busy running the rest of this suite, more than a fixed few.
    harness.run_steps(3);
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.ctx.has_pending_images() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run_steps(2);

    harness.snapshot("image-diff");
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

/// The same as far as the user is concerned: type into the file, then press the pane's own
/// [save] button and cmd+s, which is how the edit actually gets asked for.
#[test]
fn the_save_button_and_the_chord_write_the_file() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-save-button");
    fixture.write("src/lib.rs", "one\n");
    fixture.commit("Add the library");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);

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
            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(id) = open_pane {
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    // Type at the end of the text, the way clicking into the file and typing does.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let text = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
    text.click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::X, "x");

    assert!(
        harness.query_by_label("[save]").is_some(),
        "an edited file should offer [save]"
    );

    // cmd+s first, with the keyboard still in the text where typing left it.
    press_key(&mut harness, egui::Key::S, egui::Modifiers::COMMAND);
    let saved_by_chord = settle(&mut harness, || {
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read") != "one\n"
    });
    assert!(
        saved_by_chord,
        "cmd+s should have written the file, saw {:?}",
        fs::read_to_string(fixture.root.join("src/lib.rs"))
    );

    // Then the button, on a second edit.
    let text = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
    text.click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::Y, "y");
    harness.get_by_label("[save]").click();
    let written = settle(&mut harness, || {
        fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("failed to read")
            .contains('y')
    });
    assert!(
        written,
        "clicking [save] should have written the file, saw {:?}",
        fs::read_to_string(fixture.root.join("src/lib.rs"))
    );
}
