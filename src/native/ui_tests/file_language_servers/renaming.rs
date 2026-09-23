//! Renaming a name everywhere it is used, and finding those uses.

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

/// F2 in a file no language server serves says so, and the palette does not offer a rename
/// there at all. A rename is a direct request, and a key that does nothing reads as a broken
/// key - so it is answered, and names the file.
#[test]
fn f2_in_a_file_no_server_serves_says_nothing_in_it_can_be_renamed() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-rename-unserved");
    fixture.write("notes/plan.txt", "greet is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let offered_a_rename = Arc::new(AtomicBool::new(false));
    let offered_in_ui = Arc::clone(&offered_a_rename);
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
            if crate::native::palette::commands_for(&app)
                .iter()
                .any(|command| command.title == "rename symbol")
            {
                offered_in_ui.store(true, Ordering::Relaxed);
            }
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
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
    press_key(&mut harness, egui::Key::F2, egui::Modifiers::NONE);

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text.contains(
                    "no language server serves notes/plan.txt, so nothing in it can be renamed",
                )
            })
    };
    assert!(
        settle(&mut harness, told),
        "F2 should have said nothing in the file can be renamed, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert!(
        !offered_a_rename.load(Ordering::Relaxed),
        "the palette offered a rename in a file nothing serves"
    );
}

/// A real crate, a real rust-analyzer, and a rename across two files: the one test that proves
/// the whole of it works in one window. The file open in a tab takes the edit in its buffer and
/// is left unsaved; the file nobody has open is written.
///
/// `#[ignore]`d for the reason the completion test above is: it starts a language server and
/// waits for it to read a project.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn renaming_in_a_real_crate_edits_the_open_tab_and_writes_the_file_nobody_has_open() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-rename-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    // The name is on the first line, so the caret is put on it by clicking into the corner of
    // the text and stepping right - no dependence on how the text was laid out.
    fixture.write(
        "src/lib.rs",
        "pub fn greet() -> u32 {\n    1\n}\n\nmod other;\n",
    );
    fixture.write(
        "src/other.rs",
        "pub fn twice() -> u32 {\n    crate::greet() * 2\n}\n",
    );
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let naming = Arc::new(Mutex::new(None::<String>));
    let naming_in_ui = Arc::clone(&naming);
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

            // Ready as the tab has it, which is what a rename is started against: the tab
            // asks after its server now and then rather than every frame, so it hears a moment
            // after the server itself says so.
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.server_heard().status() == crate::api::LspStatus::Ready),
                Ordering::Relaxed,
            );
            let palette = &app.model.palette;
            *naming_in_ui.lock().expect("the palette is not shared") = (palette.open
                && palette.mode == crate::native::palette::PaletteMode::Rename)
                .then(|| palette.query.clone());
            if let Some(editor) = app.model.file_editors.values().next() {
                *text_in_ui.lock().expect("the text is not shared") =
                    editor.text_for_test().to_string();
            }
            // The whole log rather than the toasts: a toast fades long before a wait on a
            // server gives up, and the failure has to say what was said.
            let front_is_the_file = crate::native::renaming::front_tab_renames(&app);
            let step = app
                .model
                .renaming
                .as_ref()
                .map_or("no rename", |renaming| renaming.step_for_test());
            let mut logged: Vec<String> = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
            logged.push(format!(
                "[rename: {step}; front tab renames: {front_is_the_file}]"
            ));
            *said_in_ui.lock().expect("the messages are not shared") = logged;
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

    // Onto the `r` of `greet`, and F2.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::Home, egui::Modifiers::NONE);
    for _ in 0.."pub fn g".len() {
        press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    }
    press_key(&mut harness, egui::Key::F2, egui::Modifiers::NONE);

    let offered = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        naming.lock().expect("the palette is not shared").is_some()
    });
    assert!(
        offered,
        "the palette never opened on the name, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert_eq!(
        naming.lock().expect("the palette is not shared").clone(),
        Some("greet".to_string()),
        "the palette opens on the name the server gave"
    );

    // The name is selected, so what is typed replaces it.
    for (key, letter) in [
        (egui::Key::H, "h"),
        (egui::Key::E, "e"),
        (egui::Key::L, "l"),
        (egui::Key::L, "l"),
        (egui::Key::O, "o"),
    ] {
        type_letter(&mut harness, key, letter);
    }
    assert_eq!(
        naming.lock().expect("the palette is not shared").clone(),
        Some("hello".to_string())
    );
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    let other = fixture.root.join("src/other.rs");
    let renamed = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        text.lock()
            .expect("the text is not shared")
            .starts_with("pub fn hello() -> u32 {")
            && std::fs::read_to_string(&other).is_ok_and(|other| other.contains("crate::hello()"))
            // Said once the file nobody had open is written, which is a moment after it is.
            && said
                .lock()
                .expect("the messages are not shared")
                .iter()
                .any(|said| said.starts_with("renamed greet to hello: 2 places in 2 files"))
    });
    assert!(
        renamed,
        "the rename never landed: the tab reads {:?}, other.rs reads {:?}, and the window said {:?}",
        text.lock().expect("the text is not shared").clone(),
        std::fs::read_to_string(&other),
        said.lock().expect("the messages are not shared").clone()
    );
    // The open tab is edited and not saved: its file on disk still has the old name.
    assert!(
        std::fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("the file is there")
            .starts_with("pub fn greet()"),
        "the open tab's file should be left for the person to save"
    );
    assert!(
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|said| said.starts_with("renamed greet to hello: 2 places in 2 files")),
        "saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// Shift-F12 in a file no language server serves says so, and names what could not be found -
/// the same answer F2 gives, for the uses of a name.
#[test]
fn shift_f12_in_a_file_no_server_serves_says_the_uses_cannot_be_found() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-references-unserved");
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
    press_key(&mut harness, egui::Key::F12, egui::Modifiers::SHIFT);

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text == "no language server serves notes/plan.txt, so the uses of a name cannot be found"
            })
    };
    assert!(
        settle(&mut harness, told),
        "shift-F12 should have said the uses cannot be found, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// A real crate, a real rust-analyzer: shift-F12 on a name lists everywhere it is used in the
/// palette, each row reading what its line says - the declaration among them.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn finding_the_uses_of_a_name_in_a_real_crate_lists_them_in_the_palette() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-references-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn greet() -> u32 {\n    1\n}\n\nmod other;\n",
    );
    fixture.write(
        "src/other.rs",
        "pub fn twice() -> u32 {\n    crate::greet() * 2\n}\n",
    );
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let rows = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let rows_in_ui = Arc::clone(&rows);
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
            let palette = &app.model.palette;
            *rows_in_ui.lock().expect("the rows are not shared") = match &palette.places {
                Some(found) if palette.open => found
                    .places
                    .iter()
                    .map(|place| {
                        (
                            format!("{}:{}", place.file_path, place.line_number),
                            place.line_text.clone().unwrap_or_default(),
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            };
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
    press_key(&mut harness, egui::Key::Home, egui::Modifiers::NONE);
    for _ in 0.."pub fn g".len() {
        press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    }
    press_key(&mut harness, egui::Key::F12, egui::Modifiers::SHIFT);

    let listed = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        rows.lock().expect("the rows are not shared").len() == 2
    });
    let seen = rows.lock().expect("the rows are not shared").clone();
    println!("listed {seen:?}");
    assert!(
        listed,
        "expected the declaration and the one use, saw {seen:?} and the window said {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert!(
        seen.contains(&(
            "src/other.rs:2".to_string(),
            "    crate::greet() * 2".to_string()
        )),
        "the use in other.rs reads its line, saw {seen:?}"
    );
}
