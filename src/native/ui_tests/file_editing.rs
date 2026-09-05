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
