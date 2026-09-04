//! Shells: starting one, what it draws, and what happens when it or the window ends.

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

use super::{app_for, seeded_fixture, asked_to_close, asked_to_stay_open};

/// A shell that ends takes its tab with it: logging out of a terminal, or an agent finishing,
/// should leave the workspace as it was before the shell was opened.
#[test]
fn a_shell_that_exits_closes_its_tab() {
    let fixture = seeded_fixture("shell-exit");
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
    pane.send(b"exit\n")
        .expect("expected to write to the shell");

    // The window is built around that shell: one review tab and one shell tab.
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

    let panes_left = Arc::new(Mutex::new(0usize));
    let panes_in_ui = Arc::clone(&panes_left);
    let placed = Arc::new(AtomicBool::new(false));
    let placed_in_ui = Arc::clone(&placed);
    let for_pane = terminal_id.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            // Only once the review has opened: opening it replaces the whole arrangement,
            // which would take a shell tab added before it with it.
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
            *panes_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| {
                    matches!(pane, Pane::Terminal { terminal_id, .. } if *terminal_id == for_pane)
                })
                .count();
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut closed = false;
    while Instant::now() < deadline {
        harness.step();
        if *panes_left.lock().expect("poisoned") == 0 && placed.load(Ordering::Relaxed) {
            closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(closed, "the tab of a shell that exited should have closed");
}

/// A restart closes the window it was asked in, shells and all: the second instance is
/// already on its way, so the window has been answered rather than asked.
#[test]
fn restarting_closes_the_window_a_shell_is_still_running_in() {
    let fixture = seeded_fixture("restart-window");
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
        frame: crate::cli::Frame::Shell,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), pane);

    // The window is closed the way a restart closes it, once it is open with its shell
    // running: starting the second instance is the half a test has no business doing.
    let restart = Arc::new(AtomicBool::new(false));
    let restart_in_ui = Arc::clone(&restart);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let warnings_in_ui = Arc::clone(&warnings);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .build_ui(move |ui| {
            if restart_in_ui.swap(false, Ordering::Relaxed) {
                app.close_window(ui.ctx());
            }
            app.draw(ui);
            // A moonshell window opens a shell of its own beside the attached one, so what
            // matters is that at least one is running for the quit guard to be about.
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.running_shells() > 0,
                Ordering::Relaxed,
            );
            *warnings_in_ui.lock().expect("poisoned") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the window should have opened on the review, with its shell running"
    );

    restart.store(true, Ordering::Relaxed);
    harness.step();
    assert!(
        asked_to_close(&harness),
        "a restart should have closed the window"
    );

    // The close the window sent comes back to it as a close request, the way the windowing
    // system delivers one. Nothing here answers that with the quit warning: the restarted
    // instance is already starting, and a window that stayed put would leave two.
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("expected the root viewport")
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();

    assert!(
        !asked_to_stay_open(&harness),
        "a restart should not have taken its own close back"
    );
    assert!(
        !warnings
            .lock()
            .expect("poisoned")
            .iter()
            .any(|text| text.contains("still running")),
        "a restart should not have been answered with the quit warning"
    );

    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// Quitting takes every shell in the window with it, so the first ⌘Q says what it is about to
/// end and the second one goes through.
#[test]
fn quitting_with_a_command_running_asks_first() {
    let fixture = seeded_fixture("quit-warning");
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

    // A shell with a command running in it, which is the work a quit would interrupt.
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
    // Written before the shell has finished starting, which is fine: the pty holds the line
    // until the shell reads it, and then the command runs for as long as the test needs.
    pane.send(b"sleep 300\n")
        .expect("expected the shell to take the command");

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

    let warnings = Arc::new(Mutex::new(Vec::new()));
    let warnings_in_ui = Arc::clone(&warnings);
    // A toast stays up for seconds, so the first warning is wiped before the second quit -
    // otherwise what is on screen afterwards says nothing about which quit put it there.
    let wipe = Arc::new(AtomicBool::new(false));
    let wipe_in_ui = Arc::clone(&wipe);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .build_ui(move |ui| {
            if wipe_in_ui.swap(false, Ordering::Relaxed) {
                app.model.toasts.clear();
            }
            app.draw(ui);
            // Both halves of what the warning needs: an open window to draw it, and a command
            // running in a shell for it to be about.
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.shells_running_a_command() == 1,
                Ordering::Relaxed,
            );
            *warnings_in_ui.lock().expect("poisoned") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
        });

    // The session opens on a worker thread, and a window still on its opening screen draws
    // nothing that could answer a quit: the warning is the open window's.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the window should have opened on the review, with a command running in its shell"
    );
    harness.run_steps(3);

    let close_requested = |harness: &mut Harness<'_>| {
        harness
            .input_mut()
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("expected the root viewport")
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
    };
    let warned_about_the_shell = |warnings: &Arc<Mutex<Vec<String>>>| {
        warnings
            .lock()
            .expect("poisoned")
            .iter()
            .any(|text| text.contains("still running"))
    };

    close_requested(&mut harness);
    assert!(
        warned_about_the_shell(&warnings),
        "the first quit should have said the shell is still running"
    );

    // The second one is the answer to that question, and says nothing new.
    wipe.store(true, Ordering::Relaxed);
    harness.step();
    close_requested(&mut harness);
    assert!(
        !warned_about_the_shell(&warnings),
        "the second quit should have gone through rather than asking again"
    );

    // The window would have taken the shell with it; the test has to do it by hand.
    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// A shell waiting at its prompt has nothing to interrupt, so quitting with one open goes
/// through rather than asking about it: the warning is about work, not about open tabs.
#[test]
fn quitting_with_a_shell_at_its_prompt_does_not_ask() {
    let fixture = seeded_fixture("quit-idle-shell");
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

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let idle = Arc::new(AtomicBool::new(false));
    let idle_in_ui = Arc::clone(&idle);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .build_ui(move |ui| {
            app.draw(ui);
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.running_shells() == 1,
                Ordering::Relaxed,
            );
            idle_in_ui.store(app.shells_running_a_command() == 0, Ordering::Relaxed);
        });

    // The shell is started and then left alone, so what has to be waited out is its prompt
    // arriving: until bash is up, the pty's foreground group is not yet the shell's own.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline
        && !(ready.load(Ordering::Relaxed) && idle.load(Ordering::Relaxed))
    {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the window should have opened on the review, with its shell open"
    );
    assert!(
        idle.load(Ordering::Relaxed),
        "a shell sitting at its prompt should not read as running a command"
    );
    harness.run_steps(3);

    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("expected the root viewport")
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();

    assert!(
        !asked_to_stay_open(&harness),
        "quitting with an idle shell should not have been held back to ask about it"
    );

    crate::backend::Backend::close_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// The + button on a frame showing a review starts the shell in that review's repo; a frame
/// showing no review falls back to wherever the last shell started, then to the review the
/// window was launched on.
#[test]
fn a_new_shell_starts_in_the_review_shown_by_its_frame() {
    let fixture = seeded_fixture("shell-session");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.model.root_session_id = "root".to_string();

    let frame = app.model.layout.primary_frame();
    app.model.layout.add_pane(
        frame,
        Pane::Review {
            session_id: "submodule".to_string(),
            title: "submodule".to_string(),
        },
        None,
    );
    assert_eq!(app.shell_session_for(frame), "submodule");

    // The board has no review of its own, so the frame it fronts uses the last shell's.
    app.model.layout.add_pane(frame, Pane::Tasks, None);
    app.model.last_shell_session_id = Some("submodule".to_string());
    assert_eq!(app.shell_session_for(frame), "submodule");

    // And before any shell has started, the review the window was launched on.
    app.model.last_shell_session_id = None;
    assert_eq!(app.shell_session_for(frame), "root");
}

/// The terminal pane, end to end: a real pty, Ghostty's VT parser, and the grid the pane
/// paints from. A shell prompt differs per machine, so this asserts on output it asked for
/// rather than snapshotting the image.
#[test]
fn a_terminal_pane_runs_a_shell_and_shows_its_output() {
    let fixture = seeded_fixture("terminal");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = LocalBackend::new(state);

    let opened = crate::backend::Backend::open_session(
        &backend,
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");

    let terminal_id = crate::backend::Backend::create_terminal(&backend, &opened.session_id, None)
        .expect("expected a shell to start");
    let attachment =
        crate::backend::Backend::attach_terminal(&backend, &opened.session_id, &terminal_id)
            .expect("expected to attach to the shell");

    let mut pane =
        egui_tty::Terminal::new(attachment).expect("expected the terminal emulator to start");

    // A login shell prints a prompt first; the marker is what this waits for.
    pane.send(b"printf 'moonreview-ok\\n'\n")
        .expect("expected to write to the shell");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut screen = String::new();
    while Instant::now() < deadline {
        pane.poll();
        screen = pane.visible_text().expect("expected to read the grid");
        // Twice: once as the echoed command, once as its output.
        if screen.matches("moonreview-ok").count() >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        screen.contains("moonreview-ok"),
        "the shell's output never reached the terminal grid; screen was:\n{screen}"
    );
    assert!(!pane.has_exited(), "the shell should still be running");

    crate::backend::Backend::close_terminal(&backend, &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}

/// The glyphs command line tools animate and decorate with. egui's bundled fonts have none
/// of them, which is why a spinner in a shell was a row of empty boxes until the window
/// started borrowing a font off the machine it runs on.
const SHELL_GLYPHS: &str = concat!(
    "\u{280B}\u{2819}\u{2839}\u{2838}\u{283C}\u{2834}\u{2826}\u{2827}\u{2807}\u{280F}", // the braille spinner
    "\u{28FE}\u{28FD}\u{28FB}\u{28BF}\u{287F}\u{28DF}\u{28EF}\u{28F7}", // and the fuller one
    "\u{2714}\u{2716}\u{26A1}\u{23F3}\u{231B}\u{1F504}", // tick, cross, bolt, hourglasses, refresh
    "\u{1F311}\u{1F312}\u{1F313}\u{1F314}\u{1F315}",     // the moon phases some tools spin
);

#[test]
fn a_shell_can_draw_the_glyphs_its_tools_animate_with() {
    let mut harness = Harness::builder().build_ui(|_ui| {});
    harness.run();

    let borrowed = crate::native::fonts::install(&harness.ctx);
    assert!(
        !borrowed.is_empty(),
        "no system font was found to borrow from; the list in native::fonts needs this platform"
    );
    harness.run();

    let mut missing = String::new();
    harness.ctx.fonts_mut(|fonts| {
        let font = egui::FontId::monospace(crate::native::theme::CODE_SIZE);
        for glyph in SHELL_GLYPHS.chars() {
            if !fonts.has_glyph(&font, glyph) && !missing.contains(glyph) {
                missing.push(glyph);
            }
        }
    });

    assert!(
        missing.is_empty(),
        "these would render as empty boxes in a shell: {missing:?}"
    );
}

/// The bold and italic faces have to keep step with the regular one: the terminal's grid is
/// one advance of the regular face wide per cell, and the file pane paints its line numbers
/// where the laid-out rows fell. A face a hair wider or taller would put a bold word out of
/// its columns and the numbers off their lines.
#[test]
fn every_code_face_has_the_same_advance_and_line_height() {
    let mut harness = Harness::builder().build_ui(|_ui| {});
    harness.run();
    crate::native::fonts::install(&harness.ctx);
    harness.run();

    let regular = crate::native::theme::code_font(crate::native::theme::CodeFace::Regular);
    let mut out_of_step = Vec::new();
    harness.ctx.fonts_mut(|fonts| {
        for face in [
            crate::native::theme::CodeFace::Bold,
            crate::native::theme::CodeFace::Italic,
        ] {
            let font = crate::native::theme::code_font(face);
            if fonts.row_height(&font) != fonts.row_height(&regular) {
                out_of_step.push(format!("{face:?} line height"));
            }
            for glyph in ('!'..='~').chain(SHELL_GLYPHS.chars()) {
                if fonts.glyph_width(&font, glyph) != fonts.glyph_width(&regular, glyph) {
                    out_of_step.push(format!("{face:?} {glyph:?}"));
                }
            }
        }
    });
    assert!(
        out_of_step.is_empty(),
        "these are laid out differently from the regular face: {out_of_step:?}"
    );
}

/// The three faces, set one over the other: the bold and italic have to be real faces, and
/// the columns have to line up down the page. A snapshot, so the faces are looked at rather
/// than only measured.
#[test]
fn code_is_set_in_a_real_bold_and_a_real_italic() {
    // The faces are installed from inside the first pass, the way the app installs them,
    // and land at the start of the next; that first pass is discarded, as the app's is.
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(420.0, 96.0))
        .build_ui(move |ui| {
            if !installed {
                crate::native::fonts::install(ui.ctx());
                installed = true;
                ui.ctx().request_discard("fonts installed");
                return;
            }
            let ink = egui::Color32::WHITE;
            let mut job = egui::text::LayoutJob::default();
            for (face, line) in [
                (
                    crate::native::theme::CodeFace::Regular,
                    "fn main() { let quick = brown(fox); } // regular\n",
                ),
                (
                    crate::native::theme::CodeFace::Bold,
                    "fn main() { let quick = brown(fox); } // bold\n",
                ),
                (
                    crate::native::theme::CodeFace::Italic,
                    "fn main() { let quick = brown(fox); } // italic\n",
                ),
            ] {
                job.append(
                    line,
                    0.0,
                    egui::TextFormat::simple(crate::native::theme::code_font(face), ink),
                );
            }
            job.append(
                "\u{2502} \u{2500}\u{2500} \u{2588}\u{2588} \u{28fe} in every face",
                0.0,
                egui::TextFormat::simple(
                    crate::native::theme::code_font(crate::native::theme::CodeFace::Bold),
                    ink,
                ),
            );
            ui.label(job);
        });
    harness.run();
    harness.snapshot("code-faces");
}

/// A bold run draws its tables and spinners from the same borrowed font a regular one does.
#[test]
fn a_bold_or_italic_run_can_still_draw_the_glyphs_its_tools_animate_with() {
    let mut harness = Harness::builder().build_ui(|_ui| {});
    harness.run();
    crate::native::fonts::install(&harness.ctx);
    harness.run();

    let mut missing = String::new();
    harness.ctx.fonts_mut(|fonts| {
        for face in [
            crate::native::theme::CodeFace::Bold,
            crate::native::theme::CodeFace::Italic,
        ] {
            let font = crate::native::theme::code_font(face);
            for glyph in SHELL_GLYPHS.chars() {
                if !fonts.has_glyph(&font, glyph) && !missing.contains(glyph) {
                    missing.push(glyph);
                }
            }
        }
    });
    assert!(
        missing.is_empty(),
        "these would render as empty boxes in a bold or italic run: {missing:?}"
    );
}

/// Switching the theme to light and back must leave a shell readable. It did not: the colours
/// the pane paints with came back identical, so every line was text the colour of its own
/// background.
#[test]
fn a_shell_stays_readable_across_a_theme_round_trip() {
    let fixture = seeded_fixture("terminal-theme");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = LocalBackend::new(state);
    let opened = crate::backend::Backend::open_session(
        &backend,
        OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("expected the session to open");
    let terminal_id = crate::backend::Backend::create_terminal(&backend, &opened.session_id, None)
        .expect("expected a shell to start");
    let attachment =
        crate::backend::Backend::attach_terminal(&backend, &opened.session_id, &terminal_id)
            .expect("expected to attach to the shell");
    let mut pane =
        egui_tty::Terminal::new(attachment).expect("expected the terminal emulator to start");

    pane.set_color_scheme(egui_tty::ColorScheme::Dark);
    let dark = pane.drawn_colors().expect("expected the shell's colors");
    assert_ne!(dark.0, dark.1, "a fresh dark shell is readable");

    pane.set_color_scheme(egui_tty::ColorScheme::Light);
    let light = pane.drawn_colors().expect("expected the shell's colors");
    assert_ne!(light.0, light.1, "and so is a light one");

    pane.set_color_scheme(egui_tty::ColorScheme::Dark);
    let back = pane.drawn_colors().expect("expected the shell's colors");
    assert_ne!(
        back.0, back.1,
        "text and background must not come back as one colour"
    );
    assert_eq!(back, dark, "dark has to look the way it did before");

    crate::backend::Backend::close_terminal(&backend, &opened.session_id, &terminal_id)
        .expect("expected the shell to close");
}
