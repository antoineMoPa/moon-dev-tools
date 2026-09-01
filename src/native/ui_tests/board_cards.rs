//! A card's own offers: its notes, the files linked to it, and the sessions it can be given.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, press_key, seeded_fixture, settle, type_letter};

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
    // The task the window is being worked in, which a file opened from a card moves: the notes
    // of a task are that task's, so the board marks its card while they are in front.
    let worked_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let worked_in_in_ui = Arc::clone(&worked_in);

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
            *worked_in_in_ui.lock().expect("poisoned") = app.worked_in_task().map(str::to_string);
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
    harness.run_steps(3);
    assert_eq!(
        *worked_in.lock().expect("poisoned"),
        Some("fix-the-login-page-2222".to_string()),
        "the notes are the second task's, so its card is the one marked"
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
