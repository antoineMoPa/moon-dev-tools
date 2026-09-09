//! `moon open <file>` arriving in the window: the tab it opens.
//!
//! The socket a shell reaches the window on is `crate::instances`' own business and is
//! tested there. What is here is the other half: that a file handed to the window turns into
//! the tab it was asked for, named by its path inside the project.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::{
    instances::window::OpenFileAsked,
    native::{panes::Pane, theme::ThemeMode},
};

use super::{Fixture, app_for, settle};

#[test]
fn a_file_a_shell_asked_for_opens_in_a_tab() {
    let fixture = Fixture::new("moon-open");
    fixture.write("src/lib.rs", "pub fn one() {}\npub fn two() {}\n");
    fixture.commit("Add the library");
    // `moon` resolves the path against the directory the shell is in before handing it over,
    // so what arrives is absolute and resolved, as the window's own project path is.
    let file = fixture
        .root
        .join("src/lib.rs")
        .canonicalize()
        .expect("expected the file to resolve");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let asked = Arc::new(Mutex::new(Some(OpenFileAsked {
        path: file,
        line: Some(2),
    })));
    let asked_in_ui = Arc::clone(&asked);
    let open = Arc::new(Mutex::new(None::<String>));
    let open_in_ui = Arc::clone(&open);
    let errors = Arc::new(AtomicBool::new(false));
    let errors_in_ui = Arc::clone(&errors);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            // Handed over on the first frame, before the project has finished opening: a
            // shell can ask at any moment, and the ask waits for the project rather than
            // being refused because it arrived early.
            if let Some(asked) = asked_in_ui.lock().expect("poisoned").take() {
                app.asked_files.push_back(asked);
            }
            app.draw(ui);

            *open_in_ui.lock().expect("poisoned") =
                app.model.layout.panes().find_map(|(_, pane)| match pane {
                    Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                });
            errors_in_ui.store(!app.model.toasts.is_empty(), Ordering::Relaxed);
        });

    let opened = settle(&mut harness, || open.lock().expect("poisoned").is_some());

    assert!(opened, "the file a shell asked for never reached a tab");
    assert_eq!(
        open.lock().expect("poisoned").clone(),
        Some("src/lib.rs".to_string())
    );
    assert!(
        !errors.load(Ordering::Relaxed),
        "opening it said something went wrong"
    );
}

/// A file of a project this window is not on: no window was open on it, so the shell handed
/// it here and the window opens a session on that project to put it in a tab of.
#[test]
fn a_file_of_another_project_opens_in_a_session_of_its_own() {
    let window_on = Fixture::new("moon-open-here");
    window_on.write("src/lib.rs", "pub fn here() {}\n");
    window_on.commit("Add the library");

    let asked_for = Fixture::new("moon-open-elsewhere");
    asked_for.write("src/other.rs", "pub fn elsewhere() {}\n");
    asked_for.commit("Add the other library");
    let file = asked_for
        .root
        .join("src/other.rs")
        .canonicalize()
        .expect("expected the file to resolve");

    let mut app = app_for(&window_on.root, ThemeMode::Dark);
    let root_session_id = app.model.root_session_id.clone();
    let asked = Arc::new(Mutex::new(Some(OpenFileAsked {
        path: file,
        line: None,
    })));
    let asked_in_ui = Arc::clone(&asked);
    let open = Arc::new(Mutex::new(None::<(String, String)>));
    let open_in_ui = Arc::clone(&open);
    let errors = Arc::new(AtomicBool::new(false));
    let errors_in_ui = Arc::clone(&errors);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if let Some(asked) = asked_in_ui.lock().expect("poisoned").take() {
                app.asked_files.push_back(asked);
            }
            app.draw(ui);

            *open_in_ui.lock().expect("poisoned") =
                app.model.layout.panes().find_map(|(_, pane)| match pane {
                    Pane::File {
                        session_id,
                        file_path,
                        ..
                    } => Some((session_id.clone(), file_path.clone())),
                    _ => None,
                });
            errors_in_ui.store(!app.model.toasts.is_empty(), Ordering::Relaxed);
        });

    let opened = settle(&mut harness, || open.lock().expect("poisoned").is_some());

    assert!(opened, "the file a shell asked for never reached a tab");
    let (session_id, file_path) = open.lock().expect("poisoned").clone().expect("a tab");
    // Named by its path inside its own project, and read through a session on that project
    // rather than through the one the window was opened on.
    assert_eq!(file_path, "src/other.rs");
    assert_ne!(session_id, root_session_id);
    assert!(
        !errors.load(Ordering::Relaxed),
        "opening it said something went wrong"
    );
}
