//! Editing the text of a file tab and writing it back to the working tree.
//!
//! Both halves of the same thing: that what was typed reaches the file on disk, and that the
//! tab says it has unsaved edits until it does. The two ways of asking for the write - the
//! pane's own `[save]` button and cmd+s - are here together because as far as the person
//! doing it is concerned they are one feature.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, press_key, settle};

/// Editing a file tab writes the file back, and the tab says so until it does.
#[test]
fn editing_a_file_tab_saves_it_to_the_working_tree() {
    let fixture = Fixture::new("file-save");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let pane_id = Arc::new(Mutex::new(None::<PaneId>));
    let pane_in_ui = Arc::clone(&pane_id);
    let dirty = Arc::new(AtomicBool::new(false));
    let dirty_in_ui = Arc::clone(&dirty);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let save = Arc::new(AtomicBool::new(false));
    let save_in_ui = Arc::clone(&save);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

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
            if let Some(text) = edit_in_ui.lock().expect("poisoned").take()
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
                && let Some(editor) = app.model.file_editors.get_mut(&id)
            {
                editor.edit_for_test(&text);
            }
            if save_in_ui.swap(false, Ordering::Relaxed)
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
            {
                let session_id = app.model.root_session_id.clone();
                app.save_file_pane(id, &session_id);
            }

            app.draw(ui);

            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(id) = open_pane {
                dirty_in_ui.store(app.file_pane_is_dirty(id), Ordering::Relaxed);
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            *pane_in_ui.lock().expect("poisoned") = open_pane;
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !loaded.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(loaded.load(Ordering::Relaxed), "the file never loaded");
    assert!(
        !dirty.load(Ordering::Relaxed),
        "a freshly opened file is clean"
    );

    *edit.lock().expect("poisoned") = Some("pub fn two() {}\n".to_string());
    harness.run_steps(2);
    assert!(dirty.load(Ordering::Relaxed), "an edit should mark the tab");
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn one() {}\n",
        "nothing should reach the file until it is saved"
    );

    // The tab carries a dot for as long as the edit is not on disk.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-tab-unsaved");

    save.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && dirty.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        !dirty.load(Ordering::Relaxed),
        "saving should clear the mark"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn two() {}\n",
        "the edit should be on disk"
    );
}

/// The same as far as the user is concerned: type into the file, then press the pane's own
/// [save] button and cmd+s, which is how the edit actually gets asked for.
#[test]
fn the_save_button_and_the_chord_write_the_file() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-save-button");
    fixture.write("src/lib.rs", "one\n");
    fixture.commit("Add the library");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);

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
            app.draw(ui);
            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(id) = open_pane {
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    // Type at the end of the text, the way clicking into the file and typing does.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let text = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
    text.click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::X, "x");

    assert!(
        harness.query_by_label("[save]").is_some(),
        "an edited file should offer [save]"
    );

    // cmd+s first, with the keyboard still in the text where typing left it.
    press_key(&mut harness, egui::Key::S, egui::Modifiers::COMMAND);
    let saved_by_chord = settle(&mut harness, || {
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read") != "one\n"
    });
    assert!(
        saved_by_chord,
        "cmd+s should have written the file, saw {:?}",
        fs::read_to_string(fixture.root.join("src/lib.rs"))
    );

    // Then the button, on a second edit.
    let text = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
    text.click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::Y, "y");
    harness.get_by_label("[save]").click();
    let written = settle(&mut harness, || {
        fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("failed to read")
            .contains('y')
    });
    assert!(
        written,
        "clicking [save] should have written the file, saw {:?}",
        fs::read_to_string(fixture.root.join("src/lib.rs"))
    );
}

/// Tab indents by what the repo's `.moonreview.json` asks for, rather than by the tab
/// character egui's text area would type. The whole way through: the file is read as the
/// review opens, carried on the model, and handed to the editor as it is drawn.
#[test]
fn pressing_tab_indents_the_way_the_repo_asks() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-tab-indent");
    fixture.write("src/lib.rs", "one\n");
    fixture.write(".moonreview.json", r#"{ "indent": { "spaces": 2 } }"#);
    fixture.commit("Add the library");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let text = Arc::new(Mutex::new(String::new()));
    let text_in_ui = Arc::clone(&text);

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
            app.draw(ui);
            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(editor) = open_pane.and_then(|id| app.model.file_editors.get(&id)) {
                loaded_in_ui.store(editor.content_for_test().is_some(), Ordering::Relaxed);
                *text_in_ui.lock().expect("poisoned") = editor.text_for_test().to_string();
            }
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    // Into the text, which leaves the caret at the end of it, and then a tab.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::Tab, egui::Modifiers::NONE);
    harness.run_steps(2);

    assert_eq!(
        text.lock().expect("poisoned").as_str(),
        "one\n  ",
        "a repo asking for two spaces should have got two spaces, not a tab character"
    );
}

/// A file the jump to a definition landed on outside the repo opens read-only: the header says
/// where it is rather than letting it look like every other file tab, and no `[save]` is
/// offered even once it has been typed into.
///
/// The window is built over a state this test holds, so that the one thing that makes such a
/// file readable at all can be arranged the way a language server arranges it - by naming the
/// file as where a definition is. Everything after that is the pane doing what it does with
/// what came back.
#[test]
fn a_file_outside_the_repo_opens_read_only_and_offers_no_save() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-outside-the-repo");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    // A dependency's source, next to the repo rather than in it - which is where a jump into
    // `~/.cargo` lands.
    let dependency = fixture
        .root
        .with_file_name("registry")
        .join("dep/src/lib.rs");
    fs::create_dir_all(dependency.parent().expect("a path has a parent"))
        .expect("failed to create the dependency directory");
    fs::write(&dependency, "pub fn dep() {}\n").expect("failed to write the dependency");
    let dependency = dependency.display().to_string();

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let open = crate::api::OpenSessionRequest {
        repo_path: fixture.root.display().to_string(),
        diff_target: None,
        active_commit: None,
    };
    let session_id = crate::service::open_session(
        &state,
        crate::api::OpenSessionRequest {
            repo_path: open.repo_path.clone(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("failed to open the session")
    .session_id;
    // What a language server answered with, which is the only thing that makes a file out
    // there readable - see [`crate::lsp::FilesNamedOutsideTheRepo`].
    crate::lsp::remember_files_named(
        &state,
        &session_id,
        &[crate::api::LspLocation {
            file_path: dependency.clone(),
            line_number: 1,
        }],
    )
    .expect("failed to record what the server named");

    let mut app = crate::native::app::App::new(
        egui::Context::default(),
        crate::native::Launch {
            backend: Arc::new(crate::backend::local::LocalBackend::new(state)),
            open: Some(open),
            frame: crate::cli::Frame::Review,
        },
    );
    app.set_theme(ThemeMode::Dark);

    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let read_only = Arc::new(AtomicBool::new(false));
    let read_only_in_ui = Arc::clone(&read_only);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let on_screen = Arc::new(Mutex::new(None::<String>));
    let on_screen_in_ui = Arc::clone(&on_screen);
    let for_pane = dependency.clone();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, &for_pane);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            if let Some((id, _)) = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                && let Some(editor) = app.model.file_editors.get(&id)
            {
                loaded_in_ui.store(editor.content_for_test().is_some(), Ordering::Relaxed);
                read_only_in_ui.store(editor.is_outside_the_repo_for_test(), Ordering::Relaxed);
                *on_screen_in_ui.lock().expect("poisoned") =
                    Some(editor.text_for_test().to_string());
            }
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the dependency never loaded"
    );
    harness.run_steps(2);

    assert_eq!(
        on_screen.lock().expect("poisoned").clone(),
        Some("pub fn dep() {}\n".to_string()),
        "the pane should be showing the file the server named"
    );
    assert!(
        read_only.load(Ordering::Relaxed),
        "a file outside the repo is read-only"
    );
    // The header note, in place of the save it does not offer.
    assert!(
        harness
            .query_by_label("outside the repo · read-only")
            .is_some(),
        "the pane should say the file is not in the repo"
    );
    assert!(
        harness.query_by_label("[save]").is_none(),
        "a file outside the repo is not saved from here"
    );

    // And still not, once it has been typed into: the text is editable, the file is not.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .click();
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::X, "x");

    assert_ne!(
        on_screen.lock().expect("poisoned").clone(),
        Some("pub fn dep() {}\n".to_string()),
        "the typing should have reached the text on screen"
    );
    assert!(
        harness.query_by_label("[save]").is_none(),
        "an edited file outside the repo still offers no way to write it"
    );
    assert_eq!(
        fs::read_to_string(&dependency).expect("failed to read the dependency back"),
        "pub fn dep() {}\n",
        "and nothing may have reached the dependency on disk"
    );
}
