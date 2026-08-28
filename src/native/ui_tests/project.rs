//! The project pane: the two commands the Project menu runs, typed into the repo's own file.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    native::{
        palette::CommandAction,
        panes::{OpenPaneRequest, PaneKind},
        theme::ThemeMode,
    },
    project::ProjectCommand,
};

use super::{app_for, seeded_fixture};

/// What the test types into the build box. A command of its own rather than a real build:
/// the point is that what is typed is what runs, and this one says so in one line.
const BUILD_COMMAND: &str = "printf %s [the-build-command-ran]";

#[test]
fn what_is_typed_into_the_project_pane_is_what_the_palette_runs() {
    // Arrange: a repo with no project file at all, which is every repo the first time.
    let fixture = seeded_fixture("project-pane");
    let project_file = fixture.root.join(".moonreview.json");
    assert!(!project_file.exists(), "the fixture starts with no project file");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // Set once the file has been read and the pane has boxes to type into.
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // What the palette would offer, read after each pass: the build command only appears
    // once the file says there is one.
    let offers_build = Arc::new(AtomicBool::new(false));
    let offers_build_in_ui = Arc::clone(&offers_build);
    // Set once the build command has a shell of its own on screen.
    let running = Arc::new(AtomicBool::new(false));
    let running_in_ui = Arc::clone(&running);
    // Raised by the assertions below to ask the window to run the build command, the way the
    // palette and the menu bar both ask for it.
    let run_build = Arc::new(AtomicBool::new(false));
    let run_build_in_ui = Arc::clone(&run_build);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Project);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if run_build_in_ui.swap(false, Ordering::Relaxed) {
                app.pending_action = Some(CommandAction::RunProject(ProjectCommand::Build));
            }
            app.draw(ui);
            ready_in_ui.store(app.model.project_editor.is_some(), Ordering::Relaxed);
            running_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .any(|(_, pane)| pane.kind() == PaneKind::Terminal),
                Ordering::Relaxed,
            );
            offers_build_in_ui.store(
                crate::native::palette::commands_for(&app)
                    .iter()
                    .any(|command| command.title == "build"),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the pane never read the project file"
    );
    harness.run_steps(3);

    assert!(
        !offers_build.load(Ordering::Relaxed),
        "a project with no build command should offer none"
    );

    // Act: the build box has the keyboard from the moment the pane opens, so what is typed
    // goes in it without a click first.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(BUILD_COMMAND.to_string()));
    harness.run_steps(3);

    // Assert: it is written to the repo's file as it is typed, and the palette offers it.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut text = String::new();
    while Instant::now() < deadline {
        harness.step();
        // Read for the command rather than for the file: a write is a create and a fill, and
        // a file that exists is not yet a file that says anything.
        text = std::fs::read_to_string(&project_file).unwrap_or_default();
        if text.contains(BUILD_COMMAND) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        text.contains(BUILD_COMMAND),
        "the project file should hold what was typed, got {text:?}"
    );
    harness.run_steps(3);
    assert!(
        offers_build.load(Ordering::Relaxed),
        "the palette should offer the build command the pane just wrote"
    );

    // And the pane says which items the menu now has.
    harness.get_by_label("the Project menu offers build");

    // The box that was typed in has the keyboard, and a blinking caret is a picture that
    // depends on which frame it was taken on. Held on, it is the same caret every run.
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
    });
    harness.run_steps(2);
    harness.snapshot("project-settings");

    // Act: running it opens a shell of its own, which is where its output is.
    run_build.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if running.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        running.load(Ordering::Relaxed),
        "the build command should have opened a shell of its own"
    );
}
