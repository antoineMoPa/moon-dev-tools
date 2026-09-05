//! Commenting on a diff, and the staging and jumping the review offers beside it.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::{api::OpenSessionRequest, backend::local::LocalBackend, native::theme::ThemeMode};

use super::{Fixture, app_for, click_at, seeded_fixture, settle};

/// Clicking a diff line selects it and opens the comment composer in one gesture, the way
/// selecting text does.
#[test]
fn clicking_a_diff_line_opens_the_comment_composer() {
    let fixture = seeded_fixture("comment");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    /// What the test needs to see from inside the UI closure.
    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selected_lines: usize,
        draft_selection: Option<String>,
        draft_is_focused: bool,
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
                seen.selected_lines = review
                    .selection
                    .map(|selection| selection.line_range().count())
                    .unwrap_or(0);
                seen.draft_selection = review.drafts.first().map(|draft| draft.selection.clone());
                seen.draft_is_focused = review
                    .drafts
                    .first()
                    .is_some_and(|draft| !draft.selection.is_empty());
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
            state
                .hunk_id
                .clone()
                .expect("expected a hunk to comment on"),
            state.patch.clone(),
        )
    };
    // Which patch line to click, and what it says, come from the same parse the review pane
    // uses - git's own header lines differ between versions and configurations.
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, expected) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.text.clone()))
        .expect("expected an added line to comment on");
    assert_eq!(
        seen.lock().expect("expected state").selected_lines,
        0,
        "nothing is selected before a click"
    );

    // Found by the same id the review pane drew the line with.
    let target = crate::native::review::hunks::diff_line_id(&hunk_id, line_index);
    let rect = harness
        .ctx
        .read_response(target)
        .expect("expected the diff line to have been drawn")
        .rect;
    click_at(&mut harness, rect.center());

    {
        let state = seen.lock().expect("expected state");
        assert_eq!(
            state.selected_lines, 1,
            "clicking a diff line selects exactly that line"
        );
        let selection = state
            .draft_selection
            .clone()
            .expect("clicking a line must open the composer");
        assert_eq!(
            selection, expected,
            "the comment must be anchored to the exact line that was clicked"
        );
        assert!(
            state.draft_is_focused,
            "the composer should be ready to type in"
        );
    }

    // And the composer is on screen, not merely in the model.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("comment-composer");
}

/// A comment being typed survives everything short of deliberately cancelling it: sweeping
/// a new run of lines parks the typed composer where it is and opens a fresh one, and an
/// Escape - which may have been aimed at a palette or a terminal in the next split - never
/// throws typed text away.
#[test]
fn reselecting_lines_keeps_the_note_being_typed() {
    let fixture = seeded_fixture("keep-note");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        notes: Vec<String>,
        selected_lines: usize,
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
                seen.notes = review
                    .drafts
                    .iter()
                    .map(|draft| draft.note.clone())
                    .collect();
                seen.selected_lines = review
                    .selection
                    .map(|selection| selection.line_range().count())
                    .unwrap_or(0);
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
    assert!(changed.len() >= 3, "the fixture needs lines to sweep over");

    let rect_of = |harness: &Harness<'_>, index: usize| {
        harness
            .ctx
            .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, index))
            .expect("expected the diff line to have been drawn")
            .rect
    };

    // Open the composer on the first changed line and type into it.
    let first = rect_of(&harness, changed[0]).center();
    click_at(&mut harness, first);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("needs work".to_string()));
    harness.step();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").notes,
        ["needs work"],
        "typing should land in the composer"
    );

    // Sweep a different run of lines: the typed composer stays parked with its text, and a
    // fresh one opens on the new run.
    let start = rect_of(&harness, changed[1]).center();
    let end = rect_of(&harness, changed[2]).center();
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

    {
        let state = seen.lock().expect("poisoned");
        assert_eq!(
            state.notes,
            ["needs work", ""],
            "the typed composer stays parked, and a fresh one opens on the new run"
        );
        assert_eq!(state.selected_lines, 2, "the new run is what is selected");
    }

    // Escape closes the fresh, empty composer - the one holding the keyboard - and leaves
    // the typed one alone, wherever the Escape was actually aimed.
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").notes,
        ["needs work"],
        "escape must never discard typed text"
    );
}

/// The comment dispatch contract, which the header and the composer both depend on.
///
/// `batch: false` hands the comment to the agent there and then; `batch: true` holds it back
/// so a batch can go at once. The batch send only moves what is actually held - which is why
/// the header must count held comments and nothing else.
#[test]
fn a_held_comment_is_what_the_batch_send_moves() {
    use crate::api::{CommentDispatchStatus, CommentRequest};

    let fixture = seeded_fixture("dispatch");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = LocalBackend::new(state);
    let opened = crate::backend::Backend::open_session(
        &backend,
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");
    let session_id = opened.session_id;

    let hunk = crate::backend::Backend::session_state(&backend, &session_id)
        .expect("expected the review state")
        .hunks
        .first()
        .cloned()
        .expect("expected a hunk to comment on");
    let anchor = crate::native::review::diff::build_diff_lines(&hunk.patch_preview)
        .into_iter()
        .find(|line| line.kind.commentable())
        .expect("expected a line to anchor to")
        .text;

    let comment =
        crate::comments::build_anchored_comment_value(&[crate::comments::AnchoredComment {
            selection: anchor,
            comment: "this needs a second look".to_string(),
            resolved: false,
        }]);

    // Held back, with no agent picked: nothing may be dispatched yet.
    crate::backend::Backend::set_comment(
        &backend,
        &session_id,
        CommentRequest {
            hunk_id: hunk.id.clone(),
            comment: comment.clone(),
            batch: true,
        },
    )
    .expect("expected the comment to be stored");

    let held = crate::backend::Backend::session_state(&backend, &session_id)
        .expect("expected the review state");
    assert_eq!(
        held.review_comments.len(),
        1,
        "the comment should be stored"
    );
    assert_eq!(
        held.review_comments[0].dispatch.status,
        CommentDispatchStatus::Batched,
        "a held comment waits for the batch send"
    );

    // The batch send needs an agent, and says so rather than doing nothing.
    let refused = crate::backend::Backend::send_comment_batch(&backend, &session_id)
        .expect_err("expected the send to be refused without an agent");
    assert!(
        refused.to_string().contains("select an agent"),
        "the refusal should say what is missing: {refused}"
    );
    assert_eq!(
        crate::backend::Backend::session_state(&backend, &session_id)
            .expect("expected the review state")
            .review_comments[0]
            .dispatch
            .status,
        CommentDispatchStatus::Batched,
        "a refused send leaves the comment held"
    );
}

/// A diff of many hunks only lays out the cards on screen, but a jump to a hunk still has to
/// reach one that is nowhere near the viewport.
#[test]
fn jumping_to_a_hunk_reaches_one_that_was_being_skipped() {
    let fixture = Fixture::new("scroll-to-hunk");
    for file in 0..80 {
        fixture.write(
            &format!("src/module_{file}/values.rs"),
            "pub const A: u32 = 1;\npub const B: u32 = 2;\n",
        );
    }
    fixture.commit("Add the modules");
    for file in 0..80 {
        fixture.write(
            &format!("src/module_{file}/values.rs"),
            "pub const A: u32 = 9;\npub const B: u32 = 2;\n",
        );
    }

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let last_hunk_id = Arc::new(Mutex::new(String::new()));
    let last_in_ui = Arc::clone(&last_hunk_id);
    let active = Arc::new(Mutex::new(None::<String>));
    let active_in_ui = Arc::clone(&active);
    let jump_to = Arc::new(Mutex::new(None::<String>));
    let jump_in_ui = Arc::clone(&jump_to);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .wgpu()
        .build_ui(move |ui| {
            if let Some(hunk_id) = jump_in_ui.lock().expect("poisoned").take() {
                let session_id = app.model.root_session_id.clone();
                app.model.review(&session_id).scroll_to_hunk = Some(hunk_id);
            }
            app.draw(ui);
            if let Some(review) = app.model.review_ref(&app.model.root_session_id) {
                if let Some(hunk) = review.hunks().last() {
                    *last_in_ui.lock().expect("poisoned") = hunk.id.clone();
                }
                ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
                *active_in_ui.lock().expect("poisoned") = review.active_hunk_id.clone();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let last = last_hunk_id.lock().expect("poisoned").clone();
    assert!(!last.is_empty(), "the review should have hunks");
    assert_ne!(
        active.lock().expect("poisoned").as_deref(),
        Some(last.as_str()),
        "the last hunk starts far below the viewport"
    );

    *jump_to.lock().expect("poisoned") = Some(last.clone());
    harness.run_steps(4);

    // Only a card that was actually drawn reports the jump, so this is how a skipped one
    // would show up: the review would never come to rest on it.
    assert_eq!(
        active.lock().expect("poisoned").as_deref(),
        Some(last.as_str()),
        "jumping should have drawn the hunk and made it the active one"
    );
}

/// The sidebar's staging dot is also the control for it, the way its status
/// badge is: one click stages the whole file, the next one takes it back out of the index.
#[test]
fn clicking_a_file_staging_dot_stages_the_whole_file() {
    let fixture = seeded_fixture("stage-dot");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    /// What the file's hunks say about the index, read from inside the UI closure.
    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    struct Staging {
        hunks: usize,
        staged: usize,
    }

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

    let before = *staging.lock().expect("poisoned");
    assert!(before.hunks > 0, "the fixture edits src/lib.rs");
    assert_eq!(before.staged, 0, "the fixture's edits start unstaged");

    let dot = harness
        .ctx
        .read_response(crate::native::review::sidebar::stage_dot_id("src/lib.rs"))
        .expect("expected the file row's staging dot to have been drawn")
        .rect;
    click_at(&mut harness, dot.center());

    // Staging runs on a worker thread and the review is refetched after it, so the model
    // catches up over the next few frames rather than on the click itself.
    let all_staged = settle(&mut harness, || {
        let seen = *staging.lock().expect("poisoned");
        seen.hunks > 0 && seen.staged == seen.hunks
    });
    assert!(
        all_staged,
        "clicking the dot should have staged every hunk of the file, saw {:?}",
        *staging.lock().expect("poisoned")
    );

    // The dot now reads staged, so the same click has to be the way back out.
    click_at(&mut harness, dot.center());
    let all_unstaged = settle(&mut harness, || {
        let seen = *staging.lock().expect("poisoned");
        seen.hunks > 0 && seen.staged == 0
    });
    assert!(
        all_unstaged,
        "clicking the dot again should have unstaged the file, saw {:?}",
        *staging.lock().expect("poisoned")
    );
}
