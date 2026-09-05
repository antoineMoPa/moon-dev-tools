//! The cards the board has marked: what marks them, and what lets them go.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{
    app_for, board_of, click_at, click_like_a_hand, notes_of, press_key, press_modifiers,
    seeded_fixture, settle, title_of,
};

/// A task's own tab coming to the front marks its card, so the board says which task the
/// window is being worked in - and the card stays marked while the board is read, since
/// clicking onto the board, which is where the mark is read, must not be what takes it away.
///
/// The mark outlives the tab that made it: it is the board's own mark now, the same one a
/// click on the card makes, and nothing about closing a shell says the task is no longer the
/// one being worked on. The board's background is what lets a mark go.
#[test]
fn a_tasks_tab_coming_to_the_front_marks_its_card() {
    let fixture = seeded_fixture("worked-in");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    /// What the test does between draws, one thing per frame it has drawn.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Step {
        OpenTheShell,
        OpenTheBoard,
        CloseTheShell,
        Done,
    }

    let step = Arc::new(Mutex::new(Step::OpenTheShell));
    let step_in_ui = Arc::clone(&step);
    // What `worked_in_task` answered after each of those, in the order they were done.
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_in_ui = Arc::clone(&seen);
    let shell_pane = Arc::new(Mutex::new(None));
    let shell_pane_in_ui = Arc::clone(&shell_pane);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            // Nothing is added until the review has opened: opening it replaces the whole
            // arrangement, which would take any pane put there before it with it.
            if !matches!(app.model.stage, crate::native::model::Stage::Ready) {
                app.draw(ui);
                return;
            }

            let mut step = step_in_ui.lock().expect("poisoned");
            match *step {
                Step::OpenTheShell => {
                    let frame = app.model.layout.active_frame();
                    *shell_pane_in_ui.lock().expect("poisoned") = Some(app.model.layout.add_pane(
                        frame,
                        Pane::Terminal {
                            terminal_id: "worked-in-shell".to_string(),
                            command: Some(crate::api::AgentKind::Claude),
                            task_id: Some("write-the-parser-1111".to_string()),
                        },
                        None,
                    ));
                    *step = Step::OpenTheBoard;
                }
                Step::OpenTheBoard => {
                    app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                    *step = Step::CloseTheShell;
                }
                Step::CloseTheShell => {
                    let pane = shell_pane_in_ui
                        .lock()
                        .expect("poisoned")
                        .expect("the shell's pane was opened first");
                    app.close_pane(pane);
                    *step = Step::Done;
                }
                Step::Done => {}
            }
            drop(step);

            app.draw(ui);
            let mut seen = seen_in_ui.lock().expect("poisoned");
            if seen.len() < 3 {
                seen.push(super::marked_task(&app));
            }
        });

    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").len() == 3),
        "expected three answers, got {:?}",
        seen.lock().expect("poisoned")
    );
    let task = Some("write-the-parser-1111".to_string());
    assert_eq!(
        *seen.lock().expect("poisoned"),
        vec![task.clone(), task.clone(), task],
        "the shell's task, still marked with the board in front, and still marked once the tab \
         is closed"
    );
}

/// A click on the board beside the cards lets the marks go: the cards go back among the
/// others without any tab having to close.
#[test]
fn clicking_the_board_beside_the_cards_lets_the_marks_go() {
    let fixture = seeded_fixture("worked-in-unfocus");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    // What has been opened so far: the shell one frame, the board the next, so the shell's
    // tab has been in front on its own before the board is.
    let opened = Arc::new(Mutex::new(0u8));
    let opened_in_ui = Arc::clone(&opened);
    let worked_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let worked_in_in_ui = Arc::clone(&worked_in);
    // Where the board's pane was drawn, for a click that lands on nothing in it.
    let board: Arc<Mutex<Option<egui::Rect>>> = Arc::new(Mutex::new(None));
    let board_in_ui = Arc::clone(&board);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            let mut opened = opened_in_ui.lock().expect("poisoned");
            if matches!(app.model.stage, crate::native::model::Stage::Ready) {
                match *opened {
                    0 => {
                        let frame = app.model.layout.active_frame();
                        app.model.layout.add_pane(
                            frame,
                            Pane::Terminal {
                                terminal_id: "worked-in-shell".to_string(),
                                command: Some(crate::api::AgentKind::Claude),
                                task_id: Some("write-the-parser-1111".to_string()),
                            },
                            None,
                        );
                        *opened = 1;
                    }
                    1 => {
                        app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                        *opened = 2;
                    }
                    _ => {}
                }
            }
            drop(opened);

            app.draw(ui);
            *worked_in_in_ui.lock().expect("poisoned") = super::marked_task(&app);
            *board_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Tasks)
                .and_then(|(pane, _)| app.frames.pane_rect(pane))
                .filter(|_| app.model.board.loaded);
        });

    let task = Some("write-the-parser-1111".to_string());
    assert!(
        settle(&mut harness, || *worked_in.lock().expect("poisoned")
            == task
            && board.lock().expect("poisoned").is_some()),
        "the shell's task is marked, with the board open and read"
    );

    // Right of the last column and below the new-column slot's own button, where nothing of
    // the board's is drawn - and well above the corner the toasts stack up in, since the
    // shell this test names has one saying it could not be attached.
    let pane = board
        .lock()
        .expect("poisoned")
        .expect("the board was drawn");
    click_at(
        &mut harness,
        egui::pos2(pane.max.x - 12.0, pane.min.y + 160.0),
    );
    assert!(
        settle(&mut harness, || worked_in
            .lock()
            .expect("poisoned")
            .is_none()),
        "a click on the board's background should take the mark off the task"
    );
}

/// cmd+click marks a card wherever on it the hand lands - a card is a stack of buttons, and a
/// click meant for the card must not press whichever of them it fell on.
///
/// shift+click takes the run between that card and the last one clicked, and cmd+click takes
/// one card back out of it: the run key and the one-card key, doing their two jobs.
#[test]
fn the_keys_mark_cards_wherever_on_them_the_hand_lands() {
    let (mut harness, seen, _repo) = board_of("marks-by-hand", &["fix-the-login-page-2222"]);
    let read = || seen.lock().expect("poisoned").clone();

    // On the card's description, which a plain click opens the task by, and next to its
    // `[start]` menu - neither of them answers while the key is down.
    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    let notes = notes_of(&harness, "fix-the-login-page-2222");
    click_like_a_hand(&mut harness, notes, egui::Modifiers::COMMAND);
    assert_eq!(
        read().marked,
        vec!["fix-the-login-page-2222".to_string()],
        "a cmd+click on the card's description should mark the card"
    );
    assert!(
        !read().page_open,
        "and open nothing, which is what its description does when it is clicked plainly"
    );

    // The run from that card up to the first, taken with shift.
    press_modifiers(&mut harness, egui::Modifiers::SHIFT);
    let first = title_of(&harness, "write-the-parser-1111");
    click_like_a_hand(&mut harness, first, egui::Modifiers::SHIFT);
    assert_eq!(
        read().marked,
        vec![
            "fix-the-login-page-2222".to_string(),
            "write-the-parser-1111".to_string()
        ],
        "shift+click should take the run between the two"
    );

    // And one of them taken back out with cmd, the other left marked.
    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    click_like_a_hand(&mut harness, first, egui::Modifiers::COMMAND);
    assert_eq!(
        read().marked,
        vec!["fix-the-login-page-2222".to_string()],
        "cmd+click should take that one card back out of the run"
    );
    press_modifiers(&mut harness, egui::Modifiers::NONE);
}

/// A cmd+click that drifts a little is still a cmd+click. A card is picked up by carrying it
/// somewhere, not by pressing it a moment too long or by a hand that is not quite still.
#[test]
fn a_cmd_click_that_drifts_marks_the_card_rather_than_picking_it_up() {
    let (mut harness, seen, _repo) = board_of("marks-not-drags", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    let card = title_of(&harness, "write-the-parser-1111");
    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(card));
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: card,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::COMMAND,
    });
    // Held a while, and drifting, the way a careful click does.
    for drift in [2.0, 4.0, 5.0, 6.0] {
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(card + egui::vec2(drift, drift)));
        harness.run_steps(2);
    }
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: card + egui::vec2(6.0, 6.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(3);
    press_modifiers(&mut harness, egui::Modifiers::NONE);

    assert_eq!(
        read().marked,
        vec!["write-the-parser-1111".to_string()],
        "the drifting cmd+click should have marked the card"
    );
    assert_eq!(
        read().column_of("write-the-parser-1111"),
        "todo",
        "and left it exactly where it was"
    );
}

/// Escape lets the marks go, for a hand already on the keyboard.
#[test]
fn escape_lets_the_marks_go() {
    let (mut harness, seen, _repo) = board_of("marks-escape", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    for task_id in ["write-the-parser-1111", "fix-the-login-page-2222"] {
        let at = title_of(&harness, task_id);
        click_like_a_hand(&mut harness, at, egui::Modifiers::COMMAND);
    }
    press_modifiers(&mut harness, egui::Modifiers::NONE);
    assert_eq!(read().marked.len(), 2);

    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    harness.run_steps(2);
    assert!(
        read().marked.is_empty(),
        "Escape should have let the marks go, got {:?}",
        read().marked
    );
}

/// A card let go of has nothing left open on it: the page a click opened is put away by the
/// click that lets that card go, whether that is Escape, the board beside the cards, or
/// another card being marked instead.
#[test]
fn letting_a_card_go_puts_its_page_away() {
    let (mut harness, seen, _repo) = board_of("marks-close-page", &[]);
    let read = || seen.lock().expect("poisoned").clone();

    let first = title_of(&harness, "write-the-parser-1111");
    click_like_a_hand(&mut harness, first, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || read().page_open),
        "clicking the card should have opened its page"
    );

    // Another card marked instead: the first card's page goes with its mark.
    let second = title_of(&harness, "fix-the-login-page-2222");
    click_like_a_hand(&mut harness, second, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || read().marked
            == vec!["fix-the-login-page-2222".to_string()]),
        "the second card should be the one marked"
    );
    assert!(
        settle(&mut harness, || read().pages_open
            == vec!["fix-the-login-page-2222".to_string()]),
        "and the only page open should be its own, got {:?}",
        read().pages_open
    );

    // And Escape, which lets the last mark go, puts that page away too.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    harness.run_steps(3);
    assert!(read().marked.is_empty(), "Escape lets the marks go");
    assert!(
        settle(&mut harness, || !read().page_open),
        "and the page of the card it let go of goes with it"
    );
}
