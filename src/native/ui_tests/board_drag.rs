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

use super::{
    app_for, board_of, click_like_a_hand, drag_from_to, drag_like_a_hand, notes_of,
    press_modifiers, seeded_fixture, settle, title_of,
};

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

/// Cards are marked with cmd+click and dragged as one: picking up any of them carries the
/// rest, and they land in the column they are dropped on as a run.
///
/// cmd stays down from the first click to the drop, because that is how the gesture is made -
/// the key gathers the cards and the same hand drags them.
#[test]
fn cards_marked_with_cmd_are_dragged_together() {
    let (mut harness, seen, _repo) = board_of("board-marks-drag", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    for task_id in ["fix-the-login-page-2222", "drop-the-old-api-3333"] {
        let at = title_of(&harness, task_id);
        click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    }
    assert_eq!(read().marked.len(), 2, "cmd+click should mark both cards");
    assert!(
        !read().page_open,
        "and it should open nothing: marking is not reading"
    );

    // One of the two picked up and carried a column to the right, cmd still down.
    let from = title_of(&harness, "fix-the-login-page-2222");
    drag_like_a_hand(
        &mut harness,
        from,
        from + egui::vec2(COLUMN_STRIDE, 40.0),
        egui::Modifiers::COMMAND,
    );
    press_modifiers(&mut harness, egui::Modifiers::NONE);

    assert!(
        settle(&mut harness, || {
            let seen = read();
            seen.column_of("fix-the-login-page-2222") == "in_progress"
                && seen.column_of("drop-the-old-api-3333") == "in_progress"
        }),
        "both marked cards should have moved, got {:?}",
        read().columns
    );
    assert_eq!(
        read().column_of("write-the-parser-1111"),
        "todo",
        "and the card that was not marked should have stayed where it was"
    );
}

/// A marked card is dragged by any part of it, with nothing held: the keys are how a group is
/// gathered, not how it is carried, and the card's own description is as good a place to take
/// hold of it as its title.
#[test]
fn a_marked_card_is_dragged_by_any_part_of_it() {
    let (mut harness, seen, _repo) = board_of("board-grab-anywhere", &["fix-the-login-page-2222"]);
    let read = || seen.lock().expect("poisoned").clone();

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    for task_id in ["write-the-parser-1111", "fix-the-login-page-2222"] {
        let at = title_of(&harness, task_id);
        click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    }
    press_modifiers(&mut harness, egui::Modifiers::NONE);
    assert_eq!(read().marked.len(), 2);

    let from = notes_of(&harness, "fix-the-login-page-2222");
    drag_like_a_hand(
        &mut harness,
        from,
        from + egui::vec2(COLUMN_STRIDE, 40.0),
        egui::Modifiers::NONE,
    );

    assert!(
        settle(&mut harness, || {
            let seen = read();
            seen.column_of("fix-the-login-page-2222") == "in_progress"
                && seen.column_of("write-the-parser-1111") == "in_progress"
        }),
        "the card should have been dragged by its description and taken the other with it, \
         got {:?}",
        read().columns
    );
    assert!(
        !read().page_open,
        "and the press that carried the card should not also have opened it"
    );
}

/// A card picked up with nothing held and no mark on it goes alone - the way dragging one icon
/// out of a selected group does in a file manager - and becomes the mark itself, which is what
/// shows where it landed.
#[test]
fn an_unmarked_card_is_dragged_alone_and_becomes_the_mark() {
    let (mut harness, seen, _repo) = board_of("board-drag-unmarked", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    let marked = title_of(&harness, "write-the-parser-1111");
    click_like_a_hand(&mut harness, marked, egui::Modifiers::COMMAND);
    press_modifiers(&mut harness, egui::Modifiers::NONE);

    let from = title_of(&harness, "drop-the-old-api-3333");
    drag_like_a_hand(
        &mut harness,
        from,
        from + egui::vec2(COLUMN_STRIDE, 40.0),
        egui::Modifiers::NONE,
    );

    assert!(
        settle(&mut harness, || read().column_of("drop-the-old-api-3333")
            == "in_progress"),
        "the card that was picked up should have moved"
    );
    assert_eq!(
        read().column_of("write-the-parser-1111"),
        "todo",
        "and the marked card should have stayed where it was"
    );
    assert_eq!(
        read().marked,
        vec!["drop-the-old-api-3333".to_string()],
        "and the card that was carried is the one marked, where it landed"
    );
}

/// A drag carries cards only when it began on one. A press on the board beside the cards is
/// the marks being let go of, however far it is carried afterwards.
#[test]
fn a_drag_that_began_on_no_card_carries_nothing() {
    let (mut harness, seen, _repo) = board_of("board-drag-from-nowhere", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    for task_id in ["write-the-parser-1111", "fix-the-login-page-2222"] {
        let at = title_of(&harness, task_id);
        click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    }
    press_modifiers(&mut harness, egui::Modifiers::NONE);
    assert_eq!(read().marked.len(), 2);

    // Below the cards, where the board has nothing drawn, carried across two columns.
    let below = title_of(&harness, "drop-the-old-api-3333") + egui::vec2(0.0, 260.0);
    drag_like_a_hand(
        &mut harness,
        below,
        below + egui::vec2(COLUMN_STRIDE * 2.0, 0.0),
        egui::Modifiers::NONE,
    );
    harness.run_steps(5);

    assert!(
        read().columns.iter().all(|(_, status)| status == "todo"),
        "nothing should have moved, got {:?}",
        read().columns
    );
    assert!(
        read().marked.is_empty(),
        "and a press on the board beside the cards lets the marks go"
    );
}
