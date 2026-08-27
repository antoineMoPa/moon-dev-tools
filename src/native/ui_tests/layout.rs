//! Frames, splits and tabs: how the window rearranges itself.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::native::{panes::Pane, panes::PaneKind, theme::ThemeMode};

use super::{frame_rects, tab_rects, Fixture, app_for, seeded_fixture, asked_to_close, settle, press_key};

/// A split handle keeps resizing while the pointer runs past it - the drag belongs to the
/// handle until the button comes up, not to the few points it happened to start on.
#[test]
fn dragging_a_split_handle_keeps_resizing_past_its_own_width() {
    let fixture = seeded_fixture("split-drag");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    let sizes = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sizes_in_ui = Arc::clone(&sizes);
    let handle_x = Arc::new(Mutex::new(None::<f32>));
    let handle_in_ui = Arc::clone(&handle_x);
    let split = Arc::new(AtomicBool::new(false));
    let split_in_ui = Arc::clone(&split);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            // A second column to have a handle between: the shell pane needs no shell to be
            // laid out, and this test is about the handle.
            if !split_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.model.layout.add_pane_against_edge(
                    egui_frames::DropSide::Right,
                    egui_frames::DEFAULT_EDGE_SHARE,
                    Pane::Agents,
                );
                split_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            if let egui_frames::LayoutNode::Split { sizes, .. } = app.model.layout.root() {
                *sizes_in_ui.lock().expect("poisoned") = sizes.clone();
            }
            // The handle sits where the first frame ends.
            let mut lefts: Vec<f32> = frame_rects(&app).iter().map(|r| r.max.x).collect();
            lefts.sort_by(|a, b| a.partial_cmp(b).expect("no NaN rects"));
            *handle_in_ui.lock().expect("poisoned") = lefts.first().copied();
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && sizes.lock().expect("poisoned").len() < 2 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);

    let before = sizes.lock().expect("poisoned").clone();
    assert_eq!(before.len(), 2, "the workspace should be split in two");
    let handle = handle_x
        .lock()
        .expect("poisoned")
        .expect("the first frame should have been drawn");
    let at = egui::pos2(handle + 2.0, 400.0);

    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();

    // Far outside the handle: with the drag registered under an id that moved with it, this
    // is where resizing used to stop.
    for step in 1..=4 {
        let dragged = egui::pos2(at.x - 40.0 * step as f32, at.y);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(dragged));
        harness.step();
    }
    let after = sizes.lock().expect("poisoned").clone();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(
        after[0] < before[0] - 0.05,
        "dragging left should have given the first column less than it had: {before:?} to {after:?}"
    );
}

/// A frame with nothing in it is never drawn: the hint its body carries is the only way to
/// see one, and seeing one means something emptied a frame and left it behind.
#[test]
fn a_frame_left_empty_is_dropped_rather_than_drawn() {
    let fixture = seeded_fixture("empty-frame");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    let frames = Arc::new(Mutex::new(0usize));
    let frames_in_ui = Arc::clone(&frames);
    let emptied = Arc::new(AtomicBool::new(false));
    let emptied_in_ui = Arc::clone(&emptied);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            // A second frame whose pane goes away without it - what a forgetful caller leaves.
            if !emptied_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let stranded = app.model.layout.add_pane_against_edge(
                    egui_frames::DropSide::Right,
                    egui_frames::DEFAULT_EDGE_SHARE,
                    Pane::Agents,
                );
                // Its pane taken out from under it, which is what a forgetful caller leaves.
                app.model.layout.close_pane(stranded);
                emptied_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);
            *frames_in_ui.lock().expect("poisoned") = app.model.layout.frame_count();
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !emptied.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);

    assert_eq!(
        *frames.lock().expect("poisoned"),
        1,
        "the stranded frame should have been dropped, leaving the review's own"
    );
}

/// With frames stacked one above the other, the far edge of the window is how a tab becomes a
/// column beside both - the frame it is dropped over would only split itself.
#[test]
fn dropping_a_tab_at_the_window_edge_makes_a_column_beside_every_frame() {
    let fixture = seeded_fixture("edge-drop");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    let shape = Arc::new(Mutex::new(String::new()));
    let shape_in_ui = Arc::clone(&shape);
    let tab_rect = Arc::new(Mutex::new(None::<egui::Rect>));
    let tab_in_ui = Arc::clone(&tab_rect);
    let stacked = Arc::new(AtomicBool::new(false));
    let stacked_in_ui = Arc::clone(&stacked);
    let right_edge = Arc::new(Mutex::new(f32::NEG_INFINITY));
    let edge_in_ui = Arc::clone(&right_edge);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            // Two frames, one above the other, and a third tab in the lower one to drag.
            if !stacked_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                let moved = app.model.layout.add_pane(frame, Pane::Agents, None);
                app.model.layout.move_pane_to_frame(
                    moved,
                    frame,
                    egui_frames::DropSide::Bottom,
                    None,
                );
                stacked_in_ui.store(true, Ordering::Relaxed);
            }

            app.draw(ui);

            *shape_in_ui.lock().expect("poisoned") = match app.model.layout.root() {
                egui_frames::LayoutNode::Split {
                    direction,
                    children,
                    ..
                } => format!("{direction:?}-{}", children.len()),
                egui_frames::LayoutNode::Frame { .. } => "frame".to_string(),
            };
            *edge_in_ui.lock().expect("poisoned") = frame_rects(&app)
                .iter()
                .map(|rect| rect.max.x)
                .fold(f32::NEG_INFINITY, f32::max);
            // The tab of the lower frame, which is the one this drags.
            *tab_in_ui.lock().expect("poisoned") = tab_rects(&app)
                .into_iter()
                .max_by(|a, b| a.min.y.partial_cmp(&b.min.y).expect("no NaN rects"));
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !stacked.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);
    assert_eq!(
        *shape.lock().expect("poisoned"),
        "Column-2",
        "the workspace should start as two stacked frames"
    );

    let from = tab_rect
        .lock()
        .expect("poisoned")
        .expect("expected a tab to drag")
        .center();
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(from),
        egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();

    // Out to the right edge of everything drawn, well past the frame's own edge band.
    let at_edge = egui::pos2(*right_edge.lock().expect("poisoned") - 4.0, 400.0);
    for step in 1..=3 {
        let towards = egui::pos2(from.x + (at_edge.x - from.x) * step as f32 / 3.0, at_edge.y);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(towards));
        harness.step();
    }
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: at_edge,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);

    assert_eq!(
        *shape.lock().expect("poisoned"),
        "Row-2",
        "the tab should have become a column beside the stack, not a split of one frame"
    );
}

/// ⌘W is the window's own chord: it takes the tab in front, not the window around it.
#[test]
fn command_w_closes_the_tab_in_front() {
    let fixture = seeded_fixture("close-tab");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    let panes_left = Arc::new(Mutex::new(Vec::<PaneId>::new()));
    let panes_in_ui = Arc::clone(&panes_left);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *panes_in_ui.lock().expect("the pane list is poisoned") = app
                .model
                .layout
                .panes()
                .map(|(pane_id, _)| pane_id)
                .collect();
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);
    assert_eq!(
        panes_left.lock().expect("the pane list is poisoned").len(),
        1,
        "the review should be the one pane open"
    );

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::W,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.step();
    harness.run_steps(2);

    assert!(
        panes_left
            .lock()
            .expect("the pane list is poisoned")
            .is_empty(),
        "⌘W should have closed the review pane"
    );

    // And with nothing left in the workspace, the window goes with it rather than sitting
    // there empty.
    assert!(
        asked_to_close(&harness),
        "closing the last tab should have closed the window"
    );
}

/// cmd+1 and cmd+2 raise the first and second tab of the active frame, the way a browser
/// walks its tabs by number.
#[test]
fn command_digits_raise_the_numbered_tabs() {
    let fixture = Fixture::new("select-tab");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let active = Arc::new(Mutex::new(None::<PaneKind>));
    let active_in_ui = Arc::clone(&active);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            // A second tab beside the review, so there is a strip to walk.
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            *active_in_ui.lock().expect("the active pane is poisoned") = app.active_pane_kind();
            ready_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .any(|(_, pane)| matches!(pane, Pane::File { .. })),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the file tab never opened");
    harness.run_steps(2);

    let active_kind = || *active.lock().expect("the active pane is poisoned");
    assert_eq!(
        active_kind(),
        Some(PaneKind::File),
        "the file tab opens in front"
    );

    let press = |harness: &mut Harness<'_>, key: egui::Key| {
        harness.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        harness.step();
        harness.run_steps(2);
    };

    press(&mut harness, egui::Key::Num1);
    assert_eq!(
        active_kind(),
        Some(PaneKind::Review),
        "cmd+1 should raise the first tab, the review"
    );

    press(&mut harness, egui::Key::Num2);
    assert_eq!(
        active_kind(),
        Some(PaneKind::File),
        "cmd+2 should raise the second tab, the file"
    );

    // A digit past the end of the strip changes nothing.
    press(&mut harness, egui::Key::Num9);
    assert_eq!(active_kind(), Some(PaneKind::File));
}

/// Clicks have to reach the widgets inside a frame's body.
///
/// This exists because of a real regression: a click-sensing widget the size of each frame,
/// registered after its contents, sat on top of everything and swallowed every click - the
/// window looked completely inert except for the split handles.
#[test]
fn a_click_reaches_a_widget_inside_a_frame() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("clickable");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let collapsed = Arc::new(Mutex::new(false));
    let collapsed_in_ui = Arc::clone(&collapsed);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            if let Ok(mut collapsed) = collapsed_in_ui.lock() {
                *collapsed = app
                    .model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.collapsed_files.contains("src/lib.rs"));
            }
            let loaded = app
                .model
                .review_ref(&app.model.root_session_id)
                .is_some_and(|review| review.payload.is_some());
            ready_in_ui.store(loaded, Ordering::Relaxed);
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the review never loaded");
    harness.run_steps(2);

    assert!(
        !*collapsed.lock().expect("poisoned"),
        "every file starts expanded"
    );

    // A file heading sits deep inside the diff pane, which is what the swallowing overlay
    // used to cover.
    harness.get_by_label("\u{23F7} src/lib.rs").click();
    harness.run_steps(2);

    assert!(
        *collapsed.lock().expect("poisoned"),
        "clicking the file heading must collapse it"
    );
}

/// `C-x o` walks the keyboard round the workspace's frames. The prefix has to survive the
/// frame it was pressed in - it is two presses, and each one arrives in a pass of its own.
#[test]
fn c_x_o_hands_the_keyboard_to_the_next_frame() {
    let fixture = seeded_fixture("focus-frame");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    let split = Arc::new(AtomicBool::new(false));
    let split_in_ui = Arc::clone(&split);
    let frames = Arc::new(Mutex::new((Vec::<egui_frames::FrameId>::new(), None)));
    let frames_in_ui = Arc::clone(&frames);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            // A second frame down the right, so there is somewhere for the keyboard to go.
            if !split_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.model.layout.add_pane_against_edge(
                    egui_frames::DropSide::Right,
                    egui_frames::DEFAULT_EDGE_SHARE,
                    Pane::File {
                        session_id,
                        file_path: "src/lib.rs".to_string(),
                    },
                );
                split_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            *frames_in_ui.lock().expect("poisoned") = (
                app.model.layout.frame_ids(),
                Some(app.model.layout.active_frame()),
            );
        });

    let ready = settle(&mut harness, || split.load(Ordering::Relaxed));
    assert!(ready, "the workspace never got its second frame");
    harness.run_steps(3);

    let (frame_ids, active) = frames.lock().expect("poisoned").clone();
    assert_eq!(
        frame_ids.len(),
        2,
        "the test needs two frames to walk between"
    );
    let active = active.expect("expected a frame to have the keyboard");
    let started_at = frame_ids
        .iter()
        .position(|id| *id == active)
        .expect("the active frame must be one of them");

    press_key(&mut harness, egui::Key::X, egui::Modifiers::CTRL);
    let (_, still) = frames.lock().expect("poisoned").clone();
    assert_eq!(still, Some(active), "C-x on its own moves nothing");

    press_key(&mut harness, egui::Key::O, egui::Modifiers::NONE);
    let (_, moved_to) = frames.lock().expect("poisoned").clone();
    assert_eq!(
        moved_to,
        Some(frame_ids[(started_at + 1) % frame_ids.len()]),
        "C-x o should have handed the keyboard to the next frame"
    );

    // And round again, back to where it started.
    press_key(&mut harness, egui::Key::X, egui::Modifiers::CTRL);
    press_key(&mut harness, egui::Key::O, egui::Modifiers::NONE);
    let (_, wrapped) = frames.lock().expect("poisoned").clone();
    assert_eq!(
        wrapped,
        Some(active),
        "the walk wraps round at the last frame"
    );
}

/// The tab brought to the front is the one being worked in, so it is the one with the
/// keyboard: a file raised with cmd+2 can be typed into without clicking into its text, and
/// going back to the review with cmd+1 takes the keyboard back off it.
#[test]
fn raising_a_tab_hands_it_the_keyboard() {
    let fixture = seeded_fixture("tab-keyboard");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            // A file beside the review, so the frame has two tabs to move between.
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                let frame = app.model.layout.active_frame();
                app.model.layout.add_pane(
                    frame,
                    Pane::File {
                        session_id,
                        file_path: "src/lib.rs".to_string(),
                    },
                    None,
                );
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });

    let ready = settle(&mut harness, || opened.load(Ordering::Relaxed));
    assert!(ready, "the file tab was never opened");
    // The file arrives in front and its text has to be fetched before there is an editor to
    // type into, which is a round trip through the backend.
    let ctx = harness.ctx.clone();
    let typing = settle(&mut harness, || {
        ctx.memory(|memory| memory.focused()).is_some()
    });
    assert!(
        typing,
        "opening a file should have put the keyboard in its editor"
    );

    press_key(&mut harness, egui::Key::Num1, egui::Modifiers::COMMAND);
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_none(),
        "the review has nothing to type into, so the file must have let the keyboard go"
    );

    press_key(&mut harness, egui::Key::Num2, egui::Modifiers::COMMAND);
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_some(),
        "raising the file again should have given its editor the keyboard back"
    );
}

/// cmd+shift+R brings the review back. Closing it is a cmd+W away, so a window can end up
/// without one, and the chord opens it again rather than sending the user to the palette.
#[test]
fn the_review_chord_opens_the_review_again_after_it_is_closed() {
    let fixture = seeded_fixture("review-chord");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let reviews = Arc::new(AtomicUsize::new(0));
    let reviews_in_ui = Arc::clone(&reviews);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            reviews_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .filter(|(_, pane)| pane.kind() == PaneKind::Review)
                    .count(),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || reviews.load(Ordering::Relaxed) == 1),
        "the window never opened on its review"
    );

    press_key(&mut harness, egui::Key::W, egui::Modifiers::COMMAND);
    assert_eq!(
        reviews.load(Ordering::Relaxed),
        0,
        "cmd+W should have closed the review tab"
    );

    press_key(
        &mut harness,
        egui::Key::R,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert_eq!(
        reviews.load(Ordering::Relaxed),
        1,
        "cmd+shift+R should have opened the review again"
    );

    // And again on a window that already has one: the review is a pane there is one of.
    press_key(
        &mut harness,
        egui::Key::R,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert_eq!(
        reviews.load(Ordering::Relaxed),
        1,
        "the review that is open should be raised rather than duplicated"
    );
}
