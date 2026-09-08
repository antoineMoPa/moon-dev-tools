//! What a click on a file in the review does: the staging and the discard a right-click on a
//! sidebar row offers, and the file itself, which every mention of one opens - from the menu,
//! or with ⌘ held down.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use super::{app_for, click_like_a_hand, press_modifiers, right_click_at, seeded_fixture, settle};
use crate::native::theme::ThemeMode;

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

    harness
        .get_by_label("[really discard the whole file]")
        .click();
    let discarded = settle(&mut harness, || {
        std::fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("failed to read")
            .contains("pub fn count")
            .eq(&false)
    });
    assert!(discarded, "confirming should have reverted the file");
}

/// The file itself is what the menu offers first, and it opens in a tab of its own.
#[test]
fn the_file_menu_opens_the_file() {
    let fixture = seeded_fixture("file-menu-open");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            *opened_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                })
                .collect();
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
    right_click_at(
        &mut harness,
        egui::pos2(dot.center().x + 70.0, dot.center().y),
    );

    assert!(
        harness.query_by_label("open the file").is_some(),
        "right-clicking a file should offer opening it"
    );
    harness.get_by_label("open the file").click();

    let in_a_tab = settle(&mut harness, || {
        opened
            .lock()
            .expect("poisoned")
            .iter()
            .any(|path| path == "src/lib.rs")
    });
    assert!(
        in_a_tab,
        "the menu should have opened the file, saw {:?}",
        *opened.lock().expect("poisoned")
    );
}

/// The heading over a file's hunks names the file too, and offers the same way into it.
#[test]
fn the_diff_heading_opens_the_file() {
    let fixture = seeded_fixture("diff-heading-open");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            *opened_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                })
                .collect();
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    let heading = harness.get_by_label("src/lib.rs").rect();
    right_click_at(&mut harness, heading.center());

    assert!(
        harness.query_by_label("open the file").is_some(),
        "right-clicking the heading should offer opening the file"
    );
    harness.get_by_label("open the file").click();

    let in_a_tab = settle(&mut harness, || {
        opened
            .lock()
            .expect("poisoned")
            .iter()
            .any(|path| path == "src/lib.rs")
    });
    assert!(
        in_a_tab,
        "the heading's menu should have opened the file, saw {:?}",
        *opened.lock().expect("poisoned")
    );
}

/// ⌘-clicking a file's name in the sidebar opens it, and reads as a link while ⌘ is down.
/// The click without ⌘ still belongs to the diff, which it scrolls to that file.
#[test]
fn a_command_click_on_a_file_opens_it() {
    let fixture = seeded_fixture("file-command-click");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            *opened_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                })
                .collect();
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

    // A plain click is the sidebar's own: it takes the diff to the file rather than opening it.
    super::click_at(&mut harness, row);
    assert!(
        opened.lock().expect("poisoned").is_empty(),
        "a click without ⌘ should not have opened a tab"
    );

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(row));
    harness.run_steps(2);
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand,
        "⌘ over a file's name should read as a link"
    );

    click_like_a_hand(&mut harness, row, egui::Modifiers::COMMAND);
    press_modifiers(&mut harness, egui::Modifiers::NONE);

    let in_a_tab = settle(&mut harness, || {
        opened
            .lock()
            .expect("poisoned")
            .iter()
            .any(|path| path == "src/lib.rs")
    });
    assert!(
        in_a_tab,
        "⌘-clicking the file should have opened it, saw {:?}",
        *opened.lock().expect("poisoned")
    );
}
