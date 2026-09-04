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

/// The deploy list a task wrote sits under everything else on its card: a row per repo, in the
/// order they are to be committed, and a click on one opens that repo's review.
///
/// Whether a row is still pending is the repo having changed files rather than anything written
/// down, so the list ticks itself off as the repos are committed - here the fixture repo has
/// changes and reads as pending.
#[test]
fn a_tasks_deploy_list_is_drawn_under_its_card() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("board-review-requests");
    fixture.write(
        ".moontasks/deploy-the-thing-1111/metadata.json",
        "{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        ".moontasks/deploy-the-thing-1111/request_for_review.txt",
        ". // chore: take the module forward\n\
         vendor/turbocharger/#moontask/show-moon-icons-in-bootscreens-59c3e24c // chore: on a branch\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let repo_name = fixture
        .root
        .file_name()
        .expect("the fixture repo has a name")
        .to_string_lossy()
        .to_string();
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // The reviews the window has open, and which pane is in front, watched from out here: the
    // row's job is to put you in the review of the repo it names.
    let reviews = Arc::new(Mutex::new(Vec::<String>::new()));
    let reviews_in_ui = Arc::clone(&reviews);
    let in_front = Arc::new(Mutex::new(None::<crate::native::panes::PaneKind>));
    let in_front_in_ui = Arc::clone(&in_front);

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
            *reviews_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| pane.kind() == crate::native::panes::PaneKind::Review)
                .map(|(_, pane)| pane.tab_title())
                .collect();
            *in_front_in_ui.lock().expect("poisoned") = app
                .front_pane
                .and_then(|id| app.model.layout.pane(id))
                .map(|pane| pane.kind());
        });

    // The row waits on two answers from worker threads - the board, and the requests read off
    // its task folders - so it is waited for rather than looked for on a fixed frame.
    let row = format!("pending {repo_name} review");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && harness.query_by_label(row.as_str()).is_none() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        harness.query_by_label(row.as_str()).is_some(),
        "the card should list the repo the task asked to have reviewed"
    );

    // The line names the repo the board is in, whose review the window is already on, so the
    // click brings that review forward rather than opening a second one of the same repo.
    harness.get_by_label(row.as_str()).click();
    assert!(
        settle(&mut harness, || *in_front.lock().expect("poisoned")
            == Some(crate::native::panes::PaneKind::Review)),
        "clicking the row should put you in the review of the repo it names"
    );
    assert_eq!(
        reviews.lock().expect("poisoned").len(),
        1,
        "the repo already had a review open, so no second one should have been made"
    );
}

/// A card in the column that finishes a task has nothing left pending on it.
///
/// The repo still has changes and no line was crossed off, so under any other column both rows
/// would read as pending. Finishing the card is the person saying the work is behind them, and it
/// finishes what the card was asking for all at once - without writing a word to the file, so a
/// card dragged back out asks for it again.
#[test]
fn a_finished_cards_deploy_list_reads_as_reviewed() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("board-review-requests-finished");
    fixture.write(
        ".moontasks/deploy-the-thing-1111/metadata.json",
        &format!(
            "{{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"{}\",\n  \
             \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n",
            crate::moontasks::store::CLOSES_REVIEWS_IN
        ),
    );
    fixture.write(
        ".moontasks/deploy-the-thing-1111/request_for_review.txt",
        ". // chore: take the module forward\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let repo_name = fixture
        .root
        .file_name()
        .expect("the fixture repo has a name")
        .to_string_lossy()
        .to_string();
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

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
        });

    // Waited for the same way the pending row is: the board and the requests read off its task
    // folders are both worker threads, and the row is drawn once both have answered.
    let reviewed = format!("{repo_name} reviewed");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && harness.query_by_label(reviewed.as_str()).is_none() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        harness.query_by_label(reviewed.as_str()).is_some(),
        "a finished card's row should read as reviewed"
    );
    assert!(
        harness
            .query_by_label(format!("pending {repo_name} review").as_str())
            .is_none(),
        "and not as pending, though the repo still has changes to commit"
    );
    assert_eq!(
        std::fs::read_to_string(
            fixture
                .root
                .join(".moontasks/deploy-the-thing-1111/request_for_review.txt"),
        )
        .expect("the file should still be there"),
        ". // chore: take the module forward\n",
        "nothing is written to the file, so moving the card back asks for it again"
    );
}

/// A card's notes are its description: their first lines sit under the title, and a click
/// opens the task's own pane with the keyboard already in its notes box - so the words go
/// where the click was aimed without a file being opened beside the board. A task with none
/// offers `[add notes]` instead, which is the same click, and what is typed there is what
/// makes `notes.md` real.
#[test]
fn a_cards_notes_open_the_task_ready_to_write() {
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
    // The task whose pane the clicks end up opening, watched for from out here: the app is the
    // closure's from the moment the harness is built.
    let task_pane: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let task_pane_in_ui = Arc::clone(&task_pane);
    // The files open in the window, which the clicks must not add to any more: the notes are
    // written on the task's pane rather than in a file beside it.
    let file_panes = Arc::new(Mutex::new(Vec::<String>::new()));
    let file_panes_in_ui = Arc::clone(&file_panes);
    // The task the window is being worked in: a task's pane is one of its tabs, so the board
    // marks its card while that pane is in front.
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
            *worked_in_in_ui.lock().expect("poisoned") = super::marked_task(&app);
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
            if let Ok(mut open) = task_pane_in_ui.lock() {
                *open = app.model.layout.panes().find_map(|(_, pane)| match pane {
                    Pane::Start { task_id, .. } => Some(task_id.clone()),
                    _ => None,
                });
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
    let pane_open = || task_pane.lock().expect("poisoned").clone();
    let files_open = || {
        file_panes
            .lock()
            .map(|panes| panes.clone())
            .unwrap_or_default()
    };

    harness.get_by_label_contains("Ship it by Friday").click();
    assert!(
        settle(&mut harness, || pane_open().as_deref()
            == Some("write-the-parser-1111")),
        "clicking the description should have opened that task's pane, got {:?}",
        pane_open()
    );
    // The keyboard is in the notes box already, with the caret past what is written there: the
    // click was someone reaching for those words, so the next ones land after them.
    harness.run_steps(2);
    type_letter(&mut harness, egui::Key::A, "And test it.");
    let notes = fixture.root.join(".moontasks/write-the-parser-1111/notes.md");
    assert!(
        settle(&mut harness, || std::fs::read_to_string(&notes)
            .is_ok_and(|written| written == "Ship it by Friday, working top down.\nAnd test it.")),
        "the typing should have gone into the notes box, saw {:?}",
        std::fs::read_to_string(&notes)
    );

    // The other card, which has no notes to show and offers to start them instead: the same
    // click, on a task whose `notes.md` is not there yet.
    harness.get_by_label("[add notes]").click();
    assert!(
        settle(&mut harness, || pane_open().as_deref()
            == Some("fix-the-login-page-2222")),
        "[add notes] should have opened the other task's pane, got {:?}",
        pane_open()
    );
    harness.run_steps(2);
    type_letter(&mut harness, egui::Key::S, "Start with the session cookie");
    let started = fixture
        .root
        .join(".moontasks/fix-the-login-page-2222/notes.md");
    assert!(
        settle(&mut harness, || std::fs::read_to_string(&started)
            .is_ok_and(|written| written.contains("Start with the session cookie"))),
        "writing the notes is what makes the file real, saw {:?}",
        std::fs::read_to_string(&started)
    );

    // And neither click opened a file beside the board: the notes are written on the pane.
    assert!(
        files_open().is_empty(),
        "the notes should not have opened as files, got {:?}",
        files_open()
    );
    assert_eq!(
        *worked_in.lock().expect("poisoned"),
        Some("fix-the-login-page-2222".to_string()),
        "the pane is the second task's, so its card is the one marked"
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
