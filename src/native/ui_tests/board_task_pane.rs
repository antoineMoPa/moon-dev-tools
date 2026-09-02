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

use super::{app_for, click_at, press_key, seeded_fixture, settle, type_letter};

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
    // to the terminal, and the marks that stop it and take it off the task.
    assert!(
        harness
            .get_all_by_label("shell")
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
                    .find_pane(|pane| matches!(pane, Pane::Start { task_id, .. } if task_id == TASK))
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

/// The task's own pane is where a task is written as well as started: the title box renames it
/// and the notes box writes its `notes.md`, both without a file being opened beside them.
#[test]
fn a_tasks_pane_writes_its_title_and_its_notes() {
    const TASK: &str = "write-the-parser-1111";
    // Where the board ends and the task's pane begins, which tells the pane's title box from
    // the board's own filter box.
    const BOARD_WIDTH: f32 = 640.0;

    let fixture = seeded_fixture("task-editors");
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
        .read_response(crate::native::board::cards::card_drag_id(TASK))
        .expect("expected the card to have been drawn")
        .rect;
    click_at(&mut harness, card.center());
    assert!(
        settle(&mut harness, || pane_open.load(Ordering::Relaxed)),
        "clicking the card should have opened the task's pane"
    );

    // The title, kept by the Enter that ends it.
    use egui_kittest::kittest::Queryable as _;
    // The title and the notes are both multiline boxes - a title wraps rather than scrolling
    // sideways - so the title is the upper of the two on the task's side of the window.
    let mut boxes: Vec<egui::Pos2> = harness
        .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
        .map(|node| node.rect().center())
        .filter(|at| at.x > BOARD_WIDTH)
        .collect();
    boxes.sort_by(|one, other| one.y.total_cmp(&other.y));
    let title_box = *boxes
        .first()
        .expect("expected the task's pane to draw a title box");
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
    let notes_box = *boxes
        .last()
        .expect("expected the task's pane to draw a notes box");
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
