//! A file kept encrypted with gpg: opened with its passphrase, and saved encrypted again.

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

use super::{Fixture, app_for, press_key};

/// What the test reads off the tab each frame.
#[derive(Default)]
struct Seen {
    pane_id: Option<PaneId>,
    content: Option<String>,
    refusal: Option<String>,
    dirty: bool,
}

fn type_text(harness: &mut Harness<'_>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_string()));
    harness.run_steps(2);
}

fn wait_for(harness: &mut Harness<'_>, seen: &Mutex<Seen>, until: impl Fn(&Seen) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !until(&seen.lock().expect("poisoned")) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(until(&seen.lock().expect("poisoned")), "timed out");
}

/// A wrong passphrase is refused with nothing shown, the right one shows the text, and a save
/// asks for a passphrase twice and writes the text encrypted with it - never in the clear.
#[test]
fn an_encrypted_file_opens_with_its_passphrase_and_saves_encrypted() {
    let fixture = Fixture::new("file-encrypted");
    fixture.write("README.md", "readme\n");
    fixture.commit("Add the readme");
    let encrypted = crate::gpg::encrypt_with_passphrase("the secret\n", "right horse").unwrap();
    fs::write(fixture.root.join("secrets.md.gpg"), encrypted).unwrap();

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let save = Arc::new(AtomicBool::new(false));
    let save_in_ui = Arc::clone(&save);
    let opened = Arc::new(AtomicBool::new(false));

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "secrets.md.gpg");
                opened.store(true, Ordering::Relaxed);
            }
            let pane_id = seen_in_ui.lock().expect("poisoned").pane_id;
            if let Some(text) = edit_in_ui.lock().expect("poisoned").take()
                && let Some(editor) = pane_id.and_then(|id| app.model.file_editors.get_mut(&id))
            {
                editor.edit_for_test(&text);
            }
            if save_in_ui.swap(false, Ordering::Relaxed)
                && let Some(id) = pane_id
            {
                let session_id = app.model.root_session_id.clone();
                app.save_file_pane(id, &session_id);
            }

            app.draw(ui);

            let mut seen = seen_in_ui.lock().expect("poisoned");
            seen.pane_id = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(editor) = seen.pane_id.and_then(|id| app.model.file_editors.get(&id)) {
                seen.content = editor.content_for_test();
                seen.refusal = editor.passphrase_refusal_for_test();
                seen.dirty = editor.is_dirty();
            }
        });

    wait_for(&mut harness, &seen, |seen| seen.pane_id.is_some());
    harness.run_steps(2);

    type_text(&mut harness, "wrong horse");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    wait_for(&mut harness, &seen, |seen| seen.refusal.is_some());
    assert_eq!(seen.lock().unwrap().content, None, "nothing is shown");

    type_text(&mut harness, "right horse");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    wait_for(&mut harness, &seen, |seen| seen.content.is_some());
    assert_eq!(
        seen.lock().unwrap().content.as_deref(),
        Some("the secret\n")
    );

    *edit.lock().unwrap() = Some("the new secret\n".to_string());
    harness.run_steps(2);
    assert!(seen.lock().unwrap().dirty);
    // Every save asks for the passphrase, twice: two that differ are refused.
    save.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    type_text(&mut harness, "new horse");
    press_key(&mut harness, egui::Key::Tab, egui::Modifiers::NONE);
    type_text(&mut harness, "new hrose");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    wait_for(&mut harness, &seen, |seen| seen.refusal.is_some());
    assert!(seen.lock().unwrap().dirty, "nothing was written");

    type_text(&mut harness, "new horse");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    wait_for(&mut harness, &seen, |seen| !seen.dirty);

    let on_disk = fs::read(fixture.root.join("secrets.md.gpg")).unwrap();
    assert!(
        !String::from_utf8_lossy(&on_disk).contains("secret"),
        "the file is not written in the clear"
    );
    assert_eq!(
        crate::gpg::decrypt(&on_disk, "new horse").unwrap(),
        "the new secret\n"
    );
}
