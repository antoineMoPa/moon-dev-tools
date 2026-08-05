//! Renders the real window offscreen and checks what came out.
//!
//! These drive [`App::draw`] through `egui_kittest`, which runs the same egui passes and the
//! same wgpu renderer the window uses. That makes it possible to assert on what the review
//! actually looks like — a diff that fails to draw, or an empty pane, shows up here.

use std::{
    fs,
    path::{Path, PathBuf},
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
    git::run_git_no_output,
    native::{Launch, app::App, theme::ThemeMode},
};

/// A throwaway git repo with a commit and some uncommitted work, which is the situation
/// moonreview exists for.
struct Fixture {
    root: PathBuf,
}

/// A fixed point in time, so a fixture commit always hashes to the same sha.
///
/// The review shows short shas and the repo's own name, and both end up in the snapshots. A
/// timestamp or a process id in either would make every run differ from the last.
const FIXTURE_DATE: &str = "2024-01-02T03:04:05+00:00";

impl Fixture {
    fn new(name: &str) -> Self {
        // The repo's directory name is what the header shows, so it is fixed; only the
        // enclosing directory carries what makes this run unique.
        let enclosing = std::env::temp_dir()
            .join(format!("moonreview-ui-{}-{name}", std::process::id()));
        let root = enclosing.join("repo");
        let _ = fs::remove_dir_all(&enclosing);
        fs::create_dir_all(&root).expect("failed to create the fixture directory");

        run_git_no_output(&root, &["init"]).expect("failed to init the fixture repo");
        for (key, value) in [
            ("user.email", "test@example.com"),
            ("user.name", "Test User"),
            ("commit.gpgsign", "false"),
        ] {
            run_git_no_output(&root, &["config", key, value]).expect("failed to configure git");
        }

        Self { root }
    }

    /// The directory to clean up: the fixture owns its enclosing directory too.
    fn enclosing(&self) -> &Path {
        self.root.parent().unwrap_or(&self.root)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create the fixture subdirectory");
        }
        fs::write(path, contents).expect("failed to write the fixture file");
    }

    fn commit(&self, message: &str) {
        run_git_no_output(&self.root, &["add", "-A"]).expect("failed to stage the fixture");

        // Committed with a fixed identity and date, so the short sha in the snapshot is the
        // same on every run and on every machine.
        let status = std::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.root)
            .env("GIT_AUTHOR_NAME", "Test User")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_AUTHOR_DATE", FIXTURE_DATE)
            .env("GIT_COMMITTER_NAME", "Test User")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_DATE", FIXTURE_DATE)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("failed to run git commit");
        assert!(status.success(), "failed to commit the fixture");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.enclosing());
    }
}

/// Build the window over a local backend, with no web server: the tests are about the UI.
fn app_for(repo_path: &Path, theme: ThemeMode) -> App {
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let launch = Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        serves_web: false,
    };

    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(theme);
    app
}

/// Drive frames until the review has loaded, then a few more so it is drawn.
///
/// The review is fetched on a worker thread, so a fixed number of frames would be a race:
/// the harness has to keep stepping until the data is actually in the model.
fn harness_with_loaded_review(app: App, theme: ThemeMode) -> Harness<'static> {
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .with_theme(match theme {
            ThemeMode::Dark => egui::Theme::Dark,
            ThemeMode::Light => egui::Theme::Light,
        })
        .wgpu()
        .build_ui(move |ui| {
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
    assert!(
        ready.load(Ordering::Relaxed),
        "the review never finished loading"
    );

    // A couple more passes so the freshly arrived diff is laid out and painted.
    harness.run_steps(3);
    harness
}

fn seeded_fixture(name: &str) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n\npub fn total(values: &[u32]) -> u32 {\n    values.iter().sum()\n}\n",
    );
    fixture.write("README.md", "# fixture\n\nA repo that exists to be reviewed.\n");
    fixture.commit("Add the library");

    // Uncommitted work: an edited line, a new line, and a whole new file.
    fixture.write(
        "src/lib.rs",
        "pub fn greet(person: &str) -> String {\n    format!(\"hello {person}\")\n}\n\npub fn total(values: &[u32]) -> u32 {\n    values.iter().copied().sum()\n}\n\npub fn count(values: &[u32]) -> usize {\n    values.len()\n}\n",
    );
    fixture.write("src/extra.rs", "pub const ANSWER: u32 = 42;\n");
    fixture
}

#[test]
fn the_review_window_draws_the_diff_it_was_opened_on() {
    let fixture = seeded_fixture("review");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);

    // The window is one image; if the diff failed to draw, this is where it shows.
    harness.snapshot("review-dark");
}

#[test]
fn the_review_window_draws_in_the_light_theme_too() {
    let fixture = seeded_fixture("review-light");
    let app = app_for(&fixture.root, ThemeMode::Light);

    let mut harness = harness_with_loaded_review(app, ThemeMode::Light);

    harness.snapshot("review-light");
}

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

/// Every glyph the chrome draws, so a missing one cannot ship as a `□` box.
///
/// egui's bundled fonts cover a small icon set and nothing more: sun, moon, arrow and tick
/// characters are all absent. Anything not in here has to be drawn or spelled out.
const CHROME_GLYPHS: &str = concat!(
    "\u{23F5}\u{23F7}", // collapse arrows
    "\u{1F5D9}",         // close
    "+",                  // open a pane
    "\u{00B7}\u{2212}", // separator, minus sign
);

#[test]
fn every_glyph_the_chrome_draws_is_in_the_bundled_fonts() {
    let mut harness = Harness::builder().build_ui(|_ui| {});
    harness.run();

    let mut missing = String::new();
    harness.ctx.fonts_mut(|fonts| {
        for size in [
            crate::native::theme::SMALL_SIZE - 2.0,
            crate::native::theme::UI_SIZE,
            crate::native::theme::CODE_SIZE,
        ] {
            for glyph in CHROME_GLYPHS.chars() {
                let font = egui::FontId::proportional(size);
                if !fonts.has_glyph(&font, glyph) && !missing.contains(glyph) {
                    missing.push(glyph);
                }
            }
        }
    });

    assert!(
        missing.is_empty(),
        "these glyphs would render as empty boxes: {missing:?}"
    );
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

    let mut pane = crate::native::terminal::TerminalPane::new(terminal_id, attachment)
        .expect("expected the terminal emulator to start");

    // A login shell prints a prompt first; the marker is what this waits for.
    pane.send(b"printf 'moonreview-ok\\n'\n")
        .expect("expected to write to the shell");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut screen = String::new();
    while Instant::now() < deadline {
        pane.pump();
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

    crate::backend::Backend::close_terminal(&backend, &opened.session_id, pane.terminal_id.as_str())
        .expect("expected the shell to close");
}

/// Clicks have to reach the widgets inside a frame's body.
///
/// This exists because of a real regression: a click-sensing widget the size of each frame,
/// registered after its contents, sat on top of everything and swallowed every click — the
/// window looked completely inert except for the split handles.
#[test]
fn a_click_reaches_a_widget_inside_a_frame() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("clickable");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let tab = Arc::new(Mutex::new(None::<&'static str>));
    let tab_in_ui = Arc::clone(&tab);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            if let Ok(mut tab) = tab_in_ui.lock() {
                *tab = app
                    .model
                    .review_ref(&app.model.root_session_id)
                    .map(|review| match review.sidebar_tab {
                        crate::native::model::SidebarTab::Files => "files",
                        crate::native::model::SidebarTab::Comments => "comments",
                    });
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

    assert_eq!(
        *tab.lock().expect("expected the tab"),
        Some("files"),
        "the sidebar starts on the files tab"
    );

    // The sidebar's tab buttons sit deep inside the frame body, which is what the swallowing
    // overlay used to cover.
    harness.get_by_label("comments").click();
    harness.run_steps(2);

    assert_eq!(
        *tab.lock().expect("expected the tab"),
        Some("comments"),
        "clicking the comments tab must switch the sidebar"
    );
}

/// Clicking a diff line selects it and opens the comment composer in one gesture, the way
/// selecting text does in the web frontend.
#[test]
fn clicking_a_diff_line_opens_the_comment_composer() {
    let fixture = seeded_fixture("comment");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    /// What the test needs to see from inside the UI closure.
    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selected_lines: usize,
        draft_selection: Option<String>,
        draft_is_focused: bool,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                seen.hunk_id = review.hunks().first().map(|hunk| hunk.id.clone());
                if let Some(hunk) = review.hunks().first() {
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.selected_lines = review
                    .selection
                    .map(|selection| selection.range().count())
                    .unwrap_or(0);
                seen.draft_selection = review.draft.as_ref().map(|draft| draft.selection.clone());
                seen.draft_is_focused = review
                    .draft
                    .as_ref()
                    .is_some_and(|draft| !draft.selection.is_empty());
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
            state
                .hunk_id
                .clone()
                .expect("expected a hunk to comment on"),
            state.patch.clone(),
        )
    };
    // Which patch line to click, and what it says, come from the same parse the review pane
    // uses — git's own header lines differ between versions and configurations.
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, expected) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.text.clone()))
        .expect("expected an added line to comment on");
    assert_eq!(
        seen.lock().expect("expected state").selected_lines,
        0,
        "nothing is selected before a click"
    );

    // Found by the same id the review pane drew the line with.
    let target = crate::native::review::hunks::diff_line_id(&hunk_id, line_index);
    let rect = harness
        .ctx
        .read_response(target)
        .expect("expected the diff line to have been drawn")
        .rect;
    click_at(&mut harness, rect.center());

    {
    let state = seen.lock().expect("expected state");
    assert_eq!(
        state.selected_lines, 1,
        "clicking a diff line selects exactly that line"
    );
    let selection = state
        .draft_selection
        .clone()
        .expect("clicking a line must open the composer");
    assert_eq!(
        selection, expected,
        "the comment must be anchored to the exact line that was clicked"
    );
    assert!(state.draft_is_focused, "the composer should be ready to type in");
    }

    // And the composer is on screen, not merely in the model.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("comment-composer");
}

/// Press and release the primary button at a position, then let the UI settle.
fn click_at(harness: &mut Harness<'_>, at: egui::Pos2) {
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();
    harness.run_steps(2);
}

/// Dragging down a hunk selects the whole run of lines the pointer swept over, and the
/// composer opens on that run once the button comes up.
#[test]
fn dragging_across_diff_lines_selects_the_run() {
    let fixture = seeded_fixture("multi-select");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selected: Option<(usize, usize)>,
        draft_selection: Option<String>,
    }

    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let Some(review) = app.model.review_ref(&app.model.root_session_id) else {
                return;
            };
            if let Ok(mut seen) = seen_in_ui.lock() {
                if let Some(hunk) = review.hunks().first() {
                    seen.hunk_id = Some(hunk.id.clone());
                    seen.patch = hunk.patch_preview.clone();
                }
                seen.selected = review
                    .selection
                    .map(|selection| (*selection.range().start(), *selection.range().end()));
                seen.draft_selection = review.draft.as_ref().map(|draft| draft.selection.clone());
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
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.kind.commentable())
        .map(|(index, _)| index)
        .collect();
    assert!(
        changed.len() >= 3,
        "the fixture needs a few lines to sweep over"
    );
    let (from, to) = (changed[0], changed[2]);

    let rect_of = |harness: &Harness<'_>, index: usize| {
        harness
            .ctx
            .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, index))
            .expect("expected the diff line to have been drawn")
            .rect
    };
    let start = rect_of(&harness, from).center();
    let end = rect_of(&harness, to).center();

    // Press on the first line, sweep to the third, release.
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();
    for at in [start + egui::vec2(0.0, 6.0), end] {
        harness.input_mut().events.push(egui::Event::PointerMoved(at));
        harness.step();
    }
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);

    let state = seen.lock().expect("expected state");
    assert_eq!(
        state.selected,
        Some((from, to)),
        "the sweep should select every line from the first to the last"
    );
    let selection = state
        .draft_selection
        .clone()
        .expect("the composer should open on the swept run");
    assert_eq!(
        selection.lines().count(),
        to - from + 1,
        "the comment is anchored to every swept line, got {selection:?}"
    );
    // The anchor text is raw patch lines, which is what a partial stage matches against.
    let expected: Vec<&str> = (from..=to).map(|index| lines[index].text.as_str()).collect();
    assert_eq!(selection, expected.join("\n"));
    drop(state);

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("multi-line-selection");
}

/// The comment dispatch contract, which the header and the composer both depend on.
///
/// `batch: false` hands the comment to the agent there and then; `batch: true` holds it back
/// so a batch can go at once. The batch send only moves what is actually held — which is why
/// the header must count held comments and nothing else.
#[test]
fn a_held_comment_is_what_the_batch_send_moves() {
    use crate::api::{CommentDispatchStatus, CommentRequest};

    let fixture = seeded_fixture("dispatch");
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
    let session_id = opened.session_id;

    let hunk = crate::backend::Backend::session_state(&backend, &session_id)
        .expect("expected the review state")
        .hunks
        .first()
        .cloned()
        .expect("expected a hunk to comment on");
    let anchor = crate::native::review::diff::build_diff_lines(&hunk.patch_preview)
        .into_iter()
        .find(|line| line.kind.commentable())
        .expect("expected a line to anchor to")
        .text;

    let comment = crate::comments::build_anchored_comment_value(&[
        crate::comments::AnchoredComment {
            selection: anchor,
            comment: "this needs a second look".to_string(),
            resolved: false,
        },
    ]);

    // Held back, with no agent picked: nothing may be dispatched yet.
    crate::backend::Backend::set_comment(
        &backend,
        &session_id,
        CommentRequest {
            hunk_id: hunk.id.clone(),
            comment: comment.clone(),
            batch: true,
        },
    )
    .expect("expected the comment to be stored");

    let held = crate::backend::Backend::session_state(&backend, &session_id)
        .expect("expected the review state");
    assert_eq!(held.review_comments.len(), 1, "the comment should be stored");
    assert_eq!(
        held.review_comments[0].dispatch.status,
        CommentDispatchStatus::Batched,
        "a held comment waits for the batch send"
    );

    // The batch send needs an agent, and says so rather than doing nothing.
    let refused = crate::backend::Backend::send_comment_batch(&backend, &session_id)
        .expect_err("expected the send to be refused without an agent");
    assert!(
        refused.to_string().contains("select an agent"),
        "the refusal should say what is missing: {refused}"
    );
    assert_eq!(
        crate::backend::Backend::session_state(&backend, &session_id)
            .expect("expected the review state")
            .review_comments[0]
            .dispatch
            .status,
        CommentDispatchStatus::Batched,
        "a refused send leaves the comment held"
    );
}
