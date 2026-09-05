//! The command palette: what it lists, what it finds, and what it opens.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, click_at, press_key, seeded_fixture, settle, type_letter};

#[test]
fn the_command_palette_lists_what_can_be_opened() {
    let fixture = seeded_fixture("palette");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // Opening the palette is what this checks, so it is opened rather than typed for.
    let open_palette = Arc::new(AtomicBool::new(false));
    let open_in_ui = Arc::clone(&open_palette);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if open_in_ui.load(Ordering::Relaxed) {
                app.model.palette.open = true;
            }
            app.draw(ui);
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

    open_palette.store(true, Ordering::Relaxed);
    // The palette's search box has focus, and a blinking caret would make the image differ
    // from one run to the next.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);

    harness.snapshot("command-palette");
}

/// `split bottom` from the palette: the frame in two the short way, with a live shell in the
/// half that opened. `split right` is the same command against the other side.
#[test]
fn a_split_command_opens_a_shell_in_the_half_it_makes() {
    let fixture = seeded_fixture("palette-split");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let asked = Arc::new(AtomicBool::new(false));
    let asked_in_ui = Arc::clone(&asked);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_in_ui = Arc::clone(&stop);
    // Frames open, shells running, and whether the split runs the short way - all read after
    // the window has drawn, because that is when the palette's command is answered.
    let state = Arc::new(Mutex::new((0usize, 0usize, false)));
    let state_in_ui = Arc::clone(&state);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            // Once the review is up, ask for the split the way the palette does.
            if !asked_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.pending_action = Some(crate::native::palette::CommandAction::Split(
                    egui_frames::DropSide::Bottom,
                ));
                asked_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            if stop_in_ui.load(Ordering::Relaxed) {
                for terminal in app.terminals.values() {
                    let _ = terminal.send(b"exit\n");
                }
            }
            *state_in_ui.lock().expect("poisoned") = (
                app.model.layout.frame_count(),
                app.running_shells(),
                matches!(
                    app.model.layout.root(),
                    egui_frames::LayoutNode::Split {
                        direction: egui_frames::SplitDirection::Column,
                        ..
                    }
                ),
            );
        });

    let opened = settle(&mut harness, || {
        let state = state.lock().expect("poisoned");
        state.0 == 2 && state.1 == 1
    });
    let (frames, shells, column) = *state.lock().expect("poisoned");
    assert!(
        opened,
        "the split never arrived: {frames} frames, {shells} shells"
    );
    assert!(
        column,
        "`split bottom` should split the frame the short way"
    );

    // The shell is this window's to end, the way quitting would end it.
    stop.store(true, Ordering::Relaxed);
    let ended = settle(&mut harness, || state.lock().expect("poisoned").1 == 0);
    assert!(ended, "the shell the split opened never exited");
}

/// Typing puts the highlight back on the first match. The list changes under the highlight
/// with every keystroke, and before this the old row number was kept and pinned to the end of
/// the shorter list, so Enter ran the last match of what had been typed.
#[test]
fn typing_in_the_palette_highlights_the_first_match_again() {
    let fixture = seeded_fixture("palette-highlight");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // The highlighted row, and the command Enter would run.
    let highlight = Arc::new(Mutex::new((0usize, String::new())));
    let highlight_in_ui = Arc::clone(&highlight);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let matches = crate::native::palette::filter(
                crate::native::palette::commands_for(&app),
                &app.model.palette.query,
            );
            let title = matches
                .get(app.model.palette.highlighted)
                .map(|command| command.title.clone())
                .unwrap_or_default();
            *highlight_in_ui.lock().expect("poisoned") = (app.model.palette.highlighted, title);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(
        &mut harness,
        egui::Key::P,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    // Down the list a way, as reading it or running the pointer over it does.
    press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(
        highlight.lock().expect("poisoned").0,
        2,
        "the arrows should have moved the highlight down the list"
    );

    type_letter(&mut harness, egui::Key::S, "s");
    let (row, title) = highlight.lock().expect("poisoned").clone();
    assert_eq!(row, 0, "typing should put the highlight back at the top");
    assert_eq!(
        title, "comment agents",
        "the first match of what is typed is what Enter runs"
    );
}

/// cmd+P finds a file of the repo by name, wherever under the root it sits, and Enter opens
/// it in a tab. What the repo ignores is not a file of the repo: a build directory with the
/// same name in it stays off the list.
#[test]
fn the_palette_finds_a_file_by_name_and_opens_it() {
    let fixture = seeded_fixture("palette-files");
    fixture.write(".gitignore", "build/\n");
    fixture.write("build/extra.rs", "pub const GENERATED: u32 = 0;\n");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // What the finder is showing, and which files are open in tabs.
    let listed = Arc::new(Mutex::new((None::<String>, Vec::<String>::new())));
    let listed_in_ui = Arc::clone(&listed);
    let open_files = Arc::new(Mutex::new(Vec::<String>::new()));
    let open_in_ui = Arc::clone(&open_files);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *listed_in_ui.lock().expect("poisoned") = (
                app.model.palette.files.searched.clone(),
                app.model.palette.files.matches.clone(),
            );
            *open_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                })
                .collect();
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(&mut harness, egui::Key::P, egui::Modifiers::COMMAND);
    for (key, letter) in [
        (egui::Key::E, "e"),
        (egui::Key::X, "x"),
        (egui::Key::T, "t"),
        (egui::Key::R, "r"),
        (egui::Key::A, "a"),
    ] {
        type_letter(&mut harness, key, letter);
    }

    assert!(
        settle(&mut harness, || {
            listed.lock().expect("poisoned").0.as_deref() == Some("extra")
        }),
        "the finder never searched for what was typed - is ag installed?"
    );
    assert_eq!(
        listed.lock().expect("poisoned").1,
        vec!["src/extra.rs".to_string()],
        "the file below src should be the only match; the ignored one is not a file of the repo"
    );

    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || open_files
            .lock()
            .expect("poisoned")
            .contains(&"src/extra.rs".to_string())),
        "enter should have opened the highlighted file in a tab"
    );
}

/// The palette's content search finds the lines that hold what is typed, and running one
/// opens the file it is in.
#[test]
fn the_palette_searches_the_files_for_text_and_opens_a_match() {
    let fixture = seeded_fixture("palette-content");
    fixture.write(".gitignore", "build/\n");
    fixture.write("build/extra.rs", "pub const ANSWER: u32 = 0;\n");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // What the search is showing - the query it answered for, and the lines it found.
    let found = Arc::new(Mutex::new((
        None::<String>,
        Vec::<(String, usize, String)>::new(),
    )));
    let found_in_ui = Arc::clone(&found);
    let open_files = Arc::new(Mutex::new(Vec::<String>::new()));
    let open_in_ui = Arc::clone(&open_files);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *found_in_ui.lock().expect("poisoned") = (
                app.model.palette.contents.searched.clone(),
                app.model
                    .palette
                    .contents
                    .matches
                    .iter()
                    .map(|found| {
                        (
                            found.file_path.clone(),
                            found.line_number,
                            found.line.clone(),
                        )
                    })
                    .collect(),
            );
            *open_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes()
                .filter_map(|(_, pane)| match pane {
                    crate::native::panes::Pane::File { file_path, .. } => Some(file_path.clone()),
                    _ => None,
                })
                .collect();
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    for (key, letter) in [
        (egui::Key::A, "a"),
        (egui::Key::N, "n"),
        (egui::Key::S, "s"),
        (egui::Key::W, "w"),
        (egui::Key::E, "e"),
        (egui::Key::R, "r"),
    ] {
        type_letter(&mut harness, key, letter);
    }

    assert!(
        settle(&mut harness, || {
            found.lock().expect("poisoned").0.as_deref() == Some("answer")
        }),
        "the search never answered for what was typed - is ag installed?"
    );
    assert_eq!(
        found.lock().expect("poisoned").1,
        vec![(
            "src/extra.rs".to_string(),
            1,
            "pub const ANSWER: u32 = 42;".to_string()
        )],
        "the line of the file below src should be the only match; the ignored file is not part \
         of the repo, and the text is matched without regard for case"
    );

    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || open_files
            .lock()
            .expect("poisoned")
            .contains(&"src/extra.rs".to_string())),
        "enter should have opened the file the match is in"
    );
}

/// A letter typed into the palette's box is a letter, even one the review binds bare - and it
/// stays one after the cmd key has been tapped on its own, which the platform sends as a key
/// press like any other.
#[test]
fn letters_type_into_the_palette_after_a_bare_cmd_press() {
    let fixture = seeded_fixture("palette-typing");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let query = Arc::new(Mutex::new(String::new()));
    let query_in_ui = Arc::clone(&query);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *query_in_ui.lock().expect("poisoned") = app.model.palette.query.clone();
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(
        &mut harness,
        egui::Key::P,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    type_letter(&mut harness, egui::Key::S, "s");
    assert_eq!(
        *query.lock().expect("poisoned"),
        "s",
        "`s` in the palette's box is the letter s"
    );

    // Tap cmd and let it go, the way a hand resting on the keyboard does.
    press_key(&mut harness, egui::Key::SuperLeft, egui::Modifiers::COMMAND);
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::SuperLeft,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);

    type_letter(&mut harness, egui::Key::S, "s");
    assert_eq!(
        *query.lock().expect("poisoned"),
        "ss",
        "a tapped cmd must not take the letters with it"
    );
}

/// Clicking away from the palette - into the review under it, a shell, a tab - puts it away,
/// and the click lands on what was clicked rather than being spent on dismissing.
#[test]
fn clicking_outside_the_palette_puts_it_away() {
    let fixture = seeded_fixture("palette-click-away");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let open = Arc::new(AtomicBool::new(false));
    let open_in_ui = Arc::clone(&open);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            open_in_ui.store(app.model.palette.open, Ordering::Relaxed);
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(
        &mut harness,
        egui::Key::P,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    assert!(open.load(Ordering::Relaxed), "⌘⇧P should have opened it");

    // Well below the palette, which is anchored near the top: the review's own body.
    click_at(&mut harness, egui::pos2(600.0, 700.0));
    assert!(
        !open.load(Ordering::Relaxed),
        "a click outside the palette should have put it away"
    );
    // And the keyboard went with it, rather than being held by a box that is no longer there:
    // the click belongs to whatever was clicked, a shell included.
    assert!(
        harness.ctx.memory(|memory| memory.focused()).is_none(),
        "the palette's search box should not still have the keyboard"
    );
}

/// The commit command is aimed at the review being read rather than at the window's own: a
/// changed submodule is a review of its own repo, with its own branch to commit, so committing
/// while reading one means that repo. With neither in front - a shell, the board - it falls
/// back to the review the window was launched on.
#[test]
fn the_commit_command_commits_the_review_in_front() {
    use crate::native::{
        palette::{CommandAction, commands_for},
        panes::{OpenPaneRequest, Pane},
    };
    use egui_frames::Layout;

    const SUBMODULE: &str = "submodule-session";

    let fixture = seeded_fixture("palette-commit-target");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let root = app.model.root_session_id.clone();

    let would_commit = |app: &crate::native::app::App| match commands_for(app)
        .into_iter()
        .find(|command| command.title == "commit")
        .map(|command| command.action)
    {
        Some(CommandAction::OpenPane(OpenPaneRequest::Commit { session_id })) => session_id,
        _ => panic!("expected a commit command on the list"),
    };
    let review = |session_id: &str, title: &str| Pane::Review {
        session_id: session_id.to_string(),
        title: title.to_string(),
    };

    // The window as it stands with a submodule review opened beside the one it was launched on,
    // the submodule in front.
    app.model.layout = Layout::with_pane(review(&root, "review"));
    let frame = app.model.layout.active_frame();
    let root_review = app
        .model
        .layout
        .find_pane(|pane| pane.reviews(&root))
        .expect("expected the root review")
        .0;
    let submodule = app
        .model
        .layout
        .add_pane(frame, review(SUBMODULE, "submodule"), None);

    assert_eq!(
        would_commit(&app),
        SUBMODULE,
        "the submodule is in front, so it is the repo the command would commit"
    );

    // A file of that submodule's review is still that review being read.
    app.model.layout.add_pane(
        frame,
        Pane::File {
            session_id: SUBMODULE.to_string(),
            file_path: "src/lib.rs".to_string(),
            task_id: None,
        },
        None,
    );
    assert_eq!(
        would_commit(&app),
        SUBMODULE,
        "a file of the submodule is that submodule being read"
    );

    app.model.layout.focus_pane(root_review);
    assert_eq!(
        would_commit(&app),
        root,
        "and back on the window's own review it is that repo again"
    );

    // Nothing that belongs to a review in front: the window's own is the one that is meant.
    app.model.layout.close_pane(submodule);
    app.model.layout.close_pane(root_review);
    app.model.layout.add_pane(frame, Pane::Agents, None);
    assert_eq!(
        would_commit(&app),
        root,
        "with no review in front the command falls back to the window's own"
    );
}

/// A pane the workspace keeps one of stays on the list once it is open, and running it then
/// brings that pane forward rather than opening a second one.
#[test]
fn the_palette_still_offers_a_review_that_is_already_open() {
    let fixture = seeded_fixture("palette-open-review");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let seen = Arc::new(Mutex::new((0_usize, String::new())));
    let seen_in_ui = Arc::clone(&seen);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let root = app.model.root_session_id.clone();
            let listed = crate::native::palette::commands_for(&app);
            let described = listed
                .iter()
                .find(|command| command.title == "review")
                .map(|command| command.description.clone())
                .unwrap_or_default();
            let reviews = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| pane.reviews(&root))
                .count();
            *seen_in_ui.lock().expect("poisoned") = (reviews, described);
            ready_in_ui.store(
                app.model
                    .review_ref(&root)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    let (reviews, described) = seen.lock().expect("poisoned").clone();
    assert_eq!(reviews, 1, "the window opens on its review");
    assert_eq!(
        described, "Bring the repo review forward",
        "the open review stays on the list, named after the fixture's directory"
    );

    press_key(
        &mut harness,
        egui::Key::P,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    for letter in [
        (egui::Key::R, "r"),
        (egui::Key::E, "e"),
        (egui::Key::V, "v"),
    ] {
        type_letter(&mut harness, letter.0, letter.1);
    }
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    let (reviews, _) = seen.lock().expect("poisoned").clone();
    assert_eq!(
        reviews, 1,
        "running it again should raise the open review, not open a second one"
    );
}
