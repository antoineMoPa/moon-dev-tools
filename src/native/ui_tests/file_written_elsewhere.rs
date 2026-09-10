//! A file tab whose file another program wrote while it was open - an agent, a formatter, a
//! checkout.
//!
//! With nothing typed into the tab, it shows the file as it now is. With edits of its own it
//! keeps them, says the file changed on disk, and offers `[reload]` beside `[save]`: the one
//! takes the other program's version, the other writes the edits over it. `[reload]` asks
//! before it throws the edits away.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::{app::App, panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, settle};

/// What the test reads off the file tab after every frame.
#[derive(Default)]
struct Seen {
    loaded: bool,
    dirty: bool,
    written_elsewhere: bool,
    text: String,
}

/// A window with `src/lib.rs` open in a tab, the text typed into it whenever `edit` holds
/// some, and what the tab holds copied into `seen` after every frame.
fn harness_on_the_file(
    mut app: App,
    edit: Arc<Mutex<Option<String>>>,
    seen: Arc<Mutex<Seen>>,
) -> Harness<'static> {
    let opened = Arc::new(AtomicBool::new(false));
    Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened.store(true, Ordering::Relaxed);
            }
            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(text) = edit.lock().expect("poisoned").take()
                && let Some(editor) = open_pane.and_then(|id| app.model.file_editors.get_mut(&id))
            {
                editor.edit_for_test(&text);
            }

            app.draw(ui);

            if let Some(id) = open_pane
                && let Some(editor) = app.model.file_editors.get(&id)
            {
                *seen.lock().expect("poisoned") = Seen {
                    loaded: editor.content_for_test().is_some(),
                    dirty: app.file_pane_is_dirty(id),
                    written_elsewhere: editor.is_written_elsewhere_for_test(),
                    text: editor.text_for_test().to_string(),
                };
            }
        })
}

/// Nothing typed into the tab, so nothing is lost by showing the file as it now is - which is
/// what happens, without being asked.
#[test]
fn a_clean_file_tab_shows_the_file_as_another_program_wrote_it() {
    let fixture = Fixture::new("file-written-elsewhere-clean");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let edit = Arc::new(Mutex::new(None));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut harness = harness_on_the_file(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::clone(&edit),
        Arc::clone(&seen),
    );
    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").loaded),
        "the file never loaded"
    );

    fixture.write("src/lib.rs", "pub fn two() {}\n");
    let caught_up = settle(&mut harness, || {
        seen.lock().expect("poisoned").text == "pub fn two() {}\n"
    });
    assert!(
        caught_up,
        "the tab should show what was written to the file, still shows {:?}",
        seen.lock().expect("poisoned").text
    );
    let seen = seen.lock().expect("poisoned");
    assert!(!seen.dirty, "the tab holds what is on disk, so it is clean");
    assert!(
        !seen.written_elsewhere,
        "nothing was held back, so there is nothing to announce"
    );
    drop(seen);
    assert!(harness.query_by_label("[reload]").is_none());
}

/// Edits in the tab stay put when the file is written under them: the header says the file
/// changed on disk, and `[reload]` takes two presses to throw the edits away for it.
#[test]
fn a_file_written_under_unsaved_edits_keeps_them_until_reload_is_pressed_twice() {
    let fixture = Fixture::new("file-written-elsewhere-edited");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let edit = Arc::new(Mutex::new(None));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut harness = harness_on_the_file(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::clone(&edit),
        Arc::clone(&seen),
    );
    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").loaded),
        "the file never loaded"
    );

    *edit.lock().expect("poisoned") = Some("pub fn mine() {}\n".to_string());
    harness.run_steps(2);
    fixture.write("src/lib.rs", "pub fn theirs() {}\n");
    assert!(
        settle(&mut harness, || seen
            .lock()
            .expect("poisoned")
            .written_elsewhere),
        "the tab never noticed the file was written"
    );
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").text,
        "pub fn mine() {}\n",
        "the edits should still be on screen"
    );
    assert!(
        harness.query_by_label("changed on disk").is_some(),
        "the header should say the file changed on disk"
    );

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-pane-written-elsewhere");

    // The first press only asks.
    harness.get_by_label("[reload]").click();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").text,
        "pub fn mine() {}\n",
        "one press of [reload] should not have dropped the edits"
    );
    assert!(
        harness.query_by_label("[reload · discard edits]").is_some(),
        "the button should ask for the second press"
    );

    harness.get_by_label("[reload · discard edits]").click();
    harness.run_steps(2);
    let seen = seen.lock().expect("poisoned");
    assert_eq!(seen.text, "pub fn theirs() {}\n");
    assert!(!seen.dirty, "the tab holds what is on disk, so it is clean");
    assert!(!seen.written_elsewhere);
    drop(seen);
    assert!(harness.query_by_label("changed on disk").is_none());
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn theirs() {}\n",
        "reloading writes nothing"
    );
}
