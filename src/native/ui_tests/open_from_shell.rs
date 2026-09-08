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
