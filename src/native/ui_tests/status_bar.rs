//! The strip along the bottom of the window: what a language server is doing while it is
//! doing it, the last thing the window said once nothing is, and the log a click on it opens.
//!
//! No language server is started here, and none may be: the servers this machine really has
//! would index the fixture repo for as long as they felt like it. What the strip reads is
//! [`crate::native::model::Model::language_servers_working`], which is what the poll writes,
//! so the tests put an answer there themselves - the wire that fills it is
//! [`crate::lsp`]'s own business and has its own tests.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    api::LspWork,
    native::{model::ToastKind, panes::PaneKind, status_bar::ServersWorking, theme::ThemeMode},
};

use super::{app_for, click_at, seeded_fixture};

/// How big the window is in these tests. The bar is along the bottom of it, so the height is
/// what says where to click.
const WINDOW: egui::Vec2 = egui::vec2(1200.0, 760.0);

#[test]
fn the_status_bar_says_what_a_language_server_is_doing_and_how_far_through_it_is() {
    // Arrange
    let fixture = seeded_fixture("status-bar-indexing");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            // Written every frame: an answer only stands for a moment, and a snapshot taken
            // on the frame after it went stale would be a picture of an empty strip. Only
            // once the session has a name, so the frames before the review opened do not
            // leave an answer of their own behind under an empty one.
            let session_id = app.model.root_session_id.clone();
            if !session_id.is_empty() {
                app.model.language_servers_working.insert(
                    session_id,
                    ServersWorking::heard_now(vec![LspWork {
                        server: "rust-analyzer".to_string(),
                        title: "Indexing".to_string(),
                        detail: Some("12/57 (serde)".to_string()),
                        percentage: Some(42),
                    }]),
                );
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    // Act: settle on the review being loaded, which is the end state - the strip is drawn on
    // every frame from the first one, so there is no stage of it to catch part way through.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);

    // Assert
    harness.snapshot("status-bar-indexing");
    harness.get_by_label_contains("rust-analyzer indexing - 12/57 (serde)");
    harness.get_by_label("42%");
}

#[test]
fn the_status_bar_reads_out_the_last_message_and_opens_the_log_when_it_is_clicked() {
    // Arrange: a window that has said one thing, which is the state a faded toast leaves
    // behind - the toast is gone from the corner and the strip still says what it said.
    let fixture = seeded_fixture("status-bar-messages");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let said = Arc::new(AtomicBool::new(false));
    let said_in_ui = Arc::clone(&said);
    let log_open = Arc::new(AtomicBool::new(false));
    let log_open_in_ui = Arc::clone(&log_open);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| {
            if matches!(app.model.stage, crate::native::model::Stage::Ready)
                && !said_in_ui.swap(true, Ordering::Relaxed)
            {
                app.model.info("staged the whole of src/lib.rs");
            }
            app.draw(ui);
            ready_in_ui.store(!app.model.messages.is_empty(), Ordering::Relaxed);
            log_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Messages)
                    .is_some(),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the window never said anything"
    );
    harness.run_steps(3);
    // Twice over, in fact: the toast is still up in the corner and the strip says the same
    // thing along the bottom, which is the point - the strip is what is left once it fades.
    assert!(
        harness
            .query_all_by_label_contains("staged the whole of src/lib.rs")
            .count()
            >= 1,
        "the strip should read out the last thing the window said"
    );

    // Act: a click anywhere along the strip.
    click_at(&mut harness, egui::pos2(WINDOW.x / 2.0, WINDOW.y - 12.0));
    harness.run_steps(3);

    // Assert: the log is open, with the message in it.
    assert!(
        log_open.load(Ordering::Relaxed),
        "clicking the strip should open the message log"
    );
    harness.get_by_label_contains("Messages (");
}

/// The rule the log exists for: the corner folds a repeated message into the one already up,
/// and the log records every time it was posted - "this happened four times" being exactly
/// what a log is read to find out.
#[test]
fn a_message_posted_twice_is_one_toast_and_two_lines_in_the_log() {
    let fixture = seeded_fixture("status-bar-repeats");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    app.model.error("rust is still indexing this project");
    app.model.error("rust is still indexing this project");

    assert_eq!(
        app.model.toasts.len(),
        1,
        "a repeated message is refreshed in the corner rather than stacked"
    );
    assert_eq!(
        app.model.messages.len(),
        2,
        "both postings are written down"
    );
    assert!(
        app.model
            .messages
            .iter()
            .all(|message| matches!(message.kind, ToastKind::Error)),
        "both should be recorded as failures"
    );
}
