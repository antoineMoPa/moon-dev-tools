//! What a server changes or says about the whole file: formatting it, the errors it found, and
//! the code actions it offers.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use crate::native::ui_tests::{Fixture, app_for, click_at, press_key, settle, type_letter};

use super::RealTab;

/// ⌥⇧F in a file no language server serves says so rather than leaving the text as it was
/// without a word - which would read as the file already being formatted.
#[test]
fn alt_shift_f_in_a_file_no_server_serves_says_it_cannot_be_formatted() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-format-unserved");
    fixture.write("notes/plan.txt", "greet   is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "notes/plan.txt");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    click_at(&mut harness, first_line);
    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::ALT | egui::Modifiers::SHIFT,
    );

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text == "no language server serves notes/plan.txt, so it cannot be formatted"
            })
    };
    assert!(
        settle(&mut harness, told),
        "⌥⇧F should have said the file cannot be formatted, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// A real crate, a real rust-analyzer: ⌥⇧F lays the tab's text out the way rustfmt does, in
/// the buffer and unsaved.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn formatting_a_file_in_a_real_crate_lays_the_tab_out_the_way_rustfmt_does() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-format-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write("src/lib.rs", "pub fn   greet( )->u32{1}\n");
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let text = Arc::new(Mutex::new(String::new()));
    let text_in_ui = Arc::clone(&text);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.server_heard().status() == crate::api::LspStatus::Ready),
                Ordering::Relaxed,
            );
            if let Some(editor) = app.model.file_editors.values().next() {
                *text_in_ui.lock().expect("the text is not shared") =
                    editor.text_for_test().to_string();
            }
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    fn wait(harness: &mut Harness<'_>, patience: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    assert!(
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    click_at(&mut harness, first_line);
    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::ALT | egui::Modifiers::SHIFT,
    );

    let formatted = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        *text.lock().expect("the text is not shared") == "pub fn greet() -> u32 {\n    1\n}\n"
    });
    assert!(
        formatted,
        "the tab reads {:?}, and the window said {:?}",
        text.lock().expect("the text is not shared").clone(),
        said.lock().expect("the messages are not shared").clone()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("src/lib.rs")).expect("the file is there"),
        "pub fn   greet( )->u32{1}\n",
        "the format is the tab's to save"
    );
}

/// A real crate, a real rust-analyzer: a type error in a saved file comes back as a diagnostic
/// on the tab, and resting the pointer on a word brings back what the server says about it.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn a_real_crate_s_errors_reach_the_tab_and_a_resting_pointer_is_told_about_the_word() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const CHECKING_TAKES_AT_MOST: Duration = Duration::from_secs(90);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-diagnostics-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write("src/lib.rs", "pub fn greet() -> u32 {\n    \"one\"\n}\n");
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let found = Arc::new(Mutex::new(Vec::<String>::new()));
    let found_in_ui = Arc::clone(&found);
    let told = Arc::new(Mutex::new(None::<String>));
    let told_in_ui = Arc::clone(&told);

    let pub_keyword = egui_moon_editor::Word {
        text: "pub".to_string(),
        at: egui_moon_editor::TextPoint {
            offset: 0,
            line: 0,
            column: 0,
        },
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            if let Some(editor) = app.model.file_editors.values().next() {
                ready_in_ui.store(
                    editor.server_heard().status() == crate::api::LspStatus::Ready,
                    Ordering::Relaxed,
                );
                *found_in_ui.lock().expect("not shared") = editor.diagnostics_for_test();
                *told_in_ui.lock().expect("not shared") = editor
                    .hovering()
                    .showing(Some(&pub_keyword))
                    .map(str::to_string);
            }
        });

    fn wait(harness: &mut Harness<'_>, patience: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    assert!(
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );

    // A space at the end of the first line and a save, which is what sets `cargo check` off.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    type_letter(&mut harness, egui::Key::Space, " ");
    press_key(&mut harness, egui::Key::S, egui::Modifiers::COMMAND);

    let checked = wait(&mut harness, CHECKING_TAKES_AT_MOST, || {
        found
            .lock()
            .expect("not shared")
            .iter()
            .any(|message| message.contains("u32"))
    });
    println!("found {:?}", found.lock().expect("not shared"));
    assert!(
        checked,
        "the type error never reached the tab, saw {:?}",
        found.lock().expect("not shared").clone()
    );

    // The pointer rests on `pub`, the first word of the file.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(first_line));
    let answered = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        told.lock().expect("not shared").is_some()
    });
    println!("told {:?}", told.lock().expect("not shared"));
    assert!(answered, "resting on `pub` brought nothing back");
}

/// ⌘. in a file no language server serves says it has no code actions, rather than opening an
/// empty list or nothing at all.
#[test]
fn command_period_in_a_file_no_server_serves_says_it_has_no_code_actions() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-actions-unserved");
    fixture.write("notes/plan.txt", "greet is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "notes/plan.txt");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            *said_in_ui.lock().expect("not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::Period, egui::Modifiers::COMMAND);

    let told = || {
        said.lock().expect("not shared").iter().any(|text| {
            text == "no language server serves notes/plan.txt, so it has no code actions"
        })
    };
    assert!(
        settle(&mut harness, told),
        "⌘. should have said the file has no code actions, saw {:?}",
        said.lock().expect("not shared").clone()
    );
}

/// A real crate, a real rust-analyzer: ⌘. on a `let` lists what rust-analyzer offers there,
/// and picking "insert explicit type" puts the type into the tab.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn a_code_action_picked_from_the_palette_edits_the_tab() {
    let mut tab = RealTab::open(
        "file-actions-real",
        "pub fn five() -> i32 {\n    let x = 5;\n    x\n}\n",
    );
    tab.caret_to(1, "    let ".len());
    press_key(
        &mut tab.harness,
        egui::Key::Period,
        egui::Modifiers::COMMAND,
    );

    let rows = Arc::clone(&tab.palette_rows);
    let offered = tab.wait(Duration::from_secs(30), || {
        rows.lock()
            .expect("not shared")
            .iter()
            .any(|title| title.to_lowercase().contains("explicit type"))
    });
    println!("offered {:?}", rows.lock().expect("not shared"));
    assert!(
        offered,
        "rust-analyzer's explicit type action was not offered, saw {:?} and the window said {:?}",
        rows.lock().expect("not shared").clone(),
        tab.said()
    );

    // Narrowed down to that one row, and taken.
    for (key, letter) in [
        (egui::Key::E, "e"),
        (egui::Key::X, "x"),
        (egui::Key::P, "p"),
        (egui::Key::L, "l"),
        (egui::Key::I, "i"),
        (egui::Key::C, "c"),
        (egui::Key::I, "i"),
        (egui::Key::T, "t"),
    ] {
        type_letter(&mut tab.harness, key, letter);
    }
    press_key(&mut tab.harness, egui::Key::Enter, egui::Modifiers::NONE);

    let text = Arc::clone(&tab.text);
    let typed = tab.wait(Duration::from_secs(10), || {
        text.lock().expect("not shared").contains("let x: i32 = 5;")
    });
    assert!(
        typed,
        "the tab reads {:?}, and the window said {:?}",
        text.lock().expect("not shared").clone(),
        tab.said()
    );
}
