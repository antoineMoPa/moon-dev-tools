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

use serde_json::json;

use crate::{
    extensions::Element,
    native::{
        extension_pane::ExtensionPane,
        model::Stage,
        panes::{OpenPaneRequest, Pane},
        theme::ThemeMode,
    },
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
    // The filter takes the keyboard as the pane opens, and a blinking caret would make the
    // image differ run to run.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);

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

    // Act: from the filter box, which has the keyboard, past `..` and the folder, onto the
    // file, and open it.
    press_key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    press_key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    press_key(&mut harness, Key::Enter, Modifiers::NONE);

    // Assert
    until_notes_open(&mut harness, &opened);
}

/// A folder is found by typing part of its name and opened with Enter, all from the filter
/// box - and the box is emptied in the folder it went into, while it keeps the keyboard.
#[test]
fn the_files_extension_opens_what_its_filter_found_with_enter() {
    use egui_kittest::kittest::NodeT;

    // Arrange
    let (mut harness, opened, _fixture) = files_pane("extension-files-filter-enter");

    // Act: `doc` typed, which leaves the folder alone in the list, and Enter.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("doc".to_string()));
    let until = Instant::now() + PATIENCE;
    while harness.query_by_label("notes.txt").is_some() {
        assert!(Instant::now() < until, "the filter never narrowed the list");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    press_key(&mut harness, Key::Enter, Modifiers::NONE);

    // Assert
    let until = Instant::now() + PATIENCE;
    while harness.query_by_label("guide.md").is_none() {
        assert!(Instant::now() < until, "Enter never went into the folder");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);
    let filter = harness.get_by_role(egui::accesskit::Role::TextInput);
    assert_eq!(
        filter.accesskit_node().value().as_deref(),
        Some(""),
        "the folder gone into is shown whole"
    );
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_some(),
        "the filter box keeps the keyboard"
    );
    assert!(!opened.load(Ordering::Relaxed));
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
    // The files pane's own filter box is one of them; the palette's is the other.
    let text_inputs = |harness: &Harness<'_>| {
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .count()
    };

    press_key(
        &mut harness,
        Key::P,
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
    );
    harness.run_steps(2);
    assert_eq!(
        text_inputs(&harness),
        2,
        "cmd+shift+P should have opened the palette"
    );

    press_key(&mut harness, Key::Escape, Modifiers::NONE);
    harness.run_steps(2);
    assert_eq!(
        text_inputs(&harness),
        1,
        "Escape should have put the palette away"
    );
}

/// An input marked `focus` has the keyboard as soon as its pane is drawn, and Escape hands it
/// back for good: the pane does not take it again on the next frame.
#[test]
fn an_input_asking_for_the_keyboard_has_it_when_the_pane_opens_until_escape() {
    let fixture = Fixture::new("extension-input-focus");
    fixture.write("notes.txt", "notes\n");
    fixture.commit("start");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let shown = Arc::new(AtomicBool::new(false));
    let shown_in_ui = Arc::clone(&shown);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| {
            if !shown_in_ui.load(Ordering::Relaxed) && matches!(app.model.stage, Stage::Ready) {
                app.open_pane(OpenPaneRequest::Extension {
                    name: "docker".to_string(),
                });
                // The view as a script would write it, put in the script's place before the
                // script is started: what docker says about this machine is not the test's.
                let (pane_id, _) = app
                    .model
                    .layout
                    .panes()
                    .find(|(_, pane)| matches!(pane, Pane::Extension { .. }))
                    .expect("the extension pane was just opened");
                app.model.extension_panes.insert(
                    pane_id,
                    ExtensionPane::showing(Element::Column {
                        children: vec![Element::Input {
                            id: "filter".to_string(),
                            value: String::new(),
                            hint: "filter".to_string(),
                            on_change: json!({ "filter": true }),
                            focus: true,
                        }],
                    }),
                );
                shown_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });

    let until = Instant::now() + PATIENCE;
    while !shown.load(Ordering::Relaxed) {
        assert!(
            Instant::now() < until,
            "the window never opened its project"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_some(),
        "the input should have the keyboard as the pane opens"
    );

    press_key(&mut harness, Key::Escape, Modifiers::NONE);

    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_none(),
        "Escape should have handed the keyboard back, and the pane should not take it again"
    );
}
