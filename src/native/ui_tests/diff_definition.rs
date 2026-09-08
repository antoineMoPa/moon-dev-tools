//! ⌘-clicking a name on a row of a review's diff to land on where it is defined.
//!
//! Reading code is what a review is for, so this is the gesture that matters most in this
//! window - and it is also the one with the most ways to have nothing to say, now that a
//! language server is the only thing that answers it. Each of those ways says something
//! different, and each of them is here.
//!
//! The row-finding and the fixtures are [`super::diff_selection`]'s: the plain click and the
//! ⌘-click are made on the same row, and a test that found that row its own way could pass
//! while the two gestures disagreed about which line they were on.

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
    Fixture, app_for, click_like_a_hand,
    diff_selection::{SeenInDiff, added_row_at, calling_fixture, row_at},
    press_modifiers, settle,
};

/// The same repo with the call taken out again, so the diff has a removed row on it.
fn removing_fixture(name: &str) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    );
    fixture.write("src/main.rs", "fn main() {\ngreet(\"moon\");\n}\n");
    fixture.commit("Add the library and the program");
    fixture.write("src/main.rs", "fn main() {\n}\n");
    fixture
}

/// ⌘-clicking a name on a diff row of a file no language server serves says so, and jumps
/// nowhere.
///
/// A language server is the only thing that answers a ⌘-click - see
/// [`crate::native::definition`] - and the ui suite runs with the window's servers switched off,
/// so that a `.rs` fixture in a test does not start the rust-analyzer this machine really has.
/// What that leaves is exactly the state most files are in anyway, and what a person must be
/// told when they are in it: which file has nothing behind it, rather than a click that reads as
/// broken.
///
/// The affordance is still checked, because it is what makes the gesture findable: with ⌘ down
/// and the pointer on a name of a row the file does hold, the name underlines and the cursor is
/// a pointing hand.
///
/// There is no picture of it, where the jump this replaced had one: what the click produces is a
/// message, and the window stamps every message it logs with the time of day, so a snapshot of
/// one is a different image every second.
#[test]
fn a_command_clicked_name_in_a_diff_says_when_no_language_server_serves_the_file() {
    let fixture = calling_fixture("diff-definition");
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
            let said: Vec<String> = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
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
                seen.said = said;
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

    let told = || {
        seen.lock()
            .expect("poisoned")
            .said
            .iter()
            .any(|text| text.contains("no language server serves src/main.rs"))
    };
    assert!(
        settle(&mut harness, told),
        "the click should have said which file nothing serves, saw {:?}",
        seen.lock().expect("poisoned").said.clone()
    );
    {
        let state = seen.lock().expect("poisoned");
        assert_eq!(
            state.drafts, 0,
            "a ⌘-click is a jump, not the start of a comment"
        );
        assert_eq!(
            state.file_panes, 0,
            "a click nothing could answer should not have opened a tab"
        );
    }
}

/// ⌘-clicking a name on a **removed** row says the file does not hold that line any more.
///
/// A removed line is text the file no longer contains, so there is no place in it for a server
/// to be asked about - the row's line number belongs to the old side of the diff and means
/// nothing in the file as it stands. So the name on such a row is not a link: it is not
/// underlined and the cursor stays as it was. The click on it is still answered rather than
/// passed on, because a ⌘-click that quietly opened a comment composer is not what the modifier
/// was held down for.
#[test]
fn a_command_clicked_name_on_a_removed_diff_row_says_the_file_has_no_such_line() {
    let fixture = removing_fixture("diff-definition-removed");
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
            let said: Vec<String> = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
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
                seen.said = said;
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
    let at = row_at(
        &harness,
        &hunk_id,
        &patch,
        crate::native::review::diff::LineKind::Removed,
    );

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.run_steps(2);
    assert_ne!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand,
        "a name on a removed row is not somewhere to jump to, so it must not read as a link"
    );

    click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    press_modifiers(&mut harness, egui::Modifiers::NONE);
    harness.run_steps(4);

    let state = seen.lock().expect("poisoned");
    assert!(
        state
            .said
            .iter()
            .any(|text| text.contains("greet is on a removed line")),
        "the click should have said why there is nothing to look up, saw {:?}",
        state.said
    );
    assert_eq!(
        state.drafts, 0,
        "a ⌘-click on a removed row should not open a comment composer"
    );
    assert_eq!(state.file_panes, 0, "nothing should have been jumped to");
}

/// A real crate, a real rust-analyzer, and a ⌘-click on a row of the diff: the one test that
/// proves a review can ask a language server at all.
///
/// It is the whole point of the review's half of [`crate::native::definition`], and it cannot
/// be proved without a server. A review holds no buffer and has never told anything about the
/// file whose rows it is showing, so the click has to open the document itself, with what the
/// working tree holds, ask about a line of the file as it stands, and hand the document back -
/// and a fake source proves none of that, because what would refuse a question about a document
/// nobody opened is the server.
///
/// Marked `#[ignore]` the way `moon_lsp`'s real-server tests are, and for the same reason: it
/// starts a language server and waits for it to read a project, which is tens of seconds on a
/// cold one and belongs nowhere near a suite that has to stay fast. Everything it needs is in
/// the fixture - a `Cargo.toml`, one file, no dependencies - so the only thing it asks of the
/// machine is that rust-analyzer is installed on it.
///
/// The first click is expected to say the server is still indexing, and that is not a
/// concession: a review is usually the first thing a window shows, nothing has started a server
/// yet, and this is the state a person's first ⌘-click of the session really meets. What must
/// not happen is silence, or "no definition" - see the assertion on what it said.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn a_command_clicked_name_in_a_real_crates_diff_lands_where_rust_analyzer_says() {
    /// How long the server is given to read the fixture. rust-analyzer on a crate this small
    /// is seconds rather than tens of them, but a cold toolchain on a busy machine is not.
    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    /// How long the click is given to be answered and landed on once the server is ready.
    const LANDING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("diff-definition-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n\npub fn call_it() -> String {\n    String::new()\n}\n",
    );
    fixture.commit("Add the crate");
    // The one changed line calls the name, and it is the first thing on its row - so the click
    // lands in the word without any arithmetic on how the text was laid out.
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n\npub fn call_it() -> String {\ngreet(\"moon\")\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let seen = Arc::new(Mutex::new(SeenInDiff::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // What the server says about the file, read every frame so a failure says how far it got.
    let status = Arc::new(Mutex::new(String::from("not asked yet")));
    let status_in_ui = Arc::clone(&status);
    let indexed = Arc::new(AtomicBool::new(false));
    let indexed_in_ui = Arc::clone(&indexed);
    let landed = Arc::new(AtomicBool::new(false));
    let landed_in_ui = Arc::clone(&landed);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            // The one place in the ui suite that means this for a review: a real `.rs` file in
            // a real crate really does start rust-analyzer.
            app.asks_language_servers = true;
            app.draw(ui);

            let session_id = app.model.root_session_id.clone();
            if let Ok(said) = app.tasks.backend().lsp_status(&session_id, "src/lib.rs") {
                *status_in_ui.lock().expect("the status is not shared") = format!("{said:?}");
                indexed_in_ui.store(said == crate::api::LspStatus::Ready, Ordering::Relaxed);
            }
            // The whole landing: the file the name is defined in is open and its text is there.
            let defined = app.model.layout.panes().find_map(|(pane_id, pane)| {
                matches!(pane, crate::native::panes::Pane::File { file_path, .. }
                    if file_path == "src/lib.rs")
                .then_some(pane_id)
            });
            landed_in_ui.store(
                defined.is_some_and(|pane_id| {
                    app.model
                        .file_editors
                        .get(&pane_id)
                        .is_some_and(|editor| editor.content_for_test().is_some())
                }),
                Ordering::Relaxed,
            );

            let said: Vec<String> = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
            let Some(review) = app.model.review_ref(&session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.said = said;
            }
            loaded_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
        });

    /// Step the window until something is true, or give up after a while. `settle`'s own
    /// twenty seconds is nowhere near long enough for a server reading a project.
    fn wait(harness: &mut Harness<'_>, patience: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    assert!(loaded.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(2);

    let click_the_name = |harness: &mut Harness<'_>, seen: &Arc<Mutex<SeenInDiff>>| {
        let (hunk_id, patch) = {
            let state = seen.lock().expect("poisoned");
            (
                state.hunk_id.clone().expect("expected a hunk"),
                state.patch.clone(),
            )
        };
        let at = added_row_at(harness, &hunk_id, &patch);
        press_modifiers(harness, egui::Modifiers::COMMAND);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.run_steps(2);
        click_like_a_hand(harness, at, egui::Modifiers::COMMAND);
        press_modifiers(harness, egui::Modifiers::NONE);
    };

    // Nothing has started a server yet, which is the state a window's first ⌘-click is really
    // made in. The click starts one by opening the document on it, and says so rather than
    // saying the name is defined nowhere.
    click_the_name(&mut harness, &seen);
    let told = wait(&mut harness, LANDING_TAKES_AT_MOST, || {
        seen.lock()
            .expect("poisoned")
            .said
            .iter()
            .any(|text| text.contains("still indexing") || text.contains("no definition"))
            || landed.load(Ordering::Relaxed)
    });
    assert!(
        told,
        "the first click said nothing at all, with the server {}",
        status.lock().expect("the status is not shared")
    );
    assert!(
        !seen
            .lock()
            .expect("poisoned")
            .said
            .iter()
            .any(|text| text.contains("no definition")),
        "a server that has not read the project must never read as the name being defined nowhere"
    );

    let started = Instant::now();
    let finished = wait(&mut harness, INDEXING_TAKES_AT_MOST, || {
        indexed.load(Ordering::Relaxed)
    });
    println!(
        "rust-analyzer was {} after {:.1}s",
        status.lock().expect("the status is not shared"),
        started.elapsed().as_secs_f32()
    );
    assert!(
        finished,
        "rust-analyzer never finished reading the crate - it got as far as {}",
        status.lock().expect("the status is not shared")
    );

    // And now the click the whole of this is for: the document is opened on the server with
    // what the working tree holds, the row's line is asked about, and the file the name is
    // defined in opens.
    click_the_name(&mut harness, &seen);
    assert!(
        wait(&mut harness, LANDING_TAKES_AT_MOST, || landed
            .load(Ordering::Relaxed)),
        "the click never landed anywhere; the window said {:?}",
        seen.lock().expect("poisoned").said
    );
}
