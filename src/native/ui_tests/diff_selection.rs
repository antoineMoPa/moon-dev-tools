//! Selecting diff lines with the mouse, copying what was selected, and ⌘-clicking a name on
//! one to land on where it is defined.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{
    Fixture, app_for, click_at, click_like_a_hand, press_modifiers, seeded_fixture, settle,
};

/// cmd+c over the diff copies what is selected - and copies the code, without the `+` that
/// says it was added. A clicked line is selected whole, so that is what arrives.
#[test]
fn copy_takes_the_selected_diff_lines_without_their_diff_markers() {
    let fixture = seeded_fixture("copy-diff");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        copied: Option<String>,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
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
            if let Ok(mut seen) = seen_in_ui.lock() {
                seen.hunk_id = review.hunks().first().map(|hunk| hunk.id.clone());
                if let Some(hunk) = review.hunks().first() {
                    seen.patch = hunk.patch_preview.clone();
                }
                if let Some(text) = ui.ctx().output(|output| {
                    output.commands.iter().find_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    })
                }) {
                    seen.copied = Some(text);
                }
            }
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(2);

    let (hunk_id, patch) = {
        let state = seen.lock().expect("expected the hunk");
        (
            state.hunk_id.clone().expect("expected a hunk to copy from"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, raw) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.text.clone()))
        .expect("expected an added line to copy");

    let target = crate::native::review::hunks::diff_line_id(&hunk_id, line_index);
    let rect = harness
        .ctx
        .read_response(target)
        .expect("expected the diff line to have been drawn")
        .rect;
    click_at(&mut harness, rect.center());
    harness.run_steps(2);

    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    harness.run_steps(2);

    let copied = seen
        .lock()
        .expect("poisoned")
        .copied
        .clone()
        .expect("cmd+c over a selected diff line should have copied it");
    assert_eq!(
        copied,
        raw[1..],
        "the code should arrive without the `+` in front of it"
    );
}

/// Dragging down a hunk selects the whole run of lines the pointer swept over, and the
/// composer opens on that run once the button comes up.
#[test]
fn dragging_across_diff_lines_selects_the_run() {
    let fixture = seeded_fixture("multi-select");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selected: Option<(usize, usize)>,
        draft_selection: Option<String>,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
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
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.selected = review.selection.map(|selection| {
                    (
                        *selection.line_range().start(),
                        *selection.line_range().end(),
                    )
                });
                seen.draft_selection = review.drafts.first().map(|draft| draft.selection.clone());
            }
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(2);

    let (hunk_id, patch) = {
        let state = seen.lock().expect("expected the hunk");
        (
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.kind.commentable())
        .map(|(index, _)| index)
        .collect();
    assert!(
        changed.len() >= 3,
        "the fixture needs a few lines to sweep over"
    );
    let (from, to) = (changed[0], changed[2]);

    let rect_of = |harness: &Harness<'_>, index: usize| {
        harness
            .ctx
            .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, index))
            .expect("expected the diff line to have been drawn")
            .rect
    };
    let start = rect_of(&harness, from).center();
    let end = rect_of(&harness, to).center();

    // Press on the first line, sweep to the third, release.
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();
    for at in [start + egui::vec2(0.0, 6.0), end] {
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.step();
    }
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);

    let state = seen.lock().expect("expected state");
    assert_eq!(
        state.selected,
        Some((from, to)),
        "the sweep should select every line from the first to the last"
    );
    let selection = state
        .draft_selection
        .clone()
        .expect("the composer should open on the swept run");
    assert_eq!(
        selection.lines().count(),
        to - from + 1,
        "the comment is anchored to every swept line, got {selection:?}"
    );
    // The anchor text is raw patch lines, which is what a partial stage matches against.
    let expected: Vec<&str> = (from..=to)
        .map(|index| lines[index].text.as_str())
        .collect();
    assert_eq!(selection, expected.join("\n"));
    drop(state);

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("multi-line-selection");
}

/// Double-clicking a word in a diff selects just that word, and cmd+c copies exactly it.
#[test]
fn double_clicking_a_word_selects_and_copies_it() {
    let fixture = seeded_fixture("word-select");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selection: Option<crate::native::model::LineSelection>,
        copied: Option<String>,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
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
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.selection = review.selection;
                if let Some(text) = ui.ctx().output(|output| {
                    output.commands.iter().find_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    })
                }) {
                    seen.copied = Some(text);
                }
            }
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(2);

    let (hunk_id, patch) = {
        let state = seen.lock().expect("expected the hunk");
        (
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, body) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.body().to_string()))
        .expect("expected an added line");

    let rect = harness
        .ctx
        .read_response(crate::native::review::hunks::diff_line_id(
            &hunk_id, line_index,
        ))
        .expect("expected the diff line to have been drawn")
        .rect;
    // A few pixels into the line's first word - the row is as wide as the pane, and a
    // double-click past the end of the text falls back to the whole line.
    let at = egui::pos2(
        crate::native::review::hunks::body_text_x(rect) + 10.0,
        rect.center().y,
    );
    // Two clicks one step apart: the harness steps a quarter second at a time, and egui
    // counts a double-click only inside 0.3s, so anything looser reads as two single clicks.
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
    harness.step();
    harness.run_steps(2);

    let selection = seen
        .lock()
        .expect("poisoned")
        .selection
        .expect("the double-click should have selected");
    assert_eq!(
        selection.line_range().count(),
        1,
        "a word lives on one line"
    );
    let (from, to) = selection
        .columns_on(line_index)
        .expect("the selection is on the clicked line");
    assert!(
        to < crate::native::model::LINE_END && to <= body.chars().count(),
        "a word selection ends inside the line"
    );
    assert!(from < to, "a word selection covers characters");

    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    harness.run_steps(2);

    let copied = seen
        .lock()
        .expect("poisoned")
        .copied
        .clone()
        .expect("cmd+c should have copied the word");
    let expected: String = body.chars().skip(from).take(to - from).collect();
    assert_eq!(copied, expected, "what copies is exactly the selected word");
    assert!(
        !copied.trim().is_empty(),
        "the middle of a code line is a word, not blank space"
    );
}

/// A repo whose diff has a row calling a name that is defined in another file: the review is
/// where code is really read, so following a name out of a diff row has to work there.
///
/// The call is the whole of the added line, so the click lands inside the name at a spot that
/// does not depend on how the row was laid out.
fn calling_fixture(name: &str) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    );
    fixture.write("src/main.rs", "fn main() {\n}\n");
    fixture.commit("Add the library and the program");
    fixture.write("src/main.rs", "fn main() {\ngreet(\"moon\");\n}\n");
    fixture
}

/// The added row of the one hunk, and where in the window it was drawn.
fn added_row_at(harness: &egui_kittest::Harness<'_>, hunk_id: &str, patch: &str) -> egui::Pos2 {
    let lines = crate::native::review::diff::build_diff_lines(patch);
    let (index, _) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .expect("expected an added line in the diff");
    let rect = harness
        .ctx
        .read_response(crate::native::review::hunks::diff_line_id(hunk_id, index))
        .expect("expected the diff line to have been drawn")
        .rect;
    // A few pixels into the row's first word, which is the name being called.
    egui::pos2(
        crate::native::review::hunks::body_text_x(rect) + 10.0,
        rect.center().y,
    )
}

/// What the review pane's state looks like from outside the frame that draws it.
#[derive(Default)]
struct SeenInDiff {
    hunk_id: Option<String>,
    patch: String,
    /// Whether the file that defines the clicked name is open, its text has arrived, and the
    /// find bar has laid the name out on it - the whole landing rather than the tab appearing,
    /// so a slow machine settles on the same picture a fast one does.
    landed: bool,
    drafts: usize,
    file_panes: usize,
}

/// ⌘-clicking a name on a diff row lands on where it is defined, the same as it does in a
/// file tab: the repo is searched, the line that defines the name wins over the line that
/// calls it, and that file opens at it - without the click also selecting the line.
#[test]
fn a_command_clicked_name_in_a_diff_opens_the_file_that_defines_it() {
    let fixture = calling_fixture("diff-definition");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let seen = Arc::new(Mutex::new(SeenInDiff::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let landed = Arc::new(AtomicBool::new(false));
    let landed_in_ui = Arc::clone(&landed);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let defined = app.model.layout.panes().find_map(|(pane_id, pane)| {
                matches!(pane, crate::native::panes::Pane::File { file_path, .. }
                    if file_path == "src/lib.rs")
                .then_some(pane_id)
            });
            let landing = defined.is_some_and(|pane_id| {
                app.model
                    .file_editors
                    .get(&pane_id)
                    .is_some_and(|editor| editor.content_for_test().is_some())
            }) && app
                .model
                .find
                .as_ref()
                .is_some_and(|find| find.total > 0 && !find.pending);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.drafts = review.drafts.len();
                seen.landed = landing;
            }
            landed_in_ui.store(landing, Ordering::Relaxed);
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    let (hunk_id, patch) = {
        let state = seen.lock().expect("poisoned");
        (
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let at = added_row_at(&harness, &hunk_id, &patch);

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    // The name reads as clickable before it is clicked: with ⌘ down and the pointer on it, the
    // row underlines the name and asks for the pointing hand. A jump nobody can see is there
    // is a jump nobody finds.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.run_steps(2);
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand,
        "⌘ over a name on a diff row should read as a link"
    );

    click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    press_modifiers(&mut harness, egui::Modifiers::NONE);

    assert!(
        settle(&mut harness, || landed.load(Ordering::Relaxed)),
        "the file that defines the name never opened"
    );
    assert_eq!(
        seen.lock().expect("poisoned").drafts,
        0,
        "a ⌘-click is a jump, not the start of a comment"
    );

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    // Long enough for the scroll bar the landing brought up to fade back out - a handful of
    // frames catches it mid-fade, which is a different picture on every machine.
    harness.run_steps(30);
    harness.snapshot("diff-definition-jump");
}

/// The plain click is untouched by the jump: it still selects the whole line and opens the
/// composer on it, which is the gesture the whole pane is built around.
#[test]
fn a_plain_click_on_a_diff_line_still_opens_the_comment_composer() {
    let fixture = calling_fixture("diff-plain-click");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let seen = Arc::new(Mutex::new(SeenInDiff::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let file_panes = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| matches!(pane, crate::native::panes::Pane::File { .. }))
                .count();
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.drafts = review.drafts.len();
                seen.file_panes = file_panes;
            }
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    let (hunk_id, patch) = {
        let state = seen.lock().expect("poisoned");
        (
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let at = added_row_at(&harness, &hunk_id, &patch);

    // The same spot the ⌘-click is made on, with no modifier held: the name under the pointer
    // has nothing to do with it.
    click_at(&mut harness, at);
    harness.run_steps(2);

    let state = seen.lock().expect("poisoned");
    assert_eq!(
        state.drafts, 1,
        "a plain click should have opened the composer on the line"
    );
    assert_eq!(
        state.file_panes, 0,
        "a plain click should not have jumped anywhere"
    );
}
