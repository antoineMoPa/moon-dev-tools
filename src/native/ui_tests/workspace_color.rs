//! Painting a window's ground, so one window is told from another.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{
    model::Stage,
    palette::CommandAction,
    panes::OpenPaneRequest,
    theme::{Palette, ThemeMode},
    workspace_color::WorkspaceColor,
};

use super::{app_for, seeded_fixture};

/// The whole point of two shades per color: the window keeps the color it was marked with
/// when the light/dark switch is thrown, rather than going back to the palette's own ground.
///
/// The window is also marked through the same command the palette runs, and the mark has to
/// reach `settings.json`, or it is gone by the next launch.
#[test]
fn a_marked_workspace_keeps_its_color_across_the_theme_switch() {
    let fixture = seeded_fixture("workspace-color");
    let repo_path = fixture.root.display().to_string();
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    // Set once the review is open, which is when the window knows which project it is on:
    // the color is remembered against the project's path.
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // Raised by the assertions below to run the palette's command, as a person would.
    let mark = Arc::new(Mutex::new(None::<WorkspaceColor>));
    let mark_in_ui = Arc::clone(&mark);
    let switch_theme = Arc::new(AtomicBool::new(false));
    let switch_theme_in_ui = Arc::clone(&switch_theme);
    // What the window is painted, read back after each pass.
    let ground = Arc::new(Mutex::new(egui::Color32::TRANSPARENT));
    let ground_in_ui = Arc::clone(&ground);
    // Whether the palette still offers the color the window already is.
    let offers_teal = Arc::new(AtomicBool::new(false));
    let offers_teal_in_ui = Arc::clone(&offers_teal);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if let Some(color) = mark_in_ui.lock().expect("the mark").take() {
                app.pending_action = Some(CommandAction::MarkWorkspace(color));
            }
            if switch_theme_in_ui.swap(false, Ordering::Relaxed) {
                app.pending_action = Some(CommandAction::ToggleTheme);
            }
            app.draw(ui);
            // Waited on until the diff itself is on screen: a review still reading is a
            // window of empty ground, which would say nothing about the color under a diff.
            ready_in_ui.store(
                matches!(app.model.stage, Stage::Ready)
                    && app.model.project_path.is_some()
                    && app
                        .model
                        .review_ref(&app.model.root_session_id)
                        .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
            *ground_in_ui.lock().expect("the ground") = app.palette_of().bg;
            offers_teal_in_ui.store(
                crate::native::palette::commands_for(&app)
                    .iter()
                    .any(|command| command.title == "workspace color: teal"),
                Ordering::Relaxed,
            );
        });

    step_until(&mut harness, &ready, "the review never finished loading");
    // A couple more passes so the freshly arrived diff is laid out and painted.
    harness.run_steps(3);

    // An unmarked window is the window that shipped before there were colors.
    assert_eq!(
        *ground.lock().expect("the ground"),
        Palette::of(ThemeMode::Dark).bg,
        "an unmarked workspace should be painted the dark palette's own ground"
    );
    assert!(
        offers_teal.load(Ordering::Relaxed),
        "the palette should offer a color the window is not"
    );

    // Act: mark it, the way the command palette does.
    *mark.lock().expect("the mark") = Some(WorkspaceColor::Teal);
    harness.run_steps(3);

    assert_eq!(
        *ground.lock().expect("the ground"),
        WorkspaceColor::Teal.bg(ThemeMode::Dark),
        "a marked workspace should be painted teal's dark shade"
    );
    assert!(
        !offers_teal.load(Ordering::Relaxed),
        "the palette should not offer the color the window already is"
    );
    harness.snapshot("workspace-color-teal");

    // The mark outlives the window: it is what the next launch reads.
    let settings = crate::settings::load();
    assert_eq!(
        settings.workspace_color(&repo_path),
        WorkspaceColor::Teal,
        "the mark should have been written to the settings"
    );

    // Act: the same window, thrown to the light palette.
    switch_theme.store(true, Ordering::Relaxed);
    harness.run_steps(3);

    let light = *ground.lock().expect("the ground");
    assert_eq!(
        light,
        WorkspaceColor::Teal.bg(ThemeMode::Light),
        "the window should still be teal, in the shade the light palette wants"
    );
    assert_ne!(
        light,
        Palette::of(ThemeMode::Light).bg,
        "and not back to the light palette's own ground"
    );
    harness.snapshot("workspace-color-teal-light");

    // And the next window on this project comes up teal without being told: the settings
    // file is what carries the mark from one run to the next, and `follow_project_color` is
    // what picks it up once the review says which project the window is on.
    let reopened = Arc::new(AtomicBool::new(false));
    let reopened_in_ui = Arc::clone(&reopened);
    let reopened_ground = Arc::new(Mutex::new(egui::Color32::TRANSPARENT));
    let reopened_ground_in_ui = Arc::clone(&reopened_ground);
    let mut next = app_for(&fixture.root, ThemeMode::Dark);

    let mut next_harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            next.draw(ui);
            reopened_in_ui.store(next.model.project_path.is_some(), Ordering::Relaxed);
            *reopened_ground_in_ui.lock().expect("the ground") = next.palette_of().bg;
        });

    step_until(&mut next_harness, &reopened, "the second window never opened the project");
    next_harness.run_steps(3);

    assert_eq!(
        *reopened_ground.lock().expect("the ground"),
        WorkspaceColor::Teal.bg(ThemeMode::Dark),
        "a window reopened on a marked project should come up in its color"
    );
}

/// The swatches are drawn where a person would look for them, and they say which color the
/// window is.
#[test]
fn the_project_pane_offers_the_colors() {
    let fixture = seeded_fixture("workspace-color-pane");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 520.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed) && matches!(app.model.stage, Stage::Ready) {
                app.open_pane(OpenPaneRequest::Project);
                app.set_workspace_color(WorkspaceColor::Ember);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(app.model.project_editor.is_some(), Ordering::Relaxed);
        });

    step_until(&mut harness, &ready, "the project file was never read");
    // The pane hands the keyboard to its first box, and a blinking caret is a picture that
    // depends on which frame it was taken on. Held on, it is the same caret every run.
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
    });
    harness.run_steps(3);

    harness.snapshot("workspace-color-swatches");
}

/// Step the window until something the test is waiting for has happened.
fn step_until(harness: &mut Harness<'static>, done: &AtomicBool, complaint: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if done.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done.load(Ordering::Relaxed), "{complaint}");
}
