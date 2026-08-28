//! Keyboard, mouse and wheel input aimed at a shell pane, and where it lands.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::{
    api::OpenSessionRequest,
    backend::local::LocalBackend,
    native::{Launch, app::App, panes::Pane, theme::ThemeMode},
};

use super::{app_for, seeded_fixture, click_at, settle, press_key, drag_from_to};

/// cmd+c pressed while the keyboard is in a shell belongs to that shell, even when the
/// review beside it still shows a selection - the selection the user just made is the
/// shell's, and answering with the diff's older one is what made this confusing.
///
/// The shell is a stand-in widget rather than a real terminal: what the review checks is
/// "the focused id is one a terminal pane recorded", and that is what this drives.
#[test]
fn a_copy_pressed_in_a_shell_stays_with_the_shell() {
    let fixture = seeded_fixture("copy-owner");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        copied: Option<String>,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let in_shell = Arc::new(AtomicBool::new(false));
    let in_shell_in_ui = Arc::clone(&in_shell);
    let shell_id = egui::Id::new("stand-in-shell");
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            if in_shell_in_ui.load(Ordering::Relaxed) {
                // What drawing a focused terminal pane does: a real widget holds the
                // keyboard, and the model knows that widget is a shell's. It floats in an
                // area of its own so the app underneath keeps its layout.
                egui::Area::new("stand-in-shell-area".into())
                    .fixed_pos(egui::pos2(2.0, 2.0))
                    .show(ui.ctx(), |ui| {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(2.0, 2.0),
                            egui::vec2(8.0, 8.0),
                        );
                        let response = ui.interact(rect, shell_id, egui::Sense::click());
                        response.request_focus();
                    });
                app.model.terminal_with_keyboard = Some(shell_id);
            }
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                if let Some(text) = ui.ctx().output(|output| {
                    output.commands.iter().find_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    })
                }) {
                    seen.copied = Some(text);
                }
            }
            ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
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

    let (hunk_id, patch) = {
        let state = seen.lock().expect("expected the hunk");
        (
            state.hunk_id.clone().expect("expected a hunk to select in"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let line_index = lines
        .iter()
        .position(|line| line.kind == crate::native::review::diff::LineKind::Added)
        .expect("expected an added line to select");

    // A diff line is selected, the way the review remembers a selection across other work.
    let rect = harness
        .ctx
        .read_response(crate::native::review::hunks::diff_line_id(
            &hunk_id, line_index,
        ))
        .expect("expected the diff line to have been drawn")
        .rect;
    click_at(&mut harness, rect.center());
    harness.run_steps(2);

    // The keyboard moves into a shell, and the chord follows it: the review must not answer.
    in_shell.store(true, Ordering::Relaxed);
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.run_steps(3);
    assert!(
        seen.lock().expect("poisoned").copied.is_none(),
        "with the keyboard in a shell, the review must leave cmd+c alone"
    );

    // The shell lets the keyboard go, and the chord is the review's again.
    in_shell.store(false, Ordering::Relaxed);
    harness
        .ctx
        .memory_mut(|memory| memory.surrender_focus(shell_id));
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.run_steps(3);
    assert!(
        seen.lock().expect("poisoned").copied.is_some(),
        "with the keyboard nowhere, the review's selection answers the copy"
    );
}

/// Tab belongs to the shell - it is how a path gets completed - not to egui's focus
/// traversal. Before the pane locked it, the first Tab moved the keyboard to the next
/// widget and everything typed after it went nowhere.
#[test]
fn tab_stays_with_the_shell_instead_of_moving_focus() {
    let fixture = seeded_fixture("terminal-tab");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = Arc::new(LocalBackend::new(state));
    let opened = crate::backend::Backend::open_session(
        backend.as_ref(),
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");

    let terminal_id =
        crate::backend::Backend::create_terminal(backend.as_ref(), &opened.session_id, None)
            .expect("expected a shell to start");
    let attachment = crate::backend::Backend::attach_terminal(
        backend.as_ref(),
        &opened.session_id,
        &terminal_id,
    )
    .expect("expected to attach to the shell");
    let pane = egui_tty::Terminal::new(attachment)
        .expect("expected the terminal emulator to start")
        .with_label(terminal_id.clone());

    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), pane);

    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    let for_pane = terminal_id.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            if !placed_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: for_pane.clone(),
                        command: None,
                        task_id: None,
                    },
                    None,
                );
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });

    let ready = settle(&mut harness, || placed.load(Ordering::Relaxed));
    assert!(ready, "the shell tab was never placed");
    harness.run_steps(3);

    // Clicking into the shell's body is how it takes the keyboard.
    click_at(&mut harness, egui::pos2(650.0, 500.0));
    let before = harness
        .ctx
        .memory(|memory| memory.focused())
        .expect("clicking into the shell should have given it the keyboard");

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);

    assert_eq!(
        harness.ctx.memory(|memory| memory.focused()),
        Some(before),
        "Tab must stay with the shell rather than moving the keyboard on"
    );

    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// The same for a shell: raising its tab puts the keyboard in it, so what is typed next goes
/// to the program running there rather than nowhere.
#[test]
fn raising_a_shell_tab_hands_it_the_keyboard() {
    let fixture = seeded_fixture("shell-tab-keyboard");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = Arc::new(LocalBackend::new(state));
    let opened = crate::backend::Backend::open_session(
        backend.as_ref(),
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");

    let terminal_id =
        crate::backend::Backend::create_terminal(backend.as_ref(), &opened.session_id, None)
            .expect("expected a shell to start");
    let attachment = crate::backend::Backend::attach_terminal(
        backend.as_ref(),
        &opened.session_id,
        &terminal_id,
    )
    .expect("expected to attach to the shell");
    let shell = egui_tty::Terminal::new(attachment)
        .expect("expected the terminal emulator to start")
        .with_label(terminal_id.clone());

    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), shell);

    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    let for_pane = terminal_id.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            // Beside the review, so the frame has a review tab and a shell tab.
            if !placed_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: for_pane.clone(),
                        command: None,
                        task_id: None,
                    },
                    None,
                );
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });

    let ready = settle(&mut harness, || placed.load(Ordering::Relaxed));
    assert!(ready, "the shell tab was never placed");
    // A shell takes focus the frame after it is asked for, so this is not the first frame.
    harness.run_steps(3);
    let in_the_shell = harness
        .ctx
        .memory(|memory| memory.focused())
        .expect("opening a shell should have put the keyboard in it");

    press_key(&mut harness, egui::Key::Num1, egui::Modifiers::COMMAND);
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_none(),
        "the review has nothing to type into, so the shell must have let the keyboard go"
    );

    press_key(&mut harness, egui::Key::Num2, egui::Modifiers::COMMAND);
    assert_eq!(
        harness.ctx.memory(|memory| memory.focused()),
        Some(in_the_shell),
        "raising the shell again should have given it the keyboard back"
    );

    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// Dragging over a shell selects what the pointer swept, in the real pane with a real pty
/// behind it. The gesture itself is tested against the emulator directly; this is about the
/// pane handing egui's pointer to it at all.
#[test]
fn dragging_over_a_shell_selects_its_text() {
    let fixture = seeded_fixture("terminal-select");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = Arc::new(LocalBackend::new(state));
    let opened = crate::backend::Backend::open_session(
        backend.as_ref(),
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");

    let terminal_id =
        crate::backend::Backend::create_terminal(backend.as_ref(), &opened.session_id, None)
            .expect("expected a shell to start");
    let attachment = crate::backend::Backend::attach_terminal(
        backend.as_ref(),
        &opened.session_id,
        &terminal_id,
    )
    .expect("expected to attach to the shell");
    let pane = egui_tty::Terminal::new(attachment)
        .expect("expected the terminal emulator to start")
        .with_label(terminal_id.clone());

    // Enough marked-up lines to fill the grid, so wherever the drag lands it lands on one.
    pane.send(b"i=0; while [ $i -lt 200 ]; do printf 'moonreviewline%s\\n' $i; i=$((i+1)); done\n")
        .expect("expected to write to the shell");

    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), pane);

    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    /// What the test needs back out of the pane each frame.
    #[derive(Default, Clone)]
    struct Seen {
        screen: String,
        selected: Option<String>,
        rect: Option<egui::Rect>,
        /// What a copy put on the clipboard, read inside the frame that did it: egui hands
        /// its output to the integration at the end of every pass, so afterwards it is gone.
        copied: Option<String>,
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let for_pane = terminal_id.clone();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            if !placed_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: for_pane.clone(),
                        command: None,
                        task_id: None,
                    },
                    None,
                );
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            let rect = app.frames.frame_rect(app.model.layout.active_frame());
            if let Some(pane) = app.terminals.get_mut(&for_pane)
                && let Ok(mut seen) = seen_in_ui.lock()
            {
                seen.screen = pane.visible_text().unwrap_or_default();
                seen.selected = pane.selected_text();
                seen.rect = rect;
                if let Some(text) = ui.ctx().output(|output| {
                    output.commands.iter().find_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    })
                }) {
                    seen.copied = Some(text);
                }
            }
        });

    // Enough of them to have filled the grid, whatever size the pane settled at.
    let printed = settle(&mut harness, || {
        seen.lock()
            .expect("poisoned")
            .screen
            .matches("moonreviewline")
            .count()
            > 5
    });
    assert!(
        printed,
        "the shell's output never filled the grid; screen was:\n{}",
        seen.lock().expect("poisoned").screen
    );
    harness.run_steps(2);

    assert!(
        seen.lock().expect("poisoned").selected.is_none(),
        "nothing is selected before a drag"
    );

    let rect = seen
        .lock()
        .expect("poisoned")
        .rect
        .expect("the shell's frame should have been drawn");
    // Across the middle of the pane, which the printed lines fill.
    let middle = rect.center().y;
    drag_from_to(
        &mut harness,
        egui::pos2(rect.min.x + 20.0, middle),
        egui::pos2(rect.max.x - 20.0, middle),
    );

    let selected = seen
        .lock()
        .expect("poisoned")
        .selected
        .clone()
        .expect("the drag should have selected something");
    // Where the sweep started is a few cells in from the left, so what comes back is the
    // tail of the marker rather than the whole of it.
    assert!(
        selected.contains("reviewline"),
        "the drag should have selected the line it swept, got {selected:?}"
    );
    assert!(
        !selected.contains('\n'),
        "a sweep along one row should not have taken any other, got {selected:?}"
    );

    // Copy takes the selection, and paste goes to the program. Both arrive as events of their
    // own rather than as keystrokes, so what this checks is that they still reach the pane
    // through everything the window does to the keyboard on the way.
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").copied.as_deref(),
        Some(selected.as_str()),
        "copy should have put the selection on the clipboard"
    );

    harness
        .input_mut()
        .events
        .push(egui::Event::Paste("moonreviewpaste".to_string()));
    harness.step();
    harness.run_steps(2);
    let pasted = settle(&mut harness, || {
        seen.lock()
            .expect("poisoned")
            .screen
            .contains("moonreviewpaste")
    });
    assert!(
        pasted,
        "paste should have reached the shell; screen was:\n{}",
        seen.lock().expect("poisoned").screen
    );

    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// The wheel over a shell pane scrolls it: back into the scrollback for a plain shell, and as
/// wheel presses for a program that asked for the mouse.
#[test]
fn the_wheel_scrolls_a_shell_pane() {
    let fixture = seeded_fixture("shell-scroll");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = Arc::new(LocalBackend::new(state));
    let opened = crate::backend::Backend::open_session(
        backend.as_ref(),
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");

    let terminal_id =
        crate::backend::Backend::create_terminal(backend.as_ref(), &opened.session_id, None)
            .expect("expected a shell to start");
    let attachment = crate::backend::Backend::attach_terminal(
        backend.as_ref(),
        &opened.session_id,
        &terminal_id,
    )
    .expect("expected to attach to the shell");
    let pane = egui_tty::Terminal::new(attachment)
        .expect("expected the terminal emulator to start")
        .with_label(terminal_id.clone());
    pane.send(b"seq 1 200\n")
        .expect("expected to write to the shell");

    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), pane);

    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    let visible = Arc::new(Mutex::new(String::new()));
    let visible_in_ui = Arc::clone(&visible);
    let for_pane = terminal_id.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            if !placed_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.active_frame();
                let pane = app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: for_pane.clone(),
                        command: None,
                        task_id: None,
                    },
                    None,
                );
                app.model.layout.focus_pane(pane);
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            if let Some(terminal) = app.terminals.get_mut(&for_pane) {
                *visible_in_ui.lock().expect("poisoned") =
                    terminal.visible_text().unwrap_or_default();
            }
        });

    // Wait for the shell to have printed all two hundred lines.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if visible.lock().expect("poisoned").contains("195") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let before = visible.lock().expect("poisoned").clone();
    assert!(
        before.contains("195"),
        "expected the shell to have printed: {before}"
    );

    // A wheel over the middle of the pane, where the shell is drawn.
    let middle = egui::pos2(650.0, 500.0);
    for _ in 0..4 {
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(middle),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, 5.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        harness.step();
    }
    harness.step();

    let after = visible.lock().expect("poisoned").clone();
    assert_ne!(before, after, "expected the wheel to scroll the shell back");
}
