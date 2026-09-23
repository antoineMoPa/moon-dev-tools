//! What is on a task's pane: the start window a task with nothing running opens on, and the
//! title and notes boxes it is written in.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, click_at, press_key, seeded_fixture, settle, type_letter};

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
    // builds: what tells the window's own `shell` button from the cards' `[start]` menus.
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

    // The window's own `shell` button, one of the card's offers laid out in a list: the one in
    // the right-hand column is the one this presses, clear of anything the cards draw.
    use egui_kittest::kittest::Queryable as _;
    let shell_button = harness
        .get_all_by_label("shell")
        .map(|node| node.rect().center())
        .find(|at| at.x > BOARD_WIDTH)
        .expect("expected the start window to draw a shell button");
    click_at(&mut harness, shell_button);
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
    let metadata = fixture
        .root
        .join(format!(".moontasks/{TASK}/metadata.json"));
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

        if renaming.lock().expect("poisoned").is_some() && typing_lands.load(Ordering::Relaxed) {
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
