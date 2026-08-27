//! The moontasks board: what it draws, what a query leaves showing, and its cards.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;
use egui_kittest::SnapshotOptions;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, press_key, seeded_fixture, settle, type_letter};

/// The board is the repo's `.moontasks` folder, so the fixture writes the folder and the
/// window is expected to show exactly what is in it.
#[test]
fn the_moontasks_board_draws_what_is_in_the_repo() {
    let fixture = seeded_fixture("board");
    // Written by hand rather than through the service: the ids a real one generates carry a
    // uuid, and the point here is a picture that is the same on every run.
    for (task_id, title, status) in [
        ("write-the-parser-1111", "Write the parser", "todo"),
        (
            "fix-the-login-page-2222",
            "Fix the login page",
            "in_progress",
        ),
        ("drop-the-old-api-3333", "Drop the old API", "done"),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // The new-task box is opened rather than clicked for: where the `+` lands depends on the
    // column, and what this checks is the box it opens.
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);
    // Which end of the column that box is standing at, which is the `+` that would have been
    // pressed to open it.
    let at_bottom = Arc::new(AtomicBool::new(false));
    let at_bottom_in_ui = Arc::clone(&at_bottom);
    let open_shell = Arc::new(AtomicBool::new(false));
    let open_shell_in_ui = Arc::clone(&open_shell);
    let shell_requested = Arc::new(AtomicBool::new(false));
    let shell_requested_in_ui = Arc::clone(&shell_requested);
    let shell_ready = Arc::new(AtomicBool::new(false));
    let shell_ready_in_ui = Arc::clone(&shell_ready);
    let shell_command_sent = Arc::new(AtomicBool::new(false));
    let shell_command_sent_in_ui = Arc::clone(&shell_command_sent);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            // Only once the review has opened: opening it replaces the whole arrangement.
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if compose_in_ui.load(Ordering::Relaxed) {
                app.model.board.composer_in = Some(crate::moontasks::ColumnId::new("todo"));
                app.model.board.composer_at = if at_bottom_in_ui.load(Ordering::Relaxed) {
                    crate::moontasks::ColumnEnd::Bottom
                } else {
                    crate::moontasks::ColumnEnd::Top
                };
            } else {
                app.model.board.composer_in = None;
            }
            if open_shell_in_ui.load(Ordering::Relaxed)
                && !shell_requested_in_ui.swap(true, Ordering::Relaxed)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Terminal { command: None });
            }
            app.draw(ui);
            if open_shell_in_ui.load(Ordering::Relaxed)
                && !shell_command_sent_in_ui.load(Ordering::Relaxed)
                && let Some(terminal) = app.terminals.values().next()
            {
                terminal
                    .send(b"clear; printf '\\033]0;terminal\\007Moon tools workspace\\n\\nTasks on the board, agents and shells at 'hand'.\\n$ '; sleep 30\n")
                    .expect("expected to write the screenshot text to the shell");
                shell_command_sent_in_ui.store(true, Ordering::Relaxed);
            }
            if let Some(terminal) = app.terminals.values_mut().next() {
                terminal.poll();
                shell_ready_in_ui.store(
                    terminal
                        .visible_text()
                        .is_ok_and(|screen| screen.contains("agents and shells at hand")),
                    Ordering::Relaxed,
                );
            }
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 3,
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the board never read the three tasks out of .moontasks"
    );

    harness.run_steps(3);
    harness.snapshot("moontasks-board");

    // A card under the pointer, which is what brings out its offer to start the notes: above,
    // the row it stands in is held open and empty, so the card is the same height either way.
    // The corner of the card is pointed at rather than the middle of it, because a widget
    // under the pointer would draw its tooltip over the picture.
    let handle = harness
        .ctx
        .read_response(egui::Id::new((
            "moontask-card",
            &"write-the-parser-1111".to_string(),
        )))
        .expect("expected the first card to have been drawn")
        .rect;
    // Just under the title, at the far end of the row from the offer itself.
    let empty = egui::pos2(handle.right() - 12.0, handle.bottom() + 12.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(empty));
    // Long enough for the fade to have run out, so the picture is of the offers all the way
    // up rather than of a moment on the way there.
    harness.run_steps(3);
    harness.snapshot("moontasks-card-pointed-at");
    // Everything the card starts is on the one menu, which is what `[start]` opens: a review
    // of the repo, a shell in the task, and an agent - the ones this machine has.
    use egui_kittest::kittest::Queryable as _;

    harness
        .get_all_by_label("[start]")
        .next()
        .expect("expected the first card to offer [start]")
        .click();
    harness.run_steps(3);
    // The pointer moved down into the menu, which hangs below the card: the card holds its
    // offers out for as long as its menu is up, rather than fading away under the hand
    // reaching into it.
    let into_menu = harness.get_by_label("shell").rect().center();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(into_menu));
    harness.run_steps(3);
    harness.snapshot("moontasks-start-menu");

    // Off the cards again, so the pictures below are of a board nothing is pointed at, with
    // the menu shut behind them.
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(1200.0, 600.0)));
    harness.run_steps(3);

    // And the new-task box the `+` on the TODO column opens, standing where its card will go.
    compose.store(true, Ordering::Relaxed);
    // Its title box has focus, and a blinking caret would make the image differ run to run.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    harness.snapshot("moontasks-new-task");

    // And the same box opened by the `+` under the last card, standing at the bottom, where a
    // card added from there will appear.
    at_bottom.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    harness.snapshot("moontasks-new-task-at-the-bottom");

    // The main workspace: the board stays visible while a task's shell works beside it.
    compose.store(false, Ordering::Relaxed);
    open_shell.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if shell_ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(shell_ready.load(Ordering::Relaxed), "the shell never drew its output");
    harness.run_steps(3);
    harness.snapshot_options(
        "moontasks-workspace",
        &SnapshotOptions::new().output_path("docs/assets"),
    );
}

/// The filter over the columns: what a query leaves showing.
///
/// cmd+F is how the box is reached from the board, and a card is found by its title or by the
/// notes under it - both are on the card, so both are what a query looks through. A column the
/// query empties still holds its cards, and says so.
#[test]
fn a_query_leaves_the_board_showing_the_cards_that_match_it() {
    let fixture = seeded_fixture("board-filter");
    for (task_id, title, status, notes) in [
        (
            "write-the-parser-1111",
            "Write the parser",
            "todo",
            "the lexer chokes on nested comments",
        ),
        ("fix-the-login-page-2222", "Fix the login page", "todo", ""),
        (
            "drop-the-old-api-3333",
            "Drop the old API",
            "in_progress",
            "the login page is its last caller",
        ),
        ("ship-the-release-4444", "Ship the release", "done", ""),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
        if !notes.is_empty() {
            fixture.write(&format!(".moontasks/{task_id}/notes.md"), notes);
        }
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // What the board is filtered by, read out of the model rather than assumed: the box is
    // typed into, and this is what the typing reached.
    let query = Arc::new(Mutex::new(String::new()));
    let query_in_ui = Arc::clone(&query);

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
            if let Ok(mut query) = query_in_ui.lock() {
                *query = app.model.board.filter.clone();
            }
            ready_in_ui.store(
                app.model.board.loaded
                    && app.model.board.tasks.len() == 4
                    && app
                        .model
                        .board
                        .tasks
                        .iter()
                        .any(|task| task.notes.contains("last caller")),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the board never read the four tasks and their notes out of .moontasks"
    );
    harness.run_steps(3);

    let showing = |harness: &Harness<'_>, task_id: &str| {
        harness
            .ctx
            .read_response(egui::Id::new(("moontask-card", &task_id.to_string())))
            .is_some()
    };
    assert!(
        showing(&harness, "ship-the-release-4444"),
        "every card is on the board before anything is typed"
    );

    // cmd+F over the board puts the keyboard in the filter box, which is where the query is
    // then typed.
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("login".to_string()));
    harness.run_steps(3);
    assert_eq!(
        query.lock().expect("expected the query").as_str(),
        "login",
        "cmd+F should have left the keyboard in the filter box"
    );

    // A caret blinking in the box would make the image differ run to run.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("moontasks-filtered");

    assert!(
        showing(&harness, "fix-the-login-page-2222"),
        "the card whose title the query is in stays"
    );
    assert!(
        showing(&harness, "drop-the-old-api-3333"),
        "and so does the one whose notes it is in"
    );
    assert!(
        !showing(&harness, "write-the-parser-1111"),
        "the card the query is nowhere in is left out"
    );
    assert!(
        !showing(&harness, "ship-the-release-4444"),
        "in every column, not only the ones with a match"
    );

    // Escape empties the box, and the board is whole again.
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(3);
    assert!(
        query.lock().expect("expected the query").is_empty(),
        "Escape should have emptied the box"
    );
    assert!(
        showing(&harness, "write-the-parser-1111"),
        "and the cards it was hiding are back"
    );
}

/// The attach modal offers the sessions the agents themselves have on this machine, which
/// is nothing a test can rely on - so the listing is injected, and what is checked is the
/// modal itself: what it shows, and Escape closing it.
#[test]
fn the_attach_modal_lists_the_agents_own_sessions() {
    use crate::api::AgentKind;

    let fixture = seeded_fixture("board-attach");
    fixture.write(
        ".moontasks/write-the-parser-1111/metadata.json",
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened_in_ui = Arc::new(AtomicBool::new(false));
    // The board and its card have been read: the snapshot's background is settled.
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let inject = Arc::new(AtomicBool::new(false));
    let inject_in_ui = Arc::clone(&inject);
    // What the modal's state is right now, read back out of the draw closure.
    let picker_open = Arc::new(AtomicBool::new(false));
    let picker_open_in_ui = Arc::clone(&picker_open);

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
            // Once, when the test says so - the way OpenAttachPicker fills it in, but with
            // sessions this machine is known not to have.
            if inject_in_ui.swap(false, Ordering::Relaxed) {
                app.model.board.attach_picker = Some(crate::native::model::AttachPicker {
                    task_id: "write-the-parser-1111".to_string(),
                    task_title: "Write the parser".to_string(),
                    sessions: Some(vec![
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::Claude,
                            id: "3f37e6a1-4a11-4333-8444-555555555555".to_string(),
                            title: "Fix the login page".to_string(),
                            updated_at_unix: 1_700_003_600,
                        },
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::OpenCode,
                            id: "ses_012f01ba5ffeTRe0q5MsyL9wbO".to_string(),
                            title: "Character-precise review selection".to_string(),
                            updated_at_unix: 1_699_900_000,
                        },
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::Codex,
                            id: "019efeff-2a80-7b11-b0b1-c5ab3e09b353".to_string(),
                            title: "Rewrite the scheduler".to_string(),
                            updated_at_unix: 1_699_800_000,
                        },
                    ]),
                    error: None,
                    manual_id: String::new(),
                    manual_agent: None,
                });
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 1,
                Ordering::Relaxed,
            );
            picker_open_in_ui.store(
                app.model.board.attach_picker.is_some(),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the board never read the task out of .moontasks"
    );

    inject.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    assert!(
        picker_open.load(Ordering::Relaxed),
        "the injected modal never showed"
    );
    harness.snapshot("moontasks-attach-session");

    // Escape is the way out that touches nothing.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(
        !picker_open.load(Ordering::Relaxed),
        "escape did not close the attach modal"
    );
}

/// A card's title is whatever someone typed on the way past, and some of them are a sentence.
/// The column is a fixed width, so a long title has to be cut into it rather than widen it -
/// widening one column used to push the rest of the board off the side of the window.
#[test]
fn a_long_task_title_is_cut_into_its_column() {
    let fixture = seeded_fixture("board-long-title");
    for (task_id, title, status) in [
        (
            "long-title-1111",
            "Rework the dispatch queue so held comments survive a restart, and take the \
             chance to rename everything around it while we are here",
            "todo",
        ),
        (
            "fix-the-login-page-2222",
            "Fix the login page",
            "in_progress",
        ),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

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
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 2,
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the board never read the two tasks out of .moontasks"
    );

    harness.run_steps(3);
    harness.snapshot("moontasks-long-title");
}

/// A card's notes are its description: their first lines sit under the title and a click
/// opens `notes.md` in a pane down the right, straight into the editor. A task with none
/// offers `[add notes]` instead, and its first open is what makes the file real.
#[test]
fn a_cards_notes_open_beside_the_board_ready_to_edit() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("board-notes");
    for (task_id, title) in [
        ("write-the-parser-1111", "Write the parser"),
        ("fix-the-login-page-2222", "Fix the login page"),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"todo\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }
    // One task has notes to show; the other has none, and offers to start them.
    fixture.write(
        ".moontasks/write-the-parser-1111/notes.md",
        "Ship it by Friday, working top down.\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // The file panes the clicks end up opening, watched for from out here: the app is the
    // closure's from the moment the harness is built.
    let file_panes = Arc::new(Mutex::new(Vec::<String>::new()));
    let file_panes_in_ui = Arc::clone(&file_panes);

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
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 2,
                Ordering::Relaxed,
            );
            if let Ok(mut panes) = file_panes_in_ui.lock() {
                *panes = app
                    .model
                    .layout
                    .panes()
                    .filter_map(|(_, pane)| match pane {
                        Pane::File { file_path, .. } => Some(file_path.clone()),
                        _ => None,
                    })
                    .collect();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the board never read the two tasks out of .moontasks"
    );
    harness.run_steps(3);

    // The description shows on the one card, and the way to start one on the other.
    let panes_open = || {
        file_panes
            .lock()
            .map(|panes| panes.clone())
            .unwrap_or_default()
    };
    let wait_for = |harness: &mut Harness<'_>, path: &str| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && !panes_open().iter().any(|open| open == path) {
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    harness
        .get_by_label_contains("Ship it by Friday")
        .click();
    wait_for(&mut harness, ".moontasks/write-the-parser-1111/notes.md");
    assert!(
        panes_open().contains(&".moontasks/write-the-parser-1111/notes.md".to_string()),
        "clicking the description should have opened the notes, got {:?}",
        panes_open()
    );
    harness.run_steps(3);
    // Straight into the editor: the way to the rendered page on screen says which mode this is.
    assert!(
        harness.query_by_label("[preview]").is_some(),
        "the notes should have opened on the text, not the rendered page"
    );

    harness.get_by_label("[add notes]").click();
    wait_for(&mut harness, ".moontasks/fix-the-login-page-2222/notes.md");
    assert!(
        panes_open().contains(&".moontasks/fix-the-login-page-2222/notes.md".to_string()),
        "[add notes] should have opened the other task's notes, got {:?}",
        panes_open()
    );
    let started = fixture
        .root
        .join(".moontasks/fix-the-login-page-2222/notes.md");
    assert!(
        started.is_file(),
        "opening the notes is what makes the file real"
    );
}

/// A file on a card is a way back to the file: its path opens it in a pane, the way the
/// notes do. And `[start]` is where one is put there, through the file finder - a pick there
/// lands on the card and opens, rather than only opening.
#[test]
fn a_linked_file_opens_from_its_card_and_start_links_another() {
    let fixture = seeded_fixture("board-files");
    fixture.write(
        ".moontasks/write-the-parser-1111/metadata.json",
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": [\n    {\n      \
         \"id\": \"file-1111\",\n      \"kind\": \"file\",\n      \
         \"file_path\": \"src/extra.rs\",\n      \"started_at_unix\": 1700000001\n    }\n  ]\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let file_panes = Arc::new(Mutex::new(Vec::<String>::new()));
    let file_panes_in_ui = Arc::clone(&file_panes);
    // What the finder has searched for, so the test can wait for the answer to the typed name.
    let searched = Arc::new(Mutex::new(None::<String>));
    let searched_in_ui = Arc::clone(&searched);
    // How many resources the card has, read back out of the board's own last answer.
    let linked = Arc::new(Mutex::new(Vec::<String>::new()));
    let linked_in_ui = Arc::clone(&linked);

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
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 1,
                Ordering::Relaxed,
            );
            if let Ok(mut panes) = file_panes_in_ui.lock() {
                *panes = app
                    .model
                    .layout
                    .panes()
                    .filter_map(|(_, pane)| match pane {
                        Pane::File { file_path, .. } => Some(file_path.clone()),
                        _ => None,
                    })
                    .collect();
            }
            if let Ok(mut searched) = searched_in_ui.lock() {
                *searched = app.model.palette.files.searched.clone();
            }
            if let Ok(mut linked) = linked_in_ui.lock() {
                *linked = app
                    .model
                    .board
                    .tasks
                    .iter()
                    .flat_map(|task| task.resources.iter())
                    .filter_map(|resource| resource.file_path.clone())
                    .collect();
            }
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );
    harness.run_steps(3);
    let panes_open = || {
        file_panes
            .lock()
            .map(|panes| panes.clone())
            .unwrap_or_default()
    };

    // The card carries the file the way it carries a run: a mark, then the path.
    harness.snapshot("moontasks-linked-file");

    // The linked file is on the card by its path, and the path opens it.
    use egui_kittest::kittest::Queryable as _;
    harness.get_by_label("src/extra.rs").click();
    assert!(
        settle(&mut harness, || panes_open().contains(&"src/extra.rs".to_string())),
        "clicking the linked file should have opened it, got {:?}",
        panes_open()
    );

    // `[start]` -> `file…` is the file finder, picking for this card: the pick is linked and
    // then opened.
    harness.get_by_label("[start]").click();
    harness.run_steps(3);
    harness.get_by_label("file…").click();
    harness.run_steps(3);
    for (key, letter) in [(egui::Key::L, "l"), (egui::Key::I, "i"), (egui::Key::B, "b")] {
        type_letter(&mut harness, key, letter);
    }
    assert!(
        settle(&mut harness, || {
            searched.lock().expect("poisoned").as_deref() == Some("lib")
        }),
        "the finder never searched for what was typed - is ag installed?"
    );
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    assert!(
        settle(&mut harness, || panes_open().contains(&"src/lib.rs".to_string())),
        "the picked file should have opened, got {:?}",
        panes_open()
    );
    assert!(
        settle(&mut harness, || linked
            .lock()
            .expect("poisoned")
            .contains(&"src/lib.rs".to_string())),
        "the picked file should be on the card, got {:?}",
        linked.lock().expect("poisoned")
    );
    let metadata = std::fs::read_to_string(
        fixture
            .root
            .join(".moontasks/write-the-parser-1111/metadata.json"),
    )
    .expect("failed to read the task's metadata");
    assert!(
        metadata.contains("\"file_path\": \"src/lib.rs\""),
        "the link should be written to the task folder: {metadata}"
    );
}
