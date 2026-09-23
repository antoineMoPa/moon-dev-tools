//! The pane a task is written on before it exists, and the empty card that stands for it on the
//! board meanwhile.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, click_at, seeded_fixture, settle, type_letter};

/// A column's `+` opens a pane to write the new task on, with the keyboard in its title box,
/// and nothing is created until `[create]` is pressed: the task's folder is named after its
/// title and keeps that name for good, so there is no task until there is a name for it.
///
/// `[create]` makes the card and turns this very pane into that task's own, with what was
/// written in the notes box carried into the task's `notes.md`.
#[test]
fn a_new_task_is_written_on_its_pane_before_it_exists() {
    const BOARD_WIDTH: f32 = 640.0;

    let fixture = seeded_fixture("new-task-pane");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);
    let draft_open = Arc::new(AtomicBool::new(false));
    let draft_open_in_ui = Arc::clone(&draft_open);
    // Where the column is holding a place for the card being written, while it is being
    // written: the empty card the board draws at that end.
    let card_held: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let card_held_in_ui = Arc::clone(&card_held);
    // The task the pane is of once there is one, which is what the writing made.
    let pane_of: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let pane_of_in_ui = Arc::clone(&pane_of);
    let tasks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tasks_in_ui = Arc::clone(&tasks);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            // The `+` is asked for rather than clicked: where it lands depends on the column,
            // and what this is about is the pane it opens.
            if compose_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::OpenNewTask(
                        crate::moontasks::ColumnId::new("todo"),
                        crate::moontasks::ColumnEnd::Top,
                    ),
                );
            }
            app.draw(ui);
            draft_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
                    .is_some(),
                Ordering::Relaxed,
            );
            *card_held_in_ui.lock().expect("expected the held card") = app
                .model
                .board
                .card_being_written
                .as_ref()
                .map(|pending| pending.column.to_string());
            *pane_of_in_ui.lock().expect("expected the pane") = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::Start { .. }))
                .and_then(|(_, pane)| pane.task_id().map(str::to_string));
            *tasks_in_ui.lock().expect("expected the tasks") = app
                .model
                .board
                .tasks
                .iter()
                .map(|task| task.title.clone())
                .collect();
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read .moontasks"
    );
    let before = tasks.lock().expect("expected the tasks").len();
    compose.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || draft_open.load(Ordering::Relaxed)),
        "the `+` should have opened a pane to write the new task on"
    );
    harness.run_steps(2);
    assert_eq!(
        tasks.lock().expect("expected the tasks").len(),
        before,
        "an unnamed task is not a task: nothing should have been created yet"
    );
    assert_eq!(
        *card_held.lock().expect("expected the held card"),
        Some("todo".to_string()),
        "the column should be holding an empty card where this one will land"
    );

    // The title box is the upper of the pane's two boxes, and it is the one the keyboard is in
    // - so what is typed next goes into the name rather than nowhere.
    use egui_kittest::kittest::Queryable as _;
    let mut boxes: Vec<_> = harness
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .map(|node| (node.rect(), node.is_focused()))
        .filter(|(rect, _)| rect.center().x > BOARD_WIDTH)
        .collect();
    boxes.sort_by(|one, other| one.0.center().y.total_cmp(&other.0.center().y));
    assert!(
        boxes
            .first()
            .expect("expected the new task's pane to draw a title box")
            .1,
        "the title box should have the keyboard"
    );

    type_letter(&mut harness, egui::Key::P, "Parse");
    // Named, and still not a task: the button under the boxes is what makes one.
    assert_eq!(
        tasks.lock().expect("expected the tasks").len(),
        before,
        "a title typed is not a task either: [create] is what makes one"
    );

    let create = harness.get_by_label("[create]").rect().center();
    click_at(&mut harness, create);
    assert!(
        settle(&mut harness, || tasks
            .lock()
            .expect("expected the tasks")
            .iter()
            .any(|title| title == "Parse")),
        "[create] should have made the card"
    );
    assert!(
        settle(&mut harness, || pane_of
            .lock()
            .expect("expected the pane")
            .is_some()),
        "and the pane the title was written on should have become that task's own"
    );
    assert!(
        !draft_open.load(Ordering::Relaxed),
        "the new-task pane is that task's pane now, not a second tab beside it"
    );
    assert!(
        card_held.lock().expect("expected the held card").is_none(),
        "and the empty card comes off the board, its own card having taken the place"
    );
}

/// Closing a new-task pane makes nothing, whatever was typed on it: `[create]` is what makes a
/// task, and closing the tab without pressing it is saying no to the task.
#[test]
fn closing_a_new_task_pane_makes_nothing() {
    let fixture = seeded_fixture("new-task-closed");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);
    let close = Arc::new(AtomicBool::new(false));
    let close_in_ui = Arc::clone(&close);
    let draft_open = Arc::new(AtomicBool::new(false));
    let draft_open_in_ui = Arc::clone(&draft_open);
    let tasks: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tasks_in_ui = Arc::clone(&tasks);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if compose_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::OpenNewTask(
                        crate::moontasks::ColumnId::new("todo"),
                        crate::moontasks::ColumnEnd::Top,
                    ),
                );
            }
            // The tab is closed the way its mark closes it, rather than clicked for: where
            // that mark lands is the tab strip's business.
            if close_in_ui.swap(false, Ordering::Relaxed)
                && let Some((pane, _)) = app
                    .model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
            {
                app.close_pane(pane);
            }
            app.draw(ui);
            draft_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
                    .is_some(),
                Ordering::Relaxed,
            );
            *tasks_in_ui.lock().expect("expected the tasks") = app
                .model
                .board
                .tasks
                .iter()
                .map(|task| task.title.clone())
                .collect();
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read .moontasks"
    );
    compose.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || draft_open.load(Ordering::Relaxed)),
        "the `+` should have opened a pane to write the new task on"
    );
    harness.run_steps(2);
    type_letter(&mut harness, egui::Key::P, "Parse");

    let before = tasks.lock().expect("expected the tasks").len();

    close.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || !draft_open.load(Ordering::Relaxed)),
        "the pane should have closed"
    );
    harness.run_steps(3);
    assert_eq!(
        tasks.lock().expect("expected the tasks").len(),
        before,
        "the title the pane was closed on should have gone with it"
    );
}

/// The empty card standing for a task being written carries the same cross a made card does,
/// and pressing it says no to the task: the pane it is being written on goes, and the board is
/// left as it was.
///
/// The cross is the only way to call the new task off from the board itself - the pane it is
/// written on is over on the right, and the card is what is in front of you.
#[test]
fn the_empty_card_of_a_new_task_is_deleted_by_its_cross() {
    const TASK: &str = "write-the-parser-1111";

    let fixture = seeded_fixture("new-task-cross");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);
    let draft_open = Arc::new(AtomicBool::new(false));
    let draft_open_in_ui = Arc::clone(&draft_open);
    let card_held = Arc::new(AtomicBool::new(false));
    let card_held_in_ui = Arc::clone(&card_held);
    let tasks = Arc::new(AtomicUsize::new(0));
    let tasks_in_ui = Arc::clone(&tasks);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if compose_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::OpenNewTask(
                        crate::moontasks::ColumnId::new("todo"),
                        crate::moontasks::ColumnEnd::Top,
                    ),
                );
            }
            app.draw(ui);
            draft_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
                    .is_some(),
                Ordering::Relaxed,
            );
            card_held_in_ui.store(
                app.model.board.card_being_written.is_some(),
                Ordering::Relaxed,
            );
            tasks_in_ui.store(app.model.board.tasks.len(), Ordering::Relaxed);
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)
            && tasks.load(Ordering::Relaxed) == 1),
        "the board never read the task it was seeded with"
    );
    compose.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || draft_open.load(Ordering::Relaxed)),
        "the `+` should have opened a pane to write the new task on"
    );
    harness.run_steps(2);
    assert!(
        card_held.load(Ordering::Relaxed),
        "the column should be holding an empty card where this one will land"
    );

    let cross = pending_cross_over(&harness, TASK);
    click_at(&mut harness, cross);
    harness.run_steps(3);
    assert!(
        !draft_open.load(Ordering::Relaxed),
        "the cross should have closed the pane the task was being written on"
    );
    assert!(
        !card_held.load(Ordering::Relaxed),
        "and taken the empty card standing for it off the board"
    );
    assert_eq!(
        tasks.load(Ordering::Relaxed),
        1,
        "and made nothing: the board is left with the card it had"
    );
}

/// The cross on the empty card drawn above `below`, worked out from that card's title: the two
/// cards are laid out the same way, so the mark stands at the same place across from a title
/// one card up the column.
fn pending_cross_over(harness: &Harness<'_>, below: &str) -> egui::Pos2 {
    use crate::native::{board::cards, widgets::CLOSE_MARK_SIZE};

    let title = harness
        .ctx
        .read_response(cards::card_drag_id(below))
        .expect("expected the card under the empty one to have been drawn")
        .rect;
    egui::pos2(
        title.right()
            + harness
                .ctx
                .style_of(egui::Theme::Dark)
                .spacing
                .item_spacing
                .x
            + CLOSE_MARK_SIZE / 2.0,
        title.top() - cards::PENDING_CARD_HEIGHT,
    )
}
