//! The strip over a review: the repo it is on and what it is pointed at.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use egui_frames::FrameId;

use crate::native::{
    panes::{Pane, PaneKind},
    theme::ThemeMode,
};

use super::{app_for, click_at, seeded_fixture, settle};

/// Clicking the repo's name over a review opens a shell standing at the repo's root - even
/// for a review started from a folder inside it.
#[test]
fn clicking_the_repo_name_opens_a_shell_at_the_repo_root() {
    let fixture = seeded_fixture("header-shell");
    let repo_name = fixture
        .root
        .file_name()
        .expect("the fixture repo has a name")
        .to_string_lossy()
        .to_string();

    let mut app = app_for(&fixture.root.join("src"), ThemeMode::Dark);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);
    // Asked its folder once the shell is there, and only once: the answer is what the shell
    // prints, and asking every frame would be typing over it.
    let asked = Arc::new(AtomicBool::new(false));
    let shell_says = Arc::new(Mutex::new(String::new()));
    let shell_says_in_ui = Arc::clone(&shell_says);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            app.draw(ui);
            loaded_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            for terminal in app.terminals.values_mut() {
                if !asked.swap(true, Ordering::Relaxed) {
                    terminal
                        .send(b"basename \"$PWD\"\n")
                        .expect("expected to write to the shell");
                }
                *shell_says_in_ui.lock().expect("poisoned") =
                    terminal.visible_text().unwrap_or_default();
            }
            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Terminal)
                    .is_some(),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    assert!(
        !pane_open.load(Ordering::Relaxed),
        "no shell before the click"
    );

    let name = harness.get_by_label(&repo_name).rect().center();
    click_at(&mut harness, name);
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the repo's name should have opened a shell"
    );
    assert!(
        settle(&mut harness, || {
            // A line of its own: the echoed command is on the prompt's line.
            shell_says
                .lock()
                .expect("poisoned")
                .lines()
                .any(|line| line.trim() == repo_name)
        }),
        "the shell should be standing at the repo's root; it said:\n{}",
        shell_says.lock().expect("poisoned")
    );
}

/// With a frame already down the right of the review, the shell joins its tabs rather than
/// splitting the window again - the way a task's tabs join the column beside the board.
#[test]
fn the_repo_names_shell_joins_the_frame_at_the_right() {
    let fixture = seeded_fixture("header-shell-right");
    let repo_name = fixture
        .root
        .file_name()
        .expect("the fixture repo has a name")
        .to_string_lossy()
        .to_string();

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // The frame put down the right of the review, and the frame the shell landed in.
    let frames = Arc::new(Mutex::new((None::<FrameId>, None::<FrameId>)));
    let frames_in_ui = Arc::clone(&frames);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            let review_loaded = app
                .model
                .review_ref(&app.model.root_session_id)
                .is_some_and(|review| review.payload.is_some());
            let mut frames = frames_in_ui.lock().expect("poisoned");
            if review_loaded && frames.0.is_none() {
                let review_frame = app.model.layout.active_frame();
                let messages = app.model.layout.add_pane_beside(
                    review_frame,
                    egui_frames::DropSide::Right,
                    Pane::Messages,
                );
                frames.0 = app.model.layout.frame_of(messages);
                app.model.layout.set_active_frame(review_frame);
            }
            app.draw(ui);
            frames.1 = app
                .model
                .layout
                .find_pane(|pane| pane.kind() == PaneKind::Terminal)
                .and_then(|(pane, _)| app.model.layout.frame_of(pane));
            loaded_in_ui.store(frames.0.is_some(), Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);
    let right = frames
        .lock()
        .expect("poisoned")
        .0
        .expect("expected the frame at the right");

    let name = harness.get_by_label(&repo_name).rect().center();
    click_at(&mut harness, name);
    assert!(
        settle(&mut harness, || frames
            .lock()
            .expect("poisoned")
            .1
            .is_some()),
        "clicking the repo's name should have opened a shell"
    );
    assert_eq!(
        frames.lock().expect("poisoned").1,
        Some(right),
        "the shell should be a tab of the frame already at the right"
    );
}
