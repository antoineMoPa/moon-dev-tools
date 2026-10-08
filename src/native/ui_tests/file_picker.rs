//! The window's own file picker: a file picked with the keyboard, and a file picked outside
//! the repo opened and saved.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::native::{palette::CommandAction, panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, press_key, settle, type_letter};

/// File › Open in a window that is its machine's desktop, which has no dialog of the OS's to
/// ask: the picker opens on the repo, what is typed narrows it to a folder, Enter goes in, an
/// arrow and Enter pick a file there, and the file opens in a tab.
#[test]
fn a_file_is_picked_with_the_keyboard_and_opens_in_a_tab() {
    let fixture = Fixture::new("file-picker-keyboard");
    fixture.write("README.md", "# fixture\n");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.write("src/main.rs", "fn main() {}\n");
    fixture.commit("Add the program");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.manages_the_session = true;
    let asked = Arc::new(AtomicBool::new(false));
    let asked_in_ui = Arc::clone(&asked);
    // The picker's rows while it is up, and the file in a tab once there is one.
    let seen = Arc::new(Mutex::new((None::<Vec<String>>, None::<String>)));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_ui(move |ui| {
            if !asked_in_ui.load(Ordering::Relaxed) && app.repo_root().is_some() {
                asked_in_ui.store(true, Ordering::Relaxed);
                app.pending_action = Some(CommandAction::OpenFile);
            }
            app.draw(ui);
            *seen_in_ui.lock().expect("poisoned") = (
                app.model
                    .file_picker
                    .as_ref()
                    .map(|picker| picker.rows_for_test()),
                app.model.layout.panes().find_map(|(_, pane)| match pane {
                    Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                }),
            );
        });
    let rows = |seen: &Arc<Mutex<(Option<Vec<String>>, Option<String>)>>| {
        seen.lock().expect("poisoned").0.clone().unwrap_or_default()
    };

    assert!(
        settle(&mut harness, || rows(&seen)
            == ["..", "src/", "README.md", ".git/"]),
        "the picker should list the repo, got {:?}",
        rows(&seen)
    );

    // Act: narrow to the folder, go in, and take the second file there.
    type_letter(&mut harness, egui::Key::S, "s");
    type_letter(&mut harness, egui::Key::R, "r");
    assert_eq!(rows(&seen), ["src/"]);
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || rows(&seen) == ["..", "lib.rs", "main.rs"]),
        "Enter on a folder should list it, got {:?}",
        rows(&seen)
    );
    press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    // Assert
    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").1.is_some()),
        "the file picked never reached a tab"
    );
    let (picker, tab) = seen.lock().expect("poisoned").clone();
    assert_eq!(tab.as_deref(), Some("src/main.rs"));
    assert_eq!(picker, None, "a pick should put the picker away");
}

/// What the window reads back of a tab on a file outside the repo.
#[derive(Clone, Default)]
struct Outside {
    root_session_id: String,
    /// The tab on the file once its text has arrived: its pane, session and path.
    tab: Option<(PaneId, String, String)>,
    toasts: Vec<String>,
}

/// A file in no repo at all, the way `/etc/hostname` is: picked, it opens through a session
/// on its own folder, a save writes it, and a save the account may not make says why.
#[test]
fn a_file_outside_the_repo_opens_and_saves_and_says_when_it_may_not() {
    let fixture = Fixture::new("file-picker-outside");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    let folder = std::env::temp_dir().join(format!("moonreview-ui-{}-etc", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("expected a folder");
    let folder = folder
        .canonicalize()
        .expect("expected the folder to resolve");
    let file = folder.join("hostname");
    std::fs::write(&file, "moonos\n").expect("expected a file");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let picked = Arc::new(Mutex::new(Some(file.clone())));
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let seen = Arc::new(Mutex::new(Outside::default()));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_ui(move |ui| {
            if app.repo_root().is_some()
                && let Some(picked) = picked.lock().expect("poisoned").take()
            {
                app.open_picked_file(picked);
            }
            let tab = seen_in_ui.lock().expect("poisoned").tab.clone();
            if let Some((pane_id, session_id, _)) = tab
                && let Some(text) = edit_in_ui.lock().expect("poisoned").take()
            {
                app.model
                    .file_editors
                    .get_mut(&pane_id)
                    .expect("the tab has an editor")
                    .edit_for_test(&text);
                app.save_file_pane(pane_id, &session_id);
            }
            app.draw(ui);
            *seen_in_ui.lock().expect("poisoned") = Outside {
                root_session_id: app.model.root_session_id.clone(),
                tab: app
                    .model
                    .layout
                    .panes()
                    .find_map(|(pane_id, pane)| match pane {
                        Pane::File {
                            session_id,
                            file_path,
                            ..
                        } if app
                            .model
                            .file_editors
                            .get(&pane_id)
                            .is_some_and(|editor| editor.content_for_test().is_some()) =>
                        {
                            Some((pane_id, session_id.clone(), file_path.clone()))
                        }
                        _ => None,
                    }),
                toasts: app
                    .model
                    .toasts
                    .iter()
                    .map(|toast| toast.text.clone())
                    .collect(),
            };
        });

    assert!(
        settle(&mut harness, || seen
            .lock()
            .expect("poisoned")
            .tab
            .is_some()),
        "the file outside the repo never loaded in a tab, the window said {:?}",
        seen.lock().expect("poisoned").toasts
    );
    let (_, session_id, file_path) = seen.lock().expect("poisoned").tab.clone().expect("a tab");
    assert_eq!(file_path, "hostname");
    assert_ne!(session_id, seen.lock().expect("poisoned").root_session_id);

    // Act: a save the account may make.
    *edit.lock().expect("poisoned") = Some("moon\n".to_string());
    assert!(
        settle(&mut harness, || std::fs::read_to_string(&file)
            .is_ok_and(|text| text == "moon\n")),
        "saving the tab should have written the file"
    );
    assert_eq!(seen.lock().expect("poisoned").toasts, Vec::<String>::new());

    // Act: and one it may not.
    let mut read_only = std::fs::metadata(&file)
        .expect("expected the file")
        .permissions();
    read_only.set_readonly(true);
    std::fs::set_permissions(&file, read_only).expect("expected to change the file's mode");
    harness.run_steps(3);
    *edit.lock().expect("poisoned") = Some("not allowed\n".to_string());

    // Assert
    assert!(
        settle(&mut harness, || !seen
            .lock()
            .expect("poisoned")
            .toasts
            .is_empty()),
        "a save that was refused should say so"
    );
    let said = seen.lock().expect("poisoned").toasts.join("\n");
    assert!(
        said.contains("could not save") && said.to_lowercase().contains("permission denied"),
        "the refusal should say why, got {said:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("expected the file"),
        "moon\n"
    );

    let _ = std::fs::remove_dir_all(&folder);
}
