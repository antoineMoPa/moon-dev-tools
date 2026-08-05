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

    /// A solid PNG of the given color: the fixture's stand-in for a real picture.
    fn write_png(&self, relative: &str, color: [u8; 4]) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create the fixture subdirectory");
        }
        let picture = image::RgbaImage::from_pixel(24, 16, image::Rgba(color));
        picture.save(&path).expect("failed to write the fixture image");
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
    let attachment =
        crate::backend::Backend::attach_terminal(backend.as_ref(), &opened.session_id, &terminal_id)
            .expect("expected to attach to the shell");
    let mut pane = crate::native::terminal::TerminalPane::new(terminal_id.clone(), attachment)
        .expect("expected the terminal emulator to start");
    pane.send(b"exit\n").expect("expected to write to the shell");

    // The window is built around that shell: one review tab and one shell tab.
    let launch = Launch {
        backend: Arc::clone(&backend) as Arc<dyn crate::backend::Backend>,
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        serves_web: false,
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
                let frame_id = app.model.layout.active_frame_id.clone();
                let layout = std::mem::replace(
                    &mut app.model.layout,
                    crate::native::layout::empty_layout(),
                );
                app.model.layout = crate::native::layout::add_pane(
                    layout,
                    &frame_id,
                    crate::native::layout::Pane::Terminal {
                        pane_id: crate::native::layout::make_id("pane"),
                        terminal_id: for_pane.clone(),
                        command: None,
                    },
                    None,
                );
                placed_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            println!(
                "stage-ready={} placed={} exited={:?}",
                matches!(app.model.stage, crate::native::model::Stage::Ready),
                placed_in_ui.load(Ordering::Relaxed),
                app.terminals.values().map(|t| t.has_exited()).collect::<Vec<_>>(),
            );
            println!(
                "screen={:?}",
                app.terminals
                    .values_mut()
                    .next()
                    .and_then(|t| t.visible_text().ok())
                    .map(|text| text.trim().replace('\n', " | ").chars().take(120).collect::<String>()),
            );
            *panes_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .panes
                .values()
                .filter(|pane| {
                    matches!(pane, crate::native::layout::Pane::Terminal { terminal_id, .. }
                        if *terminal_id == for_pane)
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

/// Storage the window can be handed in a test, standing in for the one eframe keeps on disk.
#[derive(Default)]
struct RememberedStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for RememberedStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

/// The agent belongs to the person reviewing, not to a session that is new every launch, so it
/// is written out on the way down and asked for again on the way up.
#[test]
fn the_agent_the_last_run_ended_on_comes_back() {
    use eframe::{App as _, Storage as _};

    let fixture = seeded_fixture("agent-memory");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let mut storage = RememberedStorage::default();

    // A window that ends on Claude says so.
    let session_id = app.model.root_session_id.clone();
    let review = app.model.review(&session_id);
    review.payload = Some(Arc::new(crate::api::SessionPayload {
        repo_name: "repo".to_string(),
        branch_name: None,
        commit_base: None,
        commits: Vec::new(),
        history_commits: Vec::new(),
        history_has_more: false,
        local_change_summary: Default::default(),
        active_commit: None,
        repo_path: "/repo".to_string(),
        read_only: false,
        patch_preview_line_limit: 500,
        available_agents: Vec::new(),
        selected_agent: crate::api::AgentKind::Claude,
        full_file_path: None,
        hunks: Vec::new(),
        review_comments: Vec::new(),
        export_text: String::new(),
    }));
    app.save(&mut storage);

    assert_eq!(
        storage.get_string("moonreview-selected-agent").as_deref(),
        Some("\"claude\""),
        "the agent should have been written out"
    );

    // And the next one starts by asking for it back.
    let mut next = app_for(&fixture.root, ThemeMode::Dark);
    next.restore_layout_from(Some(&storage));

    assert!(
        next.model.restored_agent == Some(crate::api::AgentKind::Claude),
        "the stored agent should be waiting to be applied"
    );
}

/// A row of fixed height cannot grow, so text too long for it is cut rather than wrapped —
/// wrapped text is what used to run over the line below.
#[test]
fn a_long_commit_subject_is_cut_to_the_row_it_is_drawn_in() {
    let subject = "Rework the dispatch queue so held comments survive a restart,         and take the chance to rename everything around it while we are here";
    let laid_out = Arc::new(Mutex::new((0usize, false)));
    let in_ui = Arc::clone(&laid_out);
    let mut harness = Harness::builder().build_ui(move |ui| {
        let galley = crate::native::widgets::cut_to_fit(
            ui,
            subject,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
            220.0,
            1,
        );
        *in_ui.lock().expect("poisoned") = (galley.rows.len(), galley.elided);
    });
    harness.run();

    let (rows, elided) = *laid_out.lock().expect("poisoned");
    assert_eq!(rows, 1, "the subject should have been cut to one row");
    assert!(elided, "and the row should say it was cut short");
}

/// Pointed at a file nobody has touched, the review shows the file itself rather than an
/// empty diff — `moonreview package.json` is a request to read it.
#[test]
fn an_unchanged_file_opens_as_the_file_itself() {
    let fixture = Fixture::new("unchanged-file");
    fixture.write("package.json", "{\n  \"name\": \"fixture\"\n}\n");
    fixture.commit("Add the manifest");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let launch = Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: Some(crate::api::DiffTarget {
                base: None,
                pathspec: Some("package.json".to_string()),
                comparison: None,
            }),
            active_commit: None,
        }),
        serves_web: false,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);

    let shown = Arc::new(Mutex::new(None::<String>));
    let shown_in_ui = Arc::clone(&shown);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *shown_in_ui.lock().expect("poisoned") = app
                .model
                .review_ref(&app.model.root_session_id)
                .and_then(|review| review.full_file.as_ref())
                .and_then(|view| view.content.clone());
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if shown.lock().expect("poisoned").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let content = shown.lock().expect("poisoned").clone();
    assert_eq!(
        content.as_deref(),
        Some("{\n  \"name\": \"fixture\"\n}\n"),
        "the file's own text should be on screen"
    );
}

/// A changed image is shown as before and after pictures. They arrive as `data:` URIs, which
/// egui has no loader for, so this is where a decoding regression turns back into an error box.
#[test]
fn a_changed_image_is_drawn_as_before_and_after() {
    let fixture = Fixture::new("image-diff");
    fixture.write("README.md", "# fixture\n");
    fixture.write_png("logo.png", [200, 90, 40, 255]);
    fixture.commit("Add the logo");
    fixture.write_png("logo.png", [40, 120, 200, 255]);

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);
    // Decoding and uploading the texture takes a pass of its own after the diff arrives.
    harness.run_steps(3);

    harness.snapshot("image-diff");
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
    "+",                  // open a pane
    "\u{00B7}\u{2212}", // separator, minus sign
    "\u{2318}", // the command key — shift and return have no glyph, so they are spelled out
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

/// ⌘W is the window's own chord: it takes the tab in front, not the window around it.
#[test]
fn command_w_closes_the_tab_in_front() {
    let fixture = seeded_fixture("close-tab");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    let panes_left = Arc::new(Mutex::new(Vec::<String>::new()));
    let panes_in_ui = Arc::clone(&panes_left);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *panes_in_ui.lock().expect("the pane list is poisoned") =
                app.model.layout.panes.keys().cloned().collect();
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
        panes_left.lock().expect("the pane list is poisoned").is_empty(),
        "⌘W should have closed the review pane"
    );
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


/// A diff of many hunks only lays out the cards on screen, but a jump to a hunk still has to
/// reach one that is nowhere near the viewport.
#[test]
fn jumping_to_a_hunk_reaches_one_that_was_being_skipped() {
    let fixture = Fixture::new("scroll-to-hunk");
    for file in 0..80 {
        fixture.write(
            &format!("src/module_{file}/values.rs"),
            "pub const A: u32 = 1;\npub const B: u32 = 2;\n",
        );
    }
    fixture.commit("Add the modules");
    for file in 0..80 {
        fixture.write(
            &format!("src/module_{file}/values.rs"),
            "pub const A: u32 = 9;\npub const B: u32 = 2;\n",
        );
    }

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let last_hunk_id = Arc::new(Mutex::new(String::new()));
    let last_in_ui = Arc::clone(&last_hunk_id);
    let active = Arc::new(Mutex::new(None::<String>));
    let active_in_ui = Arc::clone(&active);
    let jump_to = Arc::new(Mutex::new(None::<String>));
    let jump_in_ui = Arc::clone(&jump_to);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let mut app = app;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .wgpu()
        .build_ui(move |ui| {
            if let Some(hunk_id) = jump_in_ui.lock().expect("poisoned").take() {
                let session_id = app.model.root_session_id.clone();
                app.model.review(&session_id).scroll_to_hunk = Some(hunk_id);
            }
            app.draw(ui);
            if let Some(review) = app.model.review_ref(&app.model.root_session_id) {
                if let Some(hunk) = review.hunks().last() {
                    *last_in_ui.lock().expect("poisoned") = hunk.id.clone();
                }
                ready_in_ui.store(review.payload.is_some(), Ordering::Relaxed);
                *active_in_ui.lock().expect("poisoned") = review.active_hunk_id.clone();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let last = last_hunk_id.lock().expect("poisoned").clone();
    assert!(!last.is_empty(), "the review should have hunks");
    assert_ne!(
        active.lock().expect("poisoned").as_deref(),
        Some(last.as_str()),
        "the last hunk starts far below the viewport"
    );

    *jump_to.lock().expect("poisoned") = Some(last.clone());
    harness.run_steps(4);

    // Only a card that was actually drawn reports the jump, so this is how a skipped one
    // would show up: the review would never come to rest on it.
    assert_eq!(
        active.lock().expect("poisoned").as_deref(),
        Some(last.as_str()),
        "jumping should have drawn the hunk and made it the active one"
    );
}
