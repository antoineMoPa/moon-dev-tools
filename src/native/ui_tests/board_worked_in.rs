//! The card drawn apart from the others: the task the window is being worked in.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, click_at, seeded_fixture, settle};

/// The window says which task it is being worked in, so the board can draw that card apart
/// from the others.
///
/// The answer is the task's shell the keyboard was last in, and it stays that until another
/// task's shell takes the front - clicking onto the board, which is where the mark is read,
/// must not be what takes it away. It goes when the tab does: the shell is the only claim the
/// window has to be working in the task at all.
#[test]
fn the_task_being_worked_in_is_the_one_whose_shell_was_last_in_front() {
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
                    *shell_pane_in_ui.lock().expect("poisoned") = Some(
                        app.model.layout.add_pane(
                            frame,
                            Pane::Terminal {
                                terminal_id: "worked-in-shell".to_string(),
                                command: Some(crate::api::AgentKind::Claude),
                                task_id: Some("write-the-parser-1111".to_string()),
                            },
                            None,
                        ),
                    );
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
                seen.push(app.worked_in_task().map(str::to_string));
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
        vec![task.clone(), task, None],
        "the shell's task, then the same with the board in front, then nothing once the tab is \
         closed"
    );
}

/// A click on the board's own background takes the mark off the task being worked in: the
/// card goes back among the others without its tab having to close.
#[test]
fn clicking_the_boards_background_takes_the_mark_off_the_task() {
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
            *worked_in_in_ui.lock().expect("poisoned") =
                app.worked_in_task().map(str::to_string);
            *board_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .find_pane(|pane| pane.kind() == crate::native::panes::PaneKind::Tasks)
                .and_then(|(pane, _)| app.frames.pane_rect(pane))
                .filter(|_| app.model.board.loaded);
        });

    let task = Some("write-the-parser-1111".to_string());
    assert!(
        settle(&mut harness, || *worked_in.lock().expect("poisoned") == task
            && board.lock().expect("poisoned").is_some()),
        "the shell's task is marked, with the board open and read"
    );

    // Right of the last column and below the new-column slot's own button, where nothing of
    // the board's is drawn - and well above the corner the toasts stack up in, since the
    // shell this test names has one saying it could not be attached.
    let pane = board.lock().expect("poisoned").expect("the board was drawn");
    click_at(&mut harness, egui::pos2(pane.max.x - 12.0, pane.min.y + 160.0));
    assert!(
        settle(&mut harness, || worked_in.lock().expect("poisoned").is_none()),
        "a click on the board's background should take the mark off the task"
    );
}
