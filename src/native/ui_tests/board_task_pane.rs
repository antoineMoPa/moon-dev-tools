//! The pane a card opens: what a click on a card reaches, the shells started from it, and the
//! tab deleting the task closes. What is written on the pane is in `board_task_pane_boxes`.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, click_at, seeded_fixture, settle, type_letter};

/// A card is the way into the task as much as it is a record of it: clicking one opens the
/// task's own pane, whatever the task has running, and the pane says what that is.
///
/// The card answers two gestures on the same title, so the rename a double click opens is
/// checked here too: the first of the two clicks opens the task's tab, and the box that opens
/// after it must still be the one the keyboard is in.
#[test]
fn clicking_a_card_opens_the_task_and_says_what_it_has_running() {
    const TASK: &str = "write-the-parser-1111";
    // Where the board ends and the task's pane begins, which tells the pane's `shell` button
    // from the `[start]` menus on the cards.
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

    // A shell started from the pane's own buttons, which takes the pane's place. The
    // task has nothing running, so the pane's only `shell` is that button.
    use egui_kittest::kittest::Queryable as _;
    let shell_button = harness
        .get_all_by_label("shell")
        .map(|node| node.rect().center())
        .find(|at| at.x > BOARD_WIDTH)
        .expect("expected the task's pane to draw a shell button");
    click_at(&mut harness, shell_button);
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
        settle(&mut harness, || renaming
            .lock()
            .expect("poisoned")
            .is_some()
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
    for (task_id, title) in [(TASK, "Write the parser"), (OTHER, "Fix the login page")] {
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
    // The menu's `shell`, which opens under the button, rather than the pane's own `shell`
    // button standing in its list across the window.
    let shell_row = harness
        .get_all_by_label("shell")
        .map(|node| node.rect().center())
        .min_by(|one, other| {
            one.distance(start_button)
                .total_cmp(&other.distance(start_button))
        })
        .expect("expected the menu to offer a shell");
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
