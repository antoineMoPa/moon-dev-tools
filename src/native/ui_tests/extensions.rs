//! An extension's pane in the real window: opened, walked with the keyboard or the pointer, and
//! a file of the project opened out of it into a tab.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui::{Key, Modifiers};
use egui_kittest::{Harness, kittest::Queryable};

use crate::native::{
    model::Stage,
    panes::{OpenPaneRequest, Pane},
    theme::ThemeMode,
};

use super::{Fixture, app_for, press_key};

/// How long the pane has to list the project, which is a script starting on a thread of its
/// own and running git.
const PATIENCE: Duration = Duration::from_secs(20);

/// Frames drawn between the listing arriving and the snapshot - see where it is used.
const SETTLING_FRAMES: usize = 5;

/// The window on a project with a folder and a file at its top, with the files extension open
/// and listing it. The flag is raised once `notes.txt` is open in a tab.
fn files_pane(name: &str) -> (Harness<'static>, Arc<AtomicBool>, Fixture) {
    let fixture = Fixture::new(name);
    fixture.write("docs/guide.md", "# guide\n");
    fixture.write("notes.txt", "notes\n");
    fixture.commit("start");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let asked = Arc::new(AtomicBool::new(false));
    let asked_in_ui = Arc::clone(&asked);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !asked_in_ui.load(Ordering::Relaxed) && matches!(app.model.stage, Stage::Ready) {
                app.open_pane(OpenPaneRequest::Extension {
                    name: "files".to_string(),
                });
                asked_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            opened_in_ui.store(
                app.model.layout.panes().any(|(_, pane)| {
                    matches!(pane, Pane::File { file_path, .. } if file_path == "notes.txt")
                }),
                Ordering::Relaxed,
            );
        });

    let until = Instant::now() + PATIENCE;
    while harness.query_by_label("notes.txt").is_none() {
        assert!(
            Instant::now() < until,
            "the files pane never listed the project"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    // The table's first frame is the one it measures its columns in; they are laid out from
    // the next.
    for _ in 0..SETTLING_FRAMES {
        harness.step();
    }
    (harness, opened, fixture)
}

/// Step the window until `notes.txt` is open in a tab.
fn until_notes_open(harness: &mut Harness<'_>, opened: &AtomicBool) {
    let until = Instant::now() + PATIENCE;
    while !opened.load(Ordering::Relaxed) {
        assert!(
            Instant::now() < until,
            "notes.txt was never opened in a tab"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_files_extension_walks_the_project_with_the_keyboard_and_opens_a_file() {
    // Arrange
    let (mut harness, opened, _fixture) = files_pane("extension-files");
    harness.snapshot("extension-files");

    // Act: past `..` and the folder, onto the file, and open it.
    press_key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    press_key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    press_key(&mut harness, Key::Enter, Modifiers::NONE);

    // Assert
    until_notes_open(&mut harness, &opened);
}

#[test]
fn a_double_click_on_a_file_in_the_files_extension_opens_it() {
    // Arrange
    let (mut harness, opened, _fixture) = files_pane("extension-files-double-click");
    let row = harness.get_by_label("notes.txt").rect().center();

    // Act: both clicks in the one frame. egui pairs two clicks by the time between them on its
    // own clock, which the harness moves by whole frames - two frames of clicks can be further
    // apart on it than a hand ever is.
    let click = [
        egui::Event::PointerButton {
            pos: row,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: row,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ];
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(row));
    harness.input_mut().events.extend(click.clone());
    harness.input_mut().events.extend(click);
    harness.step();

    // Assert
    until_notes_open(&mut harness, &opened);
}

/// The palette opened over an extension's pane has the keyboard, so Escape puts it away
/// rather than reaching the script underneath.
#[test]
fn escape_puts_the_palette_away_over_an_extension() {
    let (mut harness, _opened, _fixture) = files_pane("extension-palette-escape");

    press_key(
        &mut harness,
        Key::P,
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
    );
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role(egui::accesskit::Role::TextInput)
            .is_some(),
        "cmd+shift+P should have opened the palette"
    );

    press_key(&mut harness, Key::Escape, Modifiers::NONE);
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role(egui::accesskit::Role::TextInput)
            .is_none(),
        "Escape should have put the palette away"
    );
}
