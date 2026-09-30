//! Spaces: several projects' worth of window kept in one window, and the switch between them.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{model::Stage, palette::CommandAction, theme::ThemeMode};

use super::{app_for, seeded_fixture};

/// What the test reads back out of the window after each pass.
#[derive(Default, Clone)]
struct Seen {
    count: usize,
    front: usize,
    on_launch_screen: bool,
    ready: bool,
    project_path: Option<String>,
    /// The names the selector's list would give the spaces.
    names: Vec<String>,
}

#[test]
fn a_space_nobody_has_been_to_opens_on_the_project_in_front() {
    let fixture = seeded_fixture("spaces");
    let repo_path = fixture.root.display().to_string();
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let asked = Arc::new(Mutex::new(None::<CommandAction>));
    let asked_in_ui = Arc::clone(&asked);
    let watched = Arc::new(Mutex::new(Seen::default()));
    let watched_in_ui = Arc::clone(&watched);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if let Some(action) = asked_in_ui.lock().expect("the ask").take() {
                app.pending_action = Some(action);
            }
            app.draw(ui);
            let on_review = matches!(app.model.stage, Stage::Ready)
                && app
                    .model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some());
            ready_in_ui.store(on_review, Ordering::Relaxed);
            *watched_in_ui.lock().expect("what was seen") = Seen {
                count: app.spaces.count(),
                front: app.spaces.front(),
                on_launch_screen: matches!(app.model.stage, Stage::Prompt { .. }),
                ready: on_review,
                project_path: app.model.project_path.clone(),
                names: app.space_views().iter().map(|space| space.name()).collect(),
            };
        });

    step_until(&mut harness, || seen(&watched).ready, "the review never finished loading");
    let start = seen(&watched);
    assert_eq!(start.count, 4, "a window starts with four spaces");
    assert_eq!(start.names, vec!["repo"; 4], "each opens on the project in front");
    harness.run_steps(3);
    // A split in the first space, so its preview has more than one block to show.
    *asked.lock().expect("the ask") = Some(CommandAction::Split(egui_frames::DropSide::Right));
    harness.run_steps(30);

    // Act: go to a space nobody has been to. It opens on the same project, with no launch
    // screen in between.
    *asked.lock().expect("the ask") = Some(CommandAction::GoToSpace(1));
    step_until(&mut harness, || seen(&watched).front == 1 && seen(&watched).ready, "the second space did not open");
    let second = seen(&watched);
    assert!(!second.on_launch_screen);
    assert_eq!(second.project_path.as_deref(), Some(repo_path.as_str()));

    // Act: back to the first, which is where it was left.
    *asked.lock().expect("the ask") = Some(CommandAction::GoToSpace(0));
    step_until(&mut harness, || seen(&watched).front == 0 && seen(&watched).ready, "the first space did not come back");
    harness.run_steps(3);
    if let Ok(directory) = std::env::var("MOON_SPACES_SNAPSHOT_DIR") {
        harness
            .render()
            .expect("the window renders")
            .save(std::path::Path::new(&directory).join("spaces.png"))
            .expect("the picture is written");
    }

    // Act: hold the first square and drag it along to the last place. The space in front is
    // the one that moved, so it is still the one in front, in its new place.
    let press = |pressed: bool, at: egui::Pos2| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let first_square = egui::pos2(787.0, 580.0);
    harness.event(egui::Event::PointerMoved(first_square));
    harness.event(press(true, first_square));
    for _ in 0..12 {
        harness.step();
        std::thread::sleep(Duration::from_millis(50));
    }
    let last_place = egui::pos2(875.0, 580.0);
    harness.event(egui::Event::PointerMoved(last_place));
    harness.run_steps(3);
    harness.event(press(false, last_place));
    harness.run_steps(3);
    assert_eq!(seen(&watched).front, 3, "the held space should have moved to the last place");

    // Act: the second is emptied, and is untouched again.
    *asked.lock().expect("the ask") = Some(CommandAction::CloseSpaceAt(0));
    harness.run_steps(3);
    assert_eq!(seen(&watched).count, 4, "the list keeps its four spaces");

    // The only space in use cannot be emptied: that is closing the window.
    *asked.lock().expect("the ask") = Some(CommandAction::CloseSpaceAt(3));
    harness.run_steps(3);
    let last = seen(&watched);
    assert_eq!(last.front, 3);
    assert!(last.ready, "and the window is still on its review");

    // Next wraps round the four: after the last comes the first.
    *asked.lock().expect("the ask") = Some(CommandAction::NextSpace);
    step_until(&mut harness, || seen(&watched).front == 0 && seen(&watched).ready, "next did not wrap");
}

fn seen(seen: &Arc<Mutex<Seen>>) -> Seen {
    seen.lock().expect("what was seen").clone()
}

fn step_until(harness: &mut Harness<'static>, done: impl Fn() -> bool, complaint: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !done() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "{complaint}");
}
