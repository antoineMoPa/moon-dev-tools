//! Window zoom - a double click on an empty part of a tab strip asks for the window to be zoomed.
#![cfg(target_os = "macos")]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::{model::Stage, panes::Pane, theme::ThemeMode};

use super::{app_for, frame_rects, seeded_fixture, settle, tab_rects};

/// The window is split in two columns, and it is the second one's strip that is double clicked:
/// every column's strip stands in for the title bar, not just the first's. A double click on a
/// tab is the tab's.
#[test]
fn a_double_click_on_the_empty_strip_of_any_column_asks_for_a_zoom() {
    let fixture = seeded_fixture("window-zoom");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let frames = Arc::new(Mutex::new(Vec::<egui::Rect>::new()));
    let frames_in_ui = Arc::clone(&frames);
    let tabs = Arc::new(Mutex::new(Vec::<egui::Rect>::new()));
    let tabs_in_ui = Arc::clone(&tabs);
    let zoom_asked = Arc::new(AtomicBool::new(false));
    let zoom_asked_in_ui = Arc::clone(&zoom_asked);
    let mut split = false;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .build_ui(move |ui| {
            if !split && matches!(app.model.stage, Stage::Ready) {
                app.model.layout.add_pane_against_edge(
                    egui_frames::DropSide::Right,
                    egui_frames::DEFAULT_EDGE_SHARE,
                    Pane::Agents,
                );
                split = true;
            }
            app.draw(ui);
            *frames_in_ui.lock().expect("poisoned") = frame_rects(&app);
            *tabs_in_ui.lock().expect("poisoned") = tab_rects(&app);
            zoom_asked_in_ui.store(app.window_drag.zoom_asked, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || frames.lock().expect("poisoned").len() == 2),
        "the window should be split in two columns"
    );
    harness.run_steps(3);

    let second_column = frames
        .lock()
        .expect("poisoned")
        .iter()
        .copied()
        .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("there are two frames");
    let tabs_of_second_column: Vec<egui::Rect> = tabs
        .lock()
        .expect("poisoned")
        .iter()
        .copied()
        .filter(|tab| second_column.contains(tab.center()))
        .collect();
    let tab = *tabs_of_second_column
        .first()
        .expect("the second column has a tab");
    let last_tab_ends = tabs_of_second_column
        .iter()
        .map(|tab| tab.max.x)
        .fold(f32::MIN, f32::max);
    let empty_strip = egui::pos2((last_tab_ends + second_column.max.x) / 2.0, tab.center().y);

    let double_click = |harness: &mut Harness<'_>, at: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        harness.input_mut().events.push(egui::Event::PointerMoved(at));
        harness.step();
        for _ in 0..2 {
            harness
                .input_mut()
                .events
                .extend([button(true), button(false)]);
            harness.step();
        }
        harness.run_steps(2);
    };

    double_click(&mut harness, tab.center());
    assert!(
        !zoom_asked.load(Ordering::Relaxed),
        "a double click on a tab is the tab's, not a zoom"
    );

    double_click(&mut harness, empty_strip);
    assert!(
        zoom_asked.load(Ordering::Relaxed),
        "a double click on the empty strip of the second column should ask for a zoom"
    );
}
