//! `moon open <file>` arriving in the window: the tab it opens.
//!
//! The socket a shell reaches the window on is `crate::instances`' own business and is
//! tested there. What is here is the other half: that a file handed to the window turns into
//! the tab it was asked for, named by its path inside the project.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_frames::PaneId;
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
        wait: false,
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

/// `moon edit` of a path nothing is at: the tab opens empty on it and nothing is written - a
/// tab closed unsaved leaves nothing behind - until the tab is saved, which creates the file.
#[test]
fn a_file_that_is_not_there_yet_is_created_by_saving_its_tab() {
    let fixture = Fixture::new("moon-open-new");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    let file = fixture
        .root
        .canonicalize()
        .expect("expected the repo to resolve")
        .join("src/notes.md");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let asked = Arc::new(Mutex::new(Some(OpenFileAsked {
        path: file.clone(),
        line: None,
        wait: false,
    })));
    let asked_in_ui = Arc::clone(&asked);
    let open = Arc::new(Mutex::new(None::<(PaneId, String, String)>));
    let open_in_ui = Arc::clone(&open);
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let save = Arc::new(AtomicBool::new(false));
    let save_in_ui = Arc::clone(&save);
    let errors = Arc::new(AtomicBool::new(false));
    let errors_in_ui = Arc::clone(&errors);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if let Some(asked) = asked_in_ui.lock().expect("poisoned").take() {
                app.asked_files.push_back(asked);
            }
            let tab = open_in_ui.lock().expect("poisoned").clone();
            if let Some((pane_id, session_id, _)) = tab {
                if let Some(text) = edit_in_ui.lock().expect("poisoned").take()
                    && let Some(editor) = app.model.file_editors.get_mut(&pane_id)
                {
                    editor.edit_for_test(&text);
                }
                if save_in_ui.swap(false, Ordering::Relaxed) {
                    app.save_file_pane(pane_id, &session_id);
                }
            }
            app.draw(ui);

            *open_in_ui.lock().expect("poisoned") =
                app.model
                    .layout
                    .panes()
                    .find_map(|(pane_id, pane)| match pane {
                        Pane::File {
                            session_id,
                            file_path,
                            ..
                        } => Some((pane_id, session_id.clone(), file_path.clone())),
                        _ => None,
                    });
            errors_in_ui.store(!app.model.toasts.is_empty(), Ordering::Relaxed);
        });

    let opened = settle(&mut harness, || open.lock().expect("poisoned").is_some());
    assert!(opened, "the new file a shell asked for never reached a tab");
    harness.run_steps(5);
    let (_, _, file_path) = open.lock().expect("poisoned").clone().expect("a tab");
    assert_eq!(file_path, "src/notes.md");
    assert!(!file.exists(), "opening a new file should not write it");
    assert!(
        !errors.load(Ordering::Relaxed),
        "opening a new file said something went wrong"
    );

    // Act: write in it, and save.
    *edit.lock().expect("poisoned") = Some("# Notes\n".to_string());
    harness.run_steps(2);
    save.store(true, Ordering::Relaxed);
    let created = settle(&mut harness, || file.exists());

    // Assert
    assert!(created, "saving the tab should have created the file");
    assert_eq!(
        std::fs::read_to_string(&file).expect("expected the file"),
        "# Notes\n"
    );
    assert!(
        !errors.load(Ordering::Relaxed),
        "saving the new file said something went wrong"
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
        wait: false,
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

/// `moon edit --wait`, as git runs it on a commit message: the window holds on to the tab it
/// opened for as long as the tab is open, and lets go of it once it is closed - which is when
/// the shell waiting on it is told it can stop.
#[test]
fn a_waited_on_tab_is_let_go_of_once_it_is_closed() {
    let fixture = Fixture::new("moon-edit-wait");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    // Where git writes the message it hands its editor, inside the repo's own `.git`.
    fixture.write(
        ".git/COMMIT_EDITMSG",
        "\n# Please enter the commit message\n",
    );
    let file = fixture
        .root
        .join(".git/COMMIT_EDITMSG")
        .canonicalize()
        .expect("expected the file to resolve");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let asked = Arc::new(Mutex::new(Some(OpenFileAsked {
        path: file,
        line: None,
        wait: true,
    })));
    let asked_in_ui = Arc::clone(&asked);
    let close = Arc::new(AtomicBool::new(false));
    let close_in_ui = Arc::clone(&close);
    // The tab, and how many tabs the window is waiting on.
    let seen = Arc::new(Mutex::new((None::<PaneId>, 0usize)));
    let read = Arc::new(Mutex::new(None::<String>));
    let read_in_ui = Arc::clone(&read);
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_ui(move |ui| {
            if let Some(asked) = asked_in_ui.lock().expect("poisoned").take() {
                app.asked_files.push_back(asked);
            }
            let tab = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(tab) = tab
                && close_in_ui.swap(false, Ordering::Relaxed)
            {
                app.close_pane(tab);
            }
            app.draw(ui);
            *seen_in_ui.lock().expect("poisoned") = (tab, app.waited_tabs.len());
            if let Some(editor) = tab.and_then(|tab| app.model.file_editors.get(&tab)) {
                *read_in_ui.lock().expect("poisoned") = editor.content_for_test();
            }
        });

    assert!(
        settle(&mut harness, || seen.lock().expect("poisoned").0.is_some()),
        "the file never reached a tab"
    );
    harness.run_steps(3);
    assert_eq!(
        seen.lock().expect("poisoned").1,
        1,
        "the window should be holding on to the open tab"
    );
    assert!(
        settle(&mut harness, || read.lock().expect("poisoned").as_deref()
            == Some("\n# Please enter the commit message\n")),
        "the tab should read the message git wrote, got {:?}",
        read.lock().expect("poisoned")
    );

    close.store(true, Ordering::Relaxed);
    assert!(
        settle(&mut harness, || *seen.lock().expect("poisoned")
            == (None, 0)),
        "closing the tab should have let go of it"
    );
}
