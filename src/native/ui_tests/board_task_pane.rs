//! The pane a card opens: what a click on a card reaches, and the boxes that pane is
//! written in.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, click_at, press_key, seeded_fixture, settle, type_letter};

/// A card is the way into the task as much as it is a record of it: clicking one opens the
/// task's own pane, whatever the task has running, and the pane says what that is.
///
/// The card answers two gestures on the same title, so the rename a double click opens is
/// checked here too: the first of the two clicks opens the task's tab, and the box that opens
/// after it must still be the one the keyboard is in.
#[test]
fn clicking_a_card_opens_the_task_and_says_what_it_has_running() {
    const TASK: &str = "write-the-parser-1111";
    // Where the board ends and the task's pane begins, which tells the pane's `[start]` from
    // the ones on the cards.
    const BOARD_WIDTH: f32 = 640.0;

    let fixture = seeded_fixture("card-click");
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
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);
    let shell_open = Arc::new(AtomicBool::new(false));
    let shell_open_in_ui = Arc::clone(&shell_open);
    let worked_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let worked_in_in_ui = Arc::clone(&worked_in);
    // The title as the card's rename box has it, while one is open.
    let renaming: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let renaming_in_ui = Arc::clone(&renaming);
    // Whether that box has the keyboard yet: it asks for it as it is drawn, and only has it
    // from the frame after, which is the one it is safe to type into.
    let typing_lands = Arc::new(AtomicBool::new(false));
    let typing_lands_in_ui = Arc::clone(&typing_lands);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::Start { task_id, .. } if task_id == TASK))
                    .is_some(),
                Ordering::Relaxed,
            );
            shell_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Terminal)
                    .is_some(),
                Ordering::Relaxed,
            );
            *worked_in_in_ui.lock().expect("poisoned") = super::marked_task(&app);
            *renaming_in_ui.lock().expect("poisoned") = app
                .model
                .board
                .renaming
                .as_ref()
                .map(|rename| rename.title.clone());
            typing_lands_in_ui.store(
                ui.ctx().memory(|memory| memory.focused()).is_some(),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );

    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the card should have opened the task's pane"
    );
    assert_eq!(
        *worked_in.lock().expect("poisoned"),
        Some(TASK.to_string()),
        "and the card is marked while that pane is in front"
    );

    // A shell started from the pane, which takes the pane's place.
    use egui_kittest::kittest::Queryable as _;
    let start_button = harness
        .get_all_by_label("[start]")
        .map(|node| node.rect().center())
        .find(|at| at.x > BOARD_WIDTH)
        .expect("expected the task's pane to draw a [start] button");
    click_at(&mut harness, start_button);
    harness.run_steps(3);
    let shell_row = harness.get_by_label("shell").rect().center();
    click_at(&mut harness, shell_row);
    assert!(
        settle(&mut harness, || shell_open.load(Ordering::Relaxed)
            && !pane_open.load(Ordering::Relaxed)),
        "starting the shell should have taken the task's pane's place"
    );

    // The card again, now that the task has a shell running: the click opens the task, not the
    // terminal, and the pane has read the board since.
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the card should open the task's pane whatever it has running"
    );
    // Stepped rather than settled: what is being waited for is on the window itself, which the
    // harness cannot be asked about from inside a closure holding it. The line the pane opened
    // with is gone now that the task has a shell, which is the pane having read the board
    // again rather than going on saying what was true when it first opened.
    let mut still_says_nothing_runs = true;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && still_says_nothing_runs {
        harness.step();
        still_says_nothing_runs = harness
            .query_by_label("nothing is running in this task yet")
            .is_some();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !still_says_nothing_runs,
        "the pane should have read the board again and stopped saying nothing is running"
    );
    // And what it says instead is the card's own row for that shell, on the pane: the way back
    // to the terminal, and the marks that stop it and take it off the task. Under the name the
    // shell's own tab carries, task title and all, so the row and the tab read as the same
    // thing.
    assert!(
        harness
            .get_all_by_label("Write the parser shell - 1")
            .any(|node| node.rect().center().x > BOARD_WIDTH),
        "the pane should list the task's shell the way its card does"
    );

    // The other thing the title answers to still answers: the first of the two clicks opens the
    // task, and the second opens the title for renaming, with the letters going there and not
    // into the tab that just opened.
    let press_and_release = |pressed| egui::Event::PointerButton {
        pos: card.center(),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(card.center()),
        press_and_release(true),
        press_and_release(false),
    ]);
    harness.step();
    harness
        .input_mut()
        .events
        .extend([press_and_release(true), press_and_release(false)]);
    harness.step();
    assert!(
        settle(&mut harness, || renaming.lock().expect("poisoned").is_some()
            && typing_lands.load(Ordering::Relaxed)),
        "a double click on the title opens it for renaming, with the keyboard in it"
    );
    assert_eq!(
        *renaming.lock().expect("poisoned"),
        Some("Write the parser".to_string()),
        "and the box opens on the title as it stands"
    );

    type_letter(&mut harness, egui::Key::X, "X");
    harness.run_steps(2);
    assert_eq!(
        *renaming.lock().expect("poisoned"),
        Some("Write the parserX".to_string()),
        "and the letters go into the title, not into the tab in front of it"
    );
}

/// A task with nothing running has nowhere for a click to go, so the click opens the start
/// window instead: what that task can start, and the card marked while it is in front.
///
/// Starting the shell from there is the end of the window - it was standing in for the shell,
/// and the shell arrives in its place.
#[test]
fn a_task_with_nothing_running_opens_its_start_window() {
    const TASK: &str = "write-the-parser-1111";

    // A second task, for the start window opened once a shell is already on screen: that one
    // lands among the shell's tabs rather than in a column of its own.
    const OTHER: &str = "fix-the-login-page-2222";

    // Where the board ends and the start window's column begins, in the window this test
    // builds: what tells the window's own `[start]` from the cards' ones.
    const BOARD_WIDTH: f32 = 640.0;

    let fixture = seeded_fixture("start-window");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        &format!(".moontasks/{OTHER}/metadata.json"),
        "{\n  \"title\": \"Fix the login page\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000001,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let start_window_open = Arc::new(AtomicBool::new(false));
    let start_window_open_in_ui = Arc::clone(&start_window_open);
    let shell_open = Arc::new(AtomicBool::new(false));
    let shell_open_in_ui = Arc::clone(&shell_open);
    // Whether the other task's start window is the first tab of the frame the shell is in.
    let opened_first = Arc::new(AtomicBool::new(false));
    let opened_first_in_ui = Arc::clone(&opened_first);
    let worked_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let worked_in_in_ui = Arc::clone(&worked_in);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
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

            start_window_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(
                        |pane| matches!(pane, Pane::Start { task_id, .. } if task_id == TASK),
                    )
                    .is_some(),
                Ordering::Relaxed,
            );
            shell_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Terminal)
                    .is_some(),
                Ordering::Relaxed,
            );
            *worked_in_in_ui.lock().expect("poisoned") = super::marked_task(&app);
            opened_first_in_ui.store(
                app.model
                    .layout
                    .find_pane(
                        |pane| matches!(pane, Pane::Start { task_id, .. } if task_id == OTHER),
                    )
                    .and_then(|(pane, _)| {
                        app.model.layout.frame_of(pane).map(|frame| (pane, frame))
                    })
                    .and_then(|(pane, frame)| {
                        let frame = app.model.layout.frame(frame)?;
                        Some(
                            frame.panes().first() == Some(&pane)
                                && frame.panes().iter().any(|pane| {
                                    app.model.layout.pane(*pane).is_some_and(|pane| {
                                        pane.kind() == crate::native::panes::PaneKind::Terminal
                                    })
                                }),
                        )
                    })
                    .unwrap_or(false),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the tasks out of .moontasks"
    );

    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || start_window_open.load(Ordering::Relaxed)),
        "clicking a card with nothing running did not open its start window"
    );
    assert_eq!(
        *worked_in.lock().expect("poisoned"),
        Some(TASK.to_string()),
        "and the card is marked while that window is in front"
    );

    // Off the card, so the picture is of the window rather than of a card holding its offers
    // out under the pointer.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(500.0, 690.0)));
    harness.run_steps(3);
    harness.snapshot("moontasks-start-window");

    // The window's own `[start]`, which is the card's button drawn again: the cards have one
    // each too, so the one in the right-hand column is the one this presses.
    use egui_kittest::kittest::Queryable as _;
    let start_button = harness
        .get_all_by_label("[start]")
        .map(|node| node.rect().center())
        .find(|at| at.x > BOARD_WIDTH)
        .expect("expected the start window to draw a [start] button");
    click_at(&mut harness, start_button);
    harness.run_steps(3);
    let shell_row = harness.get_by_label("shell").rect().center();
    click_at(&mut harness, shell_row);
    assert!(
        settle(&mut harness, || shell_open.load(Ordering::Relaxed)
            && !start_window_open.load(Ordering::Relaxed)),
        "starting the shell should have taken the start window's place"
    );

    // The other task, now that a shell is on screen: its start window goes among that shell's
    // tabs, and in front of them - a tab opened to be read now and closed in a moment is no
    // use at the far end of a long strip.
    //
    // The shell's column took its share of the board's, and the cards are still walking to
    // where that leaves them - a rect read mid-walk is a click that lands beside the card
    // rather than on it. So the rect is read again and clicked again until one of them lands,
    // rather than betting on the walk being over after some number of frames.
    let mut landed = false;
    for _ in 0..8 {
        harness.run_steps(4);
        let other_card = harness
            .ctx
            .read_response(crate::native::board::cards::card_drag_id(OTHER))
            .expect("expected the other card to have been drawn")
            .rect;
        click_at(&mut harness, other_card.center());
        harness.run_steps(4);
        if opened_first.load(Ordering::Relaxed) {
            landed = true;
            break;
        }
    }
    assert!(
        landed,
        "the start window should be the first tab of the frame the shell is in"
    );
}

/// A shell started from a card is one of that task's tabs, so it joins the column beside the
/// board - the one the start windows and the other tasks' shells are already in - rather than
/// splitting the workspace again.
///
/// The column here holds a start window and nothing else, which is the case that used to be
/// missed: a shell would only join a frame that already had a shell in it, so every agent
/// started from the board while a task was open took a column of its own.
#[test]
fn a_shell_started_from_a_card_joins_the_column_beside_the_board() {
    use egui_kittest::kittest::Queryable as _;

    const TASK: &str = "write-the-parser-1111";
    const OTHER: &str = "fix-the-login-page-2222";

    let fixture = seeded_fixture("card-shell-column");
    for (task_id, title) in [
        (TASK, "Write the parser"),
        (OTHER, "Fix the login page"),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"todo\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let start_window_open = Arc::new(AtomicBool::new(false));
    let start_window_open_in_ui = Arc::clone(&start_window_open);
    // How many frames the workspace is split into, and whether the shell landed among the
    // start window's tabs rather than beside them.
    let frames = Arc::new(AtomicUsize::new(0));
    let frames_in_ui = Arc::clone(&frames);
    let shell_beside_the_window = Arc::new(AtomicBool::new(false));
    let shell_beside_in_ui = Arc::clone(&shell_beside_the_window);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            start_window_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(
                        |pane| matches!(pane, Pane::Start { task_id, .. } if task_id == TASK),
                    )
                    .is_some(),
                Ordering::Relaxed,
            );
            frames_in_ui.store(app.model.layout.frame_count(), Ordering::Relaxed);
            let frame_of = |wanted: fn(&Pane) -> bool| {
                app.model
                    .layout
                    .find_pane(wanted)
                    .and_then(|(pane, _)| app.model.layout.frame_of(pane))
            };
            let shell = frame_of(|pane| pane.kind() == crate::native::panes::PaneKind::Terminal);
            let window = frame_of(|pane| pane.kind() == crate::native::panes::PaneKind::Start);
            shell_beside_in_ui.store(shell.is_some() && shell == window, Ordering::Relaxed);
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 2,
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the board never read the tasks out of .moontasks"
    );

    // One task open in a column of its own down the right, which is the workspace the shell is
    // then started into.
    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || start_window_open.load(Ordering::Relaxed)),
        "clicking the card should have opened the task's pane"
    );
    harness.run_steps(4);
    assert_eq!(
        frames.load(Ordering::Relaxed),
        2,
        "the task's pane should have taken the column beside the board"
    );

    // The other task's own `[start]`, off its card rather than off the pane. The card is found
    // by its title, which is what it is dragged by, and its button is the first one under that
    // title in the same column of the board.
    let other_title = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(OTHER))
        .expect("expected the other card to have been drawn")
        .rect;
    let start_button = harness
        .get_all_by_label("[start]")
        .map(|node| node.rect())
        // Drawn under that title and within the card's own width - the button sits at the
        // card's right-hand edge, so it is where it starts that is inside the card.
        .filter(|button| {
            button.top() > other_title.top() && other_title.x_range().contains(button.left())
        })
        .min_by(|one, other| one.top().total_cmp(&other.top()))
        .expect("expected the other card to draw a [start] button")
        .center();
    click_at(&mut harness, start_button);
    harness.run_steps(3);
    let shell_row = harness.get_by_label("shell").rect().center();
    click_at(&mut harness, shell_row);

    assert!(
        settle(&mut harness, || shell_beside_the_window
            .load(Ordering::Relaxed)),
        "the shell should have joined the column the task's pane is in"
    );
    assert_eq!(
        frames.load(Ordering::Relaxed),
        2,
        "and the workspace should not have been split again for it"
    );
}

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

    let create = harness
        .get_by_label("[create]")
        .rect()
        .center();
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
        card_held
            .lock()
            .expect("expected the held card")
            .is_none(),
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

/// A task deleted while its pane is open takes the tab with it: a tab standing there saying
/// the task is gone is a tab you have to close yourself.
#[test]
fn deleting_a_task_closes_the_tab_its_pane_was_in() {
    const TASK: &str = "write-the-parser-1111";

    let fixture = seeded_fixture("task-deleted");
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
    let open_task = Arc::new(AtomicBool::new(false));
    let open_task_in_ui = Arc::clone(&open_task);
    let delete = Arc::new(AtomicBool::new(false));
    let delete_in_ui = Arc::clone(&delete);
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if open_task_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::OpenStart {
                        task_id: TASK.to_string(),
                        title: "Write the parser".to_string(),
                        opens_on: crate::native::board::actions::TaskPaneBox::Neither,
                    },
                );
            }
            // Deleted the way the card's mark deletes it, rather than clicked for: what this is
            // about is the tab that was open on it.
            if delete_in_ui.swap(false, Ordering::Relaxed) {
                crate::native::board::actions::apply(
                    &mut app,
                    crate::native::board::BoardAction::Delete(TASK.to_string()),
                );
            }
            app.draw(ui);
            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::Start { .. }))
                    .is_some(),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read .moontasks"
    );
    open_task.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "the task's pane should have opened"
    );

    delete.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || !pane_open.load(Ordering::Relaxed)),
        "the deleted task's tab should have closed itself"
    );
}

/// A task on the board, and its pane opened by a click on its card - the window every test of
/// the pane's boxes starts from.
fn a_task_pane_open_on(name: &str, task_id: &str) -> (Fixture, Harness<'static>) {
    let fixture = seeded_fixture(name);
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Start)
                    .is_some(),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );
    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(task_id))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the card should have opened the task's pane"
    );
    (fixture, harness)
}

/// The title box and the notes box of the task's pane, in that order.
///
/// Both are multiline boxes - a title wraps rather than scrolling sideways - so they are told
/// apart by where they sit: on the task's side of the window rather than the board's, which is
/// what tells the title box from the board's own filter box, and the title above the notes.
fn the_panes_boxes(harness: &Harness<'_>) -> (egui::Pos2, egui::Pos2) {
    // Where the board ends and the task's pane begins.
    const BOARD_WIDTH: f32 = 640.0;

    use egui_kittest::kittest::Queryable as _;
    let mut boxes: Vec<egui::Pos2> = harness
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .map(|node| node.rect().center())
        .filter(|at| at.x > BOARD_WIDTH)
        .collect();
    boxes.sort_by(|one, other| one.y.total_cmp(&other.y));
    assert_eq!(
        boxes.len(),
        2,
        "expected the task's pane to draw a title box and a notes box"
    );
    (boxes[0], boxes[1])
}

/// What the notes box is showing, read off the box itself rather than off the file - the box
/// is what the typing is in, and what it says is the thing a lost word is lost from.
fn the_notes_box_says(harness: &Harness<'_>) -> Option<String> {
    use egui_kittest::kittest::Queryable as _;
    let notes_box = the_panes_boxes(harness).1;
    harness
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .find(|node| node.rect().center() == notes_box)
        .and_then(|node| node.value())
}

/// The task's own pane is where a task is written as well as started: the title box renames it
/// and the notes box writes its `notes.md`, both without a file being opened beside them.
#[test]
fn a_tasks_pane_writes_its_title_and_its_notes() {
    const TASK: &str = "write-the-parser-1111";

    let (fixture, mut harness) = a_task_pane_open_on("task-editors", TASK);

    // The title, kept by the Enter that ends it.
    let (title_box, notes_box) = the_panes_boxes(&harness);
    click_at(&mut harness, title_box);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    type_letter(&mut harness, egui::Key::X, "X");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    let metadata = fixture.root.join(format!(".moontasks/{TASK}/metadata.json"));
    assert!(
        settle(&mut harness, || std::fs::read_to_string(&metadata)
            .is_ok_and(|written| written.contains("Write the parserX"))),
        "the title box should have renamed the task, saw {:?}",
        std::fs::read_to_string(&metadata)
    );

    // The notes, kept on their own a moment after the typing stops.
    click_at(&mut harness, notes_box);
    harness.run_steps(2);
    type_letter(&mut harness, egui::Key::S, "Ship it by Friday");
    let notes = fixture.root.join(format!(".moontasks/{TASK}/notes.md"));
    assert!(
        settle(&mut harness, || std::fs::read_to_string(&notes)
            .is_ok_and(|written| written.contains("Ship it by Friday"))),
        "the notes box should have written notes.md, saw {:?}",
        std::fs::read_to_string(&notes)
    );
}

/// A change to `notes.md` beside the box reaches the box.
///
/// The box passes over the board's answers while it is waiting for its own writing to be read
/// back, so this is the other side of that: an answer nobody here wrote - an agent writing the
/// task's notes, or the file open in an editor - is what the box is for showing, and it takes
/// it within a poll.
#[test]
fn notes_written_beside_the_box_reach_it() {
    const TASK: &str = "write-the-parser-2222";
    const WRITTEN_BESIDE: &str = "The parser is where the agent got to";

    let (fixture, mut harness) = a_task_pane_open_on("task-notes-beside", TASK);
    fixture.write(&format!(".moontasks/{TASK}/notes.md"), WRITTEN_BESIDE);

    // Stepped here rather than through `settle`, which cannot hand the box to its condition:
    // what is being waited for is drawn, and reading it needs the harness the steps are on.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut shown = the_notes_box_says(&harness);
    while Instant::now() < deadline && shown.as_deref() != Some(WRITTEN_BESIDE) {
        harness.step();
        shown = the_notes_box_says(&harness);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        shown.as_deref(),
        Some(WRITTEN_BESIDE),
        "the notes box should have taken what was written beside it"
    );
}

/// A triple click on a card's title is a double click and one more: the double opens the title
/// for renaming, and the third click selects the whole of it, so the next letters replace the
/// title rather than landing inside it. The third click is the awkward one - it is routed
/// against the frame where the title was still a label, so the rename box never hears it and
/// has to read it off the pointer.
#[test]
fn triple_clicking_the_title_selects_all_of_it() {
    const TASK: &str = "write-the-parser-1111";

    let fixture = seeded_fixture("card-triple-click");
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
    let pane_open = Arc::new(AtomicBool::new(false));
    let pane_open_in_ui = Arc::clone(&pane_open);
    // The title as the card's rename box has it, while one is open.
    let renaming: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let renaming_in_ui = Arc::clone(&renaming);
    // Whether that box has the keyboard yet: it asks for it as it is drawn, and only has it
    // from the frame after, which is the one it is safe to type into.
    let typing_lands = Arc::new(AtomicBool::new(false));
    let typing_lands_in_ui = Arc::clone(&typing_lands);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        // Three clicks are a triple within 0.6 seconds of the first, on egui's clock, which
        // ticks by this much every step. The default quarter second would leave the third
        // click 0.1 seconds inside that window - too close to a timing to be testing one.
        .with_step_dt(0.05)
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            pane_open_in_ui.store(
                app.model
                    .layout
                    .find_pane(
                        |pane| matches!(pane, Pane::Start { task_id, .. } if task_id == TASK),
                    )
                    .is_some(),
                Ordering::Relaxed,
            );
            *renaming_in_ui.lock().expect("poisoned") = app
                .model
                .board
                .renaming
                .as_ref()
                .map(|rename| rename.title.clone());
            typing_lands_in_ui.store(
                ui.ctx().memory(|memory| memory.focused()).is_some(),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );

    // The first click of the triple opens the task's pane, whose column takes its share of the
    // board's width and sets the cards walking to where that leaves them. The pane is opened
    // ahead of the gesture instead, and the triple is tried again until all three of its
    // clicks land on where the title has walked to, rather than betting on the walk being
    // over after some number of frames.
    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the card should have opened the task's pane"
    );

    let mut selected = false;
    for _ in 0..8 {
        // Enough of a pause that a click of an attempt that missed cannot be counted into
        // this one's triple.
        harness.run_steps(14);
        let card = harness
            .ctx
            .read_response(crate::native::board::cards::card_drag_id(TASK))
            .expect("expected the card to have been drawn")
            .rect;

        // Three presses one frame apart, by hand rather than through `click_at`: the settle
        // steps in there would put whole frames between the clicks, and the point is the
        // tight ones, where the third press falls on the frame the label has only just left.
        let press_and_release = |pressed| egui::Event::PointerButton {
            pos: card.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(card.center()),
            press_and_release(true),
            press_and_release(false),
        ]);
        harness.step();
        harness
            .input_mut()
            .events
            .extend([press_and_release(true), press_and_release(false)]);
        harness.step();
        harness
            .input_mut()
            .events
            .extend([press_and_release(true), press_and_release(false)]);
        harness.run_steps(3);

        if renaming.lock().expect("poisoned").is_some() && typing_lands.load(Ordering::Relaxed)
        {
            selected = true;
            break;
        }
    }
    assert!(
        selected,
        "a triple click on the title opens it for renaming, with the keyboard in it"
    );
    assert_eq!(
        *renaming.lock().expect("poisoned"),
        Some("Write the parser".to_string()),
        "and the box opens on the title as it stands"
    );

    // The whole title was selected, so one letter is the whole of what remains - where the
    // double click's box, tested above, has the letter joining the title instead.
    type_letter(&mut harness, egui::Key::Y, "Y");
    harness.run_steps(2);
    assert_eq!(
        *renaming.lock().expect("poisoned"),
        Some("Y".to_string()),
        "a letter typed after the triple click should replace the whole title"
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
