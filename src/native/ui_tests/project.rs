//! The project pane: the two commands the Project menu runs and how the repo's files are
//! indented, typed and picked into the repo's own file.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    assert!(
        !project_file.exists(),
        "the fixture starts with no project file"
    );

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

    // The box that was typed in has the keyboard, and a blinking caret is a picture that
    // depends on which frame it was taken on. Held on, it is the same caret every run.
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
    });
    harness.run_steps(2);
    harness.snapshot("project-settings");

    // Act: pick an indentation, which is written the moment it is picked rather than on a
    // button of its own.
    harness.get_by_label("8 spaces").click();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut text = String::new();
    while Instant::now() < deadline {
        harness.step();
        text = std::fs::read_to_string(&project_file).unwrap_or_default();
        if text.contains("\"spaces\": 8") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        text.contains("\"spaces\": 8"),
        "the project file should hold the indentation that was picked, got {text:?}"
    );

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

/// What the reuse test's project builds with: a line short enough that the shell's echo of it
/// is on one row of the pane, so counting the rows it is on counts the times it was run.
const REUSED_BUILD_COMMAND: &str = "printf mooned";

/// A second build goes into the shell the first one ran in. A build asked for over and over
/// is how a window fills with shells otherwise: one per press, each holding a prompt nobody
/// typed at again.
#[test]
fn a_second_build_is_typed_into_the_first_builds_shell() {
    // Arrange: a repo whose build command says, in the shell it runs in, that it ran.
    let fixture = seeded_fixture("project-build-shell");
    crate::project::write_project(
        &fixture.root,
        &crate::project::ProjectConfig::typed(
            REUSED_BUILD_COMMAND,
            "",
            egui_moon_editor::Indent::default(),
        ),
    )
    .expect("expected the project file to be written");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    // Raised by the test to ask for a build, the way the palette and the menu bar ask.
    let run_build = Arc::new(AtomicBool::new(false));
    let run_build_in_ui = Arc::clone(&run_build);
    // Set once the window has read the project file and has a build to run.
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // How many shell tabs are open, and how many times the build's line is on the screen of
    // the first of them - once per run, in the shell's echo of what it was sent.
    let shells = Arc::new(AtomicUsize::new(0));
    let shells_in_ui = Arc::clone(&shells);
    let runs = Arc::new(AtomicUsize::new(0));
    let runs_in_ui = Arc::clone(&runs);
    // Set while the server says a shell has something running in it: a busy shell is not one
    // to type the next build into, so the second press waits for the first to be done.
    let busy = Arc::new(AtomicBool::new(true));
    let busy_in_ui = Arc::clone(&busy);
    // What that shell's tab reads, which is the name the server gave it.
    let tab_title = Arc::new(Mutex::new(String::new()));
    let tab_title_in_ui = Arc::clone(&tab_title);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if run_build_in_ui.swap(false, Ordering::Relaxed) {
                app.pending_action = Some(CommandAction::RunProject(ProjectCommand::Build));
            }
            app.draw(ui);
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.model.project.build.is_some(),
                Ordering::Relaxed,
            );
            shells_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .filter(|(_, pane)| pane.kind() == PaneKind::Terminal)
                    .count(),
                Ordering::Relaxed,
            );
            busy_in_ui.store(
                !app.model.shells_running_a_command.is_empty(),
                Ordering::Relaxed,
            );
            let screen = app
                .terminals
                .values_mut()
                .next()
                .and_then(|terminal| terminal.visible_text().ok())
                .unwrap_or_default();
            runs_in_ui.store(
                screen.matches(REUSED_BUILD_COMMAND).count(),
                Ordering::Relaxed,
            );
            if let Some((_, pane)) = app
                .model
                .layout
                .find_pane(|pane| pane.kind() == PaneKind::Terminal)
            {
                let title = app.shell_tab_title(pane);
                *tab_title_in_ui.lock().expect("poisoned") = title;
            }
        });

    let settle_until = |harness: &mut Harness<'_>, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };

    assert!(
        settle_until(&mut harness, &|| ready.load(Ordering::Relaxed)),
        "the window never opened on the project"
    );

    // Act: the first build, which opens the shell it runs in.
    run_build.store(true, Ordering::Relaxed);
    assert!(
        settle_until(&mut harness, &|| runs.load(Ordering::Relaxed) == 1
            && !busy.load(Ordering::Relaxed)),
        "the build should have run in a shell of its own and finished there"
    );
    assert_eq!(shells.load(Ordering::Relaxed), 1, "one shell so far");
    assert!(
        settle_until(&mut harness, &|| tab_title
            .lock()
            .expect("poisoned")
            .as_str()
            == crate::project::PROJECT_SHELL_NAME),
        "the shell the project's commands run in should be named {:?}, and its tab read {:?}",
        crate::project::PROJECT_SHELL_NAME,
        tab_title.lock().expect("poisoned"),
    );

    // Act: the same build again, with that shell open and waiting at its prompt.
    run_build.store(true, Ordering::Relaxed);
    assert!(
        settle_until(&mut harness, &|| runs.load(Ordering::Relaxed) == 2),
        "the second build should have been typed into the shell the first one ran in"
    );

    // Assert: it went into the shell that was already open rather than beside it.
    harness.run_steps(3);
    assert_eq!(
        shells.load(Ordering::Relaxed),
        1,
        "the second build should have opened no second shell"
    );
}

/// A build shell that ended behind another tab is not one to type the next build into. The
/// window only ever heard a shell had ended from drawing it, so one that ended out of sight
/// went on reading as open and waiting: the next build was typed at a dead pty, and failed
/// with the pty's own `Input/output error`.
#[test]
fn a_build_shell_that_ended_behind_another_tab_is_not_typed_into() {
    // Arrange: a repo with a build, as the reuse test has it.
    let fixture = seeded_fixture("project-build-shell-ended-hidden");
    crate::project::write_project(
        &fixture.root,
        &crate::project::ProjectConfig::typed(
            REUSED_BUILD_COMMAND,
            "",
            egui_moon_editor::Indent::default(),
        ),
    )
    .expect("expected the project file to be written");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let run_build = Arc::new(AtomicBool::new(false));
    let run_build_in_ui = Arc::clone(&run_build);
    // Raised to open a second shell as a tab over the build's, which takes it out of sight.
    let cover_build_shell = Arc::new(AtomicBool::new(false));
    let cover_build_shell_in_ui = Arc::clone(&cover_build_shell);
    // Raised to end the build's shell, the way `exit` typed at its prompt does.
    let end_build_shell = Arc::new(AtomicBool::new(false));
    let end_build_shell_in_ui = Arc::clone(&end_build_shell);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let busy = Arc::new(AtomicBool::new(true));
    let busy_in_ui = Arc::clone(&busy);
    // The shells with a tab, by the id the server gave each.
    let shell_ids = Arc::new(Mutex::new(Vec::<String>::new()));
    let shell_ids_in_ui = Arc::clone(&shell_ids);
    let build_shell = Arc::new(Mutex::new(None::<String>));
    let build_shell_in_ui = Arc::clone(&build_shell);
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let errors_in_ui = Arc::clone(&errors);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if run_build_in_ui.swap(false, Ordering::Relaxed) {
                app.pending_action = Some(CommandAction::RunProject(ProjectCommand::Build));
            }
            if cover_build_shell_in_ui.swap(false, Ordering::Relaxed) {
                let (build_pane, _) = app
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Terminal)
                    .expect("expected the build's shell to have a tab");
                let frame = app
                    .model
                    .layout
                    .frame_of(build_pane)
                    .expect("expected that tab to be in a frame");
                app.spawn_terminal(
                    app.model.root_session_id.clone(),
                    None,
                    crate::native::workspace::TerminalPlacement::Tab(frame),
                );
            }
            if end_build_shell_in_ui.swap(false, Ordering::Relaxed) {
                let build_shell = app
                    .model
                    .project_shell
                    .clone()
                    .expect("expected the window to remember the build's shell");
                app.terminals[&build_shell]
                    .send(b"exit\r")
                    .expect("expected the build's shell to take the line");
            }
            app.draw(ui);
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.model.project.build.is_some(),
                Ordering::Relaxed,
            );
            busy_in_ui.store(
                !app.model.shells_running_a_command.is_empty(),
                Ordering::Relaxed,
            );
            *shell_ids_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::Terminal { terminal_id, .. } => {
                        Some(terminal_id.clone())
                    }
                    _ => None,
                })
                .collect();
            *build_shell_in_ui.lock().expect("poisoned") = app.model.project_shell.clone();
            *errors_in_ui.lock().expect("poisoned") = app
                .model
                .messages
                .iter()
                .filter(|message| message.kind == crate::native::model::ToastKind::Error)
                .map(|message| message.text.clone())
                .collect();
        });

    let settle_until = |harness: &mut Harness<'_>, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    let open_shells = || shell_ids.lock().expect("poisoned").clone();

    assert!(
        settle_until(&mut harness, &|| ready.load(Ordering::Relaxed)),
        "the window never opened on the project"
    );
    run_build.store(true, Ordering::Relaxed);
    assert!(
        settle_until(&mut harness, &|| open_shells().len() == 1
            && build_shell.lock().expect("poisoned").is_some()
            && !busy.load(Ordering::Relaxed)),
        "the build should have run in a shell of its own and finished there"
    );
    let first_build_shell = build_shell
        .lock()
        .expect("poisoned")
        .clone()
        .expect("expected the build's shell");

    // Act: another shell's tab goes over the build's, and the build's shell ends out of sight.
    cover_build_shell.store(true, Ordering::Relaxed);
    assert!(
        settle_until(&mut harness, &|| open_shells().len() == 2),
        "the second shell should have opened as a tab"
    );
    end_build_shell.store(true, Ordering::Relaxed);

    // Assert: the window hears of it without the tab being looked at, and closes it.
    assert!(
        settle_until(&mut harness, &|| !open_shells().contains(&first_build_shell)),
        "a shell that ended behind another tab should have had its tab closed, and the open \
         shells were {:?}",
        open_shells(),
    );

    // Act: the next build.
    run_build.store(true, Ordering::Relaxed);

    // Assert: it opened a shell of its own rather than typing at the one that is gone.
    assert!(
        settle_until(&mut harness, &|| build_shell
            .lock()
            .expect("poisoned")
            .as_ref()
            .is_some_and(|shell| *shell != first_build_shell)),
        "the next build should have opened a shell of its own"
    );
    assert_eq!(
        *errors.lock().expect("poisoned"),
        Vec::<String>::new(),
        "and nothing should have gone wrong on the way"
    );
}

/// A build in a window with no room for another column takes a tab in the frame it was asked
/// from, rather than splitting off a column too narrow to read a build in.
#[test]
fn a_build_with_no_room_for_a_column_takes_a_tab() {
    let fixture = seeded_fixture("project-build-tab");
    crate::project::write_project(
        &fixture.root,
        &crate::project::ProjectConfig::typed(
            REUSED_BUILD_COMMAND,
            "",
            egui_moon_editor::Indent::default(),
        ),
    )
    .expect("expected the project file to be written");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let run_build = Arc::new(AtomicBool::new(false));
    let run_build_in_ui = Arc::clone(&run_build);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // The shape the build landed in: how many frames the workspace is in, and whether one of
    // them holds a shell.
    let frames = Arc::new(AtomicUsize::new(0));
    let frames_in_ui = Arc::clone(&frames);
    let has_shell = Arc::new(AtomicBool::new(false));
    let has_shell_in_ui = Arc::clone(&has_shell);

    // Narrow enough that a column down the right would leave both halves too cramped to work
    // in - see `fits_another_column`.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(880.0, 700.0))
        .wgpu()
        .build_ui(move |ui| {
            if run_build_in_ui.swap(false, Ordering::Relaxed) {
                app.pending_action = Some(CommandAction::RunProject(ProjectCommand::Build));
            }
            app.draw(ui);
            ready_in_ui.store(
                matches!(app.model.stage, crate::native::model::Stage::Ready)
                    && app.model.project.build.is_some(),
                Ordering::Relaxed,
            );
            frames_in_ui.store(app.model.layout.frame_count(), Ordering::Relaxed);
            has_shell_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .any(|(_, pane)| pane.kind() == PaneKind::Terminal),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready.load(Ordering::Relaxed),
        "the window never opened on the project"
    );
    let frames_before = frames.load(Ordering::Relaxed);

    run_build.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !has_shell.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        has_shell.load(Ordering::Relaxed),
        "the build should have opened a shell"
    );
    harness.run_steps(3);
    assert_eq!(
        frames.load(Ordering::Relaxed),
        frames_before,
        "the build's shell should have joined the tabs of a frame that was already there"
    );
}
