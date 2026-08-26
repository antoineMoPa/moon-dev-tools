//! Dragging cards and column headings around the moontasks board.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, seeded_fixture, drag_from_to};

/// Dragging a card is how a column is put in order, so where it is let go of has to be where
/// it lands - not merely which column it landed in.
#[test]
fn a_card_dropped_above_another_takes_its_place() {
    let fixture = seeded_fixture("board-order");
    // Cards that have never been moved read in the order they were created, so the fixture
    // says when each one was.
    for (task_id, title, created) in [
        ("write-the-parser-1111", "Write the parser", 1700000000),
        ("fix-the-login-page-2222", "Fix the login page", 1700000001),
        ("drop-the-old-api-3333", "Drop the old API", 1700000002),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"todo\",\n  \
                 \"created_at_unix\": {created},\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // What the board is showing, read out on every frame: the drop is answered on a worker
    // thread, so the order has to be watched for rather than counted in frames.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let order_in_ui = Arc::clone(&order);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            if let Ok(mut order) = order_in_ui.lock() {
                *order = app
                    .model
                    .board
                    .tasks
                    .iter()
                    .map(|task| task.title.clone())
                    .collect();
            }
        });

    let read = || order.lock().expect("expected the board").clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && read().len() != 3 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        read(),
        ["Write the parser", "Fix the login page", "Drop the old API"],
        "the board never read the three tasks in the order they were written"
    );
    harness.run_steps(3);

    // The title, which is the handle a card is dragged by and so the thing the pointer has
    // to press on.
    let handle_of = |harness: &Harness<'_>, task_id: &str| {
        harness
            .ctx
            .read_response(egui::Id::new(("moontask-card", &task_id.to_string())))
            .expect("expected the card to have been drawn")
            .rect
    };
    let second = handle_of(&harness, "fix-the-login-page-2222");
    let last = handle_of(&harness, "drop-the-old-api-3333");
    // Picked up by its title and let go of just above the second card's title: past the whole
    // of the first card - notes box and all - which is the gap between the two.
    let start = last.center();
    let end = second.center_top() - egui::vec2(0.0, 4.0);

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
    for at in [start + egui::vec2(0.0, -20.0), end] {
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.step();
    }
    // A few frames with the pointer where it is: the slot a card is being held over is worked
    // out at the end of a frame and taken up by the next one, and the cards making room for
    // it walk there rather than jumping.
    harness.run_steps(12);

    // Mid-drag: the card is under the cursor, and the space being held for it is where it
    // would land - between the two cards it is being dropped between.
    harness.snapshot("moontasks-drag");

    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    // The pointer leaves the cards before the picture is taken: the landed card's title is
    // right where the drop was, and hovering it long enough draws the tooltip - which names
    // the fixture's own folder, process id and all, so no two runs would match.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(1200.0, 600.0)));
    harness.step();

    // Just dropped: the card is in the slot it was held over, marked so it can be picked back
    // out of the column it landed in.
    harness.snapshot("moontasks-dropped");

    let expected = ["Write the parser", "Drop the old API", "Fix the login page"];
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && read() != expected {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        read(),
        expected,
        "the card should have landed in the gap it was dropped in"
    );
}

/// A card is moved between columns by dragging it, which is the only way to move one - the
/// arrows that used to do it are gone.
#[test]
fn dragging_a_card_moves_it_to_the_column_it_is_dropped_on() {
    let fixture = seeded_fixture("board-drag");
    let task_id = "write-the-parser-1111";
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // Where the card is drawn, and which column it is in, both read back out of the window.
    let seen = Arc::new(Mutex::new((egui::Rect::NOTHING, String::new())));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            if let Some(task) = app.model.board.tasks.first() {
                ready_in_ui.store(true, Ordering::Relaxed);
                let title = ui
                    .ctx()
                    .read_response(egui::Id::new(("moontask-card", &task.id)));
                *seen_in_ui.lock().expect("poisoned") = (
                    title
                        .map(|response| response.rect)
                        .unwrap_or(egui::Rect::NOTHING),
                    task.status.to_string(),
                );
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let (handle, status) = seen.lock().expect("poisoned").clone();
    assert_eq!(status, "todo", "the card starts in TODO");
    assert!(
        handle.is_positive(),
        "the card's drag handle was never drawn"
    );

    // One column to the right, which is IN PROGRESS.
    let onto = handle.center() + egui::vec2(COLUMN_STRIDE, 40.0);
    drag_from_to(&mut harness, handle.center(), onto);

    // The move is written to `.moontasks` and read back, so the board has to poll to see it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if seen.lock().expect("poisoned").1 == "in_progress" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        seen.lock().expect("poisoned").1,
        "in_progress",
        "dropping the card on IN PROGRESS should have moved it there"
    );
}

/// How far apart two columns of the board are drawn, which is what a drag has to cover to
/// reach the next one.
const COLUMN_STRIDE: f32 = 298.0;

/// A column is moved by dragging its heading, and its cards go with it - a card names the
/// column it is in rather than a place on the board, so nothing about the card changes.
#[test]
fn dragging_a_heading_moves_the_column_and_its_cards() {
    let fixture = seeded_fixture("column-drag");
    let task_id = "write-the-parser-1111";
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    /// The order of the columns, where TODO's heading is, and which column the card is in.
    #[derive(Clone)]
    struct Seen {
        order: Vec<String>,
        handle: egui::Rect,
        card_status: String,
    }
    let seen = Arc::new(Mutex::new(Seen {
        order: Vec::new(),
        handle: egui::Rect::NOTHING,
        card_status: String::new(),
    }));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            let heading = ui
                .ctx()
                .read_response(egui::Id::new(("moontask-column", "todo")));
            if let Ok(mut seen) = seen_in_ui.lock() {
                seen.order = app
                    .model
                    .board
                    .columns
                    .iter()
                    .map(|column| column.id.to_string())
                    .collect();
                seen.handle = heading
                    .map(|response| response.rect)
                    .unwrap_or(egui::Rect::NOTHING);
                seen.card_status = app
                    .model
                    .board
                    .tasks
                    .first()
                    .map(|task| task.status.to_string())
                    .unwrap_or_default();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if seen.lock().expect("poisoned").handle.is_positive() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let before = seen.lock().expect("poisoned").clone();
    assert_eq!(
        before.order.first().map(String::as_str),
        Some("todo"),
        "TODO starts on the left, got {:?}",
        before.order
    );
    assert_eq!(before.card_status, "todo");
    assert!(
        before.handle.is_positive(),
        "the column's drag handle was never drawn"
    );

    // Carried past the middle of IN PROGRESS, which is what puts TODO on the far side of it.
    // The column travels on the cursor, so what has to clear that middle is the cursor.
    let onto = egui::pos2(
        before.handle.center().x + COLUMN_STRIDE * 1.5,
        before.handle.center().y,
    );
    drag_from_to(&mut harness, before.handle.center(), onto);

    // The move is written to `.moontasks` and read back, so the board has to poll to see it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if seen
            .lock()
            .expect("poisoned")
            .order
            .first()
            .map(String::as_str)
            == Some("in_progress")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let after = seen.lock().expect("poisoned").clone();
    assert_eq!(
        after.order,
        ["in_progress", "todo", "done"],
        "dropping the heading one place right should have moved the column there"
    );
    assert_eq!(
        after.card_status, "todo",
        "the card should have travelled with its column, unchanged"
    );
}
