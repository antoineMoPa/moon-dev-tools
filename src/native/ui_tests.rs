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

use egui_frames::PaneId;
use egui_kittest::Harness;

use crate::{
    api::OpenSessionRequest,
    backend::local::LocalBackend,
    git::run_git_no_output,
    native::{
        Launch,
        app::App,
        panes::{Pane, PaneKind},
        theme::ThemeMode,
    },
};

/// Where the window drew its frames this pass.
fn frame_rects(app: &App) -> Vec<egui::Rect> {
    app.model
        .layout
        .frame_ids()
        .into_iter()
        .filter_map(|frame| app.frames.frame_rect(frame))
        .collect()
}

/// The same for its tabs, which is what a drag has to start on.
fn tab_rects(app: &App) -> Vec<egui::Rect> {
    app.model
        .layout
        .panes()
        .filter_map(|(pane, _)| app.frames.tab_rect(pane))
        .collect()
}

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
        let enclosing =
            std::env::temp_dir().join(format!("moonreview-ui-{}-{name}", std::process::id()));
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
        picture
            .save(&path)
            .expect("failed to write the fixture image");
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
    app_for_frame(repo_path, theme, crate::cli::Frame::Review)
}

/// The same, opened on whichever of the three executables' frames.
fn app_for_frame(repo_path: &Path, theme: ThemeMode, frame: crate::cli::Frame) -> App {
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let launch = Launch {
        backend: Arc::new(LocalBackend::new(state)),
        open: Some(OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: None,
            active_commit: None,
        }),
        serves_web: false,
        frame,
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
    fixture.write(
        "README.md",
        "# fixture\n\nA repo that exists to be reviewed.\n",
    );
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
        serves_web: false,
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

/// Quitting takes every shell in the window with it, so the first ⌘Q says what it is about to
/// end and the second one goes through.
#[test]
fn quitting_with_a_shell_still_running_asks_first() {
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

    // A shell that is left alone, and so is still running when the quit arrives.
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
        serves_web: false,
        frame: crate::cli::Frame::Review,
    };
    let mut app = App::new(egui::Context::default(), launch);
    app.set_theme(ThemeMode::Dark);
    app.terminals.insert(terminal_id.clone(), pane);

    let warnings = Arc::new(Mutex::new(Vec::new()));
    let warnings_in_ui = Arc::clone(&warnings);
    // A toast stays up for seconds, so the first warning is wiped before the second quit —
    // otherwise what is on screen afterwards says nothing about which quit put it there.
    let wipe = Arc::new(AtomicBool::new(false));
    let wipe_in_ui = Arc::clone(&wipe);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1300.0, 820.0))
        .build_ui(move |ui| {
            if wipe_in_ui.swap(false, Ordering::Relaxed) {
                app.model.toasts.clear();
            }
            app.draw(ui);
            *warnings_in_ui.lock().expect("poisoned") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
        });
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

/// The file tab: a fringe of line numbers beside the text, and the text editable.
#[test]
fn a_file_opens_in_a_tab_of_its_own() {
    let fixture = Fixture::new("file-pane");
    // One line far wider than the pane: it must scroll sideways rather than wrap, and the
    // line numbers must stay put while it does.
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n\npub fn total(values: &[u32], and_a_very_long_parameter_list: &[u32], so_that_this_line_runs_well_past_the_edge_of_the_pane: bool) -> u32 {\n    values.iter().sum()\n}\n",
    );
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .layout
                    .panes()
                    .any(|(_, pane)| matches!(pane, Pane::File { .. }))
                    && app
                        .model
                        .file_editors
                        .values()
                        .any(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !ready.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.load(Ordering::Relaxed), "the file tab never opened");

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    harness.snapshot("file-pane");
}

/// Editing a file tab writes the file back, and the tab says so until it does.
#[test]
fn editing_a_file_tab_saves_it_to_the_working_tree() {
    let fixture = Fixture::new("file-save");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;
    let pane_id = Arc::new(Mutex::new(None::<PaneId>));
    let pane_in_ui = Arc::clone(&pane_id);
    let dirty = Arc::new(AtomicBool::new(false));
    let dirty_in_ui = Arc::clone(&dirty);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let edit = Arc::new(Mutex::new(None::<String>));
    let edit_in_ui = Arc::clone(&edit);
    let save = Arc::new(AtomicBool::new(false));
    let save_in_ui = Arc::clone(&save);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/lib.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if let Some(text) = edit_in_ui.lock().expect("poisoned").take()
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
                && let Some(editor) = app.model.file_editors.get_mut(&id)
            {
                editor.edit_for_test(&text);
            }
            if save_in_ui.swap(false, Ordering::Relaxed)
                && let Some(id) = *pane_in_ui.lock().expect("poisoned")
            {
                let session_id = app.model.root_session_id.clone();
                app.save_file_pane(id, &session_id);
            }

            app.draw(ui);

            let open_pane = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id);
            if let Some(id) = open_pane {
                dirty_in_ui.store(app.file_pane_is_dirty(id), Ordering::Relaxed);
                loaded_in_ui.store(
                    app.model
                        .file_editors
                        .get(&id)
                        .and_then(|editor| editor.content_for_test())
                        .is_some(),
                    Ordering::Relaxed,
                );
            }
            *pane_in_ui.lock().expect("poisoned") = open_pane;
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !loaded.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(loaded.load(Ordering::Relaxed), "the file never loaded");
    assert!(
        !dirty.load(Ordering::Relaxed),
        "a freshly opened file is clean"
    );

    *edit.lock().expect("poisoned") = Some("pub fn two() {}\n".to_string());
    harness.run_steps(2);
    assert!(dirty.load(Ordering::Relaxed), "an edit should mark the tab");
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn one() {}\n",
        "nothing should reach the file until it is saved"
    );

    // The tab carries a dot for as long as the edit is not on disk.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("file-tab-unsaved");

    save.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && dirty.load(Ordering::Relaxed) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        !dirty.load(Ordering::Relaxed),
        "saving should clear the mark"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("src/lib.rs")).expect("failed to read"),
        "pub fn two() {}\n",
        "the edit should be on disk"
    );
}

/// A split handle keeps resizing while the pointer runs past it — the drag belongs to the
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
            // A second frame whose pane goes away without it — what a forgetful caller leaves.
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
/// column beside both — the frame it is dropped over would only split itself.
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

/// The agent belongs to the person reviewing, not to a session that is new every launch, so it
/// is written to `~/.moonreview/settings.json` and asked for again on the way up.
#[test]
fn the_agent_the_last_run_ended_on_comes_back() {
    let fixture = seeded_fixture("agent-memory");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    // A window that ends on Claude says so. The restored agent is cleared first: until it has
    // been put back, the session still reads as no agent at all.
    let session_id = app.model.root_session_id.clone();
    app.model.restored_agent = None;
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
    app.remember_selected_agent();

    assert_eq!(
        crate::settings::load().selected_agent,
        crate::api::AgentKind::Claude,
        "the agent should have been written to the settings file"
    );

    // And the next one starts by asking for it back.
    let next = app_for(&fixture.root, ThemeMode::Dark);

    assert_eq!(
        next.model.restored_agent,
        Some(crate::api::AgentKind::Claude),
        "the saved agent should be waiting to be applied"
    );

    if let Some(path) = crate::settings::path() {
        let _ = fs::remove_file(path);
    }
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
        frame: crate::cli::Frame::Review,
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
            // The file opens as a tab of its own, so what it is showing is looked for there.
            *shown_in_ui.lock().expect("poisoned") = app
                .model
                .layout
                .find_pane(|pane| matches!(pane, Pane::File { .. }))
                .map(|(pane_id, _)| pane_id)
                .and_then(|pane_id| app.model.file_editors.get(&pane_id))
                .and_then(|editor| editor.content_for_test());
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
    // Decoding and uploading the textures takes passes of their own after the diff arrives —
    // and on a machine busy running the rest of this suite, more than a fixed few.
    harness.run_steps(3);
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.ctx.has_pending_images() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    harness.run_steps(2);

    harness.snapshot("image-diff");
}

/// A window opened from a desktop launcher starts outside every repo, so it has to ask which
/// one to review — with the folder picker of the OS, since the repo is on this machine.
#[test]
fn a_window_with_no_repo_asks_which_one_to_review() {
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
            serves_web: false,
            frame: crate::cli::Frame::Review,
        },
    );
    app.set_theme(ThemeMode::Dark);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| app.draw(ui));
    harness.run_steps(3);

    harness.snapshot("repo-prompt");
}

/// The three executables share the launch screen, so it has to say what the window it is in
/// front of actually opens — a board is not a review.
#[test]
fn the_launch_screen_of_the_board_does_not_offer_a_review() {
    use egui_kittest::kittest::Queryable as _;

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
            serves_web: false,
            frame: crate::cli::Frame::Tasks,
        },
    );
    app.set_theme(ThemeMode::Dark);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| app.draw(ui));
    harness.run_steps(3);

    assert!(
        harness.query_by_label_contains("moontasks").is_some(),
        "expected the board's launch screen to name the board's executable"
    );
    assert!(
        harness.query_by_label_contains("review").is_none(),
        "expected nothing on the board's launch screen to mention reviewing"
    );
    assert!(
        harness.query_by_label_contains("board").is_some(),
        "expected the board's launch screen to ask which repo's board to open"
    );
}

/// Going back to yesterday's project should not mean naming it again, so the launch screen
/// lists the ones opened before and opens the clicked one.
#[test]
fn the_launch_screen_offers_the_projects_opened_before() {
    use egui_kittest::kittest::Queryable as _;

    let mut saved = crate::settings::Settings::default();
    saved.remember_project("/home/you/older");
    saved.remember_project("/home/you/newest");
    crate::settings::store(&saved).expect("expected the settings to be written");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
            serves_web: false,
            frame: crate::cli::Frame::Review,
        },
    );
    app.set_theme(ThemeMode::Dark);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| app.draw(ui));
    harness.run_steps(3);
    harness.snapshot("repo-prompt-recents");

    // Named by their own directory rather than the whole path.
    assert!(
        harness.query_by_label_contains("newest").is_some(),
        "expected the launch screen to list the project opened last"
    );
    assert!(
        harness.query_by_label_contains("older").is_some(),
        "expected the launch screen to list the earlier project too"
    );

    if let Some(path) = crate::settings::path() {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn the_review_window_draws_in_the_light_theme_too() {
    let fixture = seeded_fixture("review-light");
    let app = app_for(&fixture.root, ThemeMode::Light);

    let mut harness = harness_with_loaded_review(app, ThemeMode::Light);

    harness.snapshot("review-light");
}

/// The board is the repo's `.moontasks` folder, so the fixture writes the folder and the
/// window is expected to show exactly what is in it.
#[test]
fn the_moontasks_board_draws_what_is_in_the_repo() {
    let fixture = seeded_fixture("board");
    // Written by hand rather than through the service: the ids a real one generates carry a
    // uuid, and the point here is a picture that is the same on every run.
    for (task_id, title, status) in [
        ("write-the-parser-1111", "Write the parser", "todo"),
        (
            "fix-the-login-page-2222",
            "Fix the login page",
            "in_progress",
        ),
        ("drop-the-old-api-3333", "Drop the old API", "done"),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // The new-task box is opened rather than clicked for: where the `+` lands depends on the
    // column, and what this checks is the box it opens.
    let compose = Arc::new(AtomicBool::new(false));
    let compose_in_ui = Arc::clone(&compose);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            // Only once the review has opened: opening it replaces the whole arrangement.
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if compose_in_ui.load(Ordering::Relaxed) {
                app.model.board.composer_in = Some(crate::moontasks::ColumnId::new("todo"));
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 3,
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
        "the board never read the three tasks out of .moontasks"
    );

    harness.run_steps(3);
    harness.snapshot("moontasks-board");

    // And the new-task box the `+` on the TODO column opens, standing where its card will go.
    compose.store(true, Ordering::Relaxed);
    // Its title box has focus, and a blinking caret would make the image differ run to run.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    harness.snapshot("moontasks-new-task");
}

/// The attach modal offers the sessions the agents themselves have on this machine, which
/// is nothing a test can rely on — so the listing is injected, and what is checked is the
/// modal itself: what it shows, and Escape closing it.
#[test]
fn the_attach_modal_lists_the_agents_own_sessions() {
    use crate::api::AgentKind;

    let fixture = seeded_fixture("board-attach");
    fixture.write(
        ".moontasks/write-the-parser-1111/metadata.json",
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened_in_ui = Arc::new(AtomicBool::new(false));
    // The board and its card have been read: the snapshot's background is settled.
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let inject = Arc::new(AtomicBool::new(false));
    let inject_in_ui = Arc::clone(&inject);
    // What the modal's state is right now, read back out of the draw closure.
    let picker_open = Arc::new(AtomicBool::new(false));
    let picker_open_in_ui = Arc::clone(&picker_open);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            // Once, when the test says so — the way OpenAttachPicker fills it in, but with
            // sessions this machine is known not to have.
            if inject_in_ui.swap(false, Ordering::Relaxed) {
                app.model.board.attach_picker = Some(crate::native::model::AttachPicker {
                    task_id: "write-the-parser-1111".to_string(),
                    task_title: "Write the parser".to_string(),
                    sessions: Some(vec![
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::Claude,
                            id: "3f37e6a1-4a11-4333-8444-555555555555".to_string(),
                            title: "Fix the login page".to_string(),
                            updated_at_unix: 1_700_003_600,
                        },
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::OpenCode,
                            id: "ses_012f01ba5ffeTRe0q5MsyL9wbO".to_string(),
                            title: "Character-precise review selection".to_string(),
                            updated_at_unix: 1_699_900_000,
                        },
                        crate::agent_sessions::AgentSessionView {
                            agent: AgentKind::Codex,
                            id: "019efeff-2a80-7b11-b0b1-c5ab3e09b353".to_string(),
                            title: "Rewrite the scheduler".to_string(),
                            updated_at_unix: 1_699_800_000,
                        },
                    ]),
                    error: None,
                    manual_id: String::new(),
                    manual_agent: None,
                });
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 1,
                Ordering::Relaxed,
            );
            picker_open_in_ui.store(
                app.model.board.attach_picker.is_some(),
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
        "the board never read the task out of .moontasks"
    );

    inject.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    assert!(
        picker_open.load(Ordering::Relaxed),
        "the injected modal never showed"
    );
    harness.snapshot("moontasks-attach-session");

    // Escape is the way out that touches nothing.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(
        !picker_open.load(Ordering::Relaxed),
        "escape did not close the attach modal"
    );
}

/// Dragging a card is how a column is put in order, so where it is let go of has to be where
/// it lands — not merely which column it landed in.
#[test]
fn a_card_dropped_above_another_takes_its_place() {
    let fixture = seeded_fixture("board-order");
    // Cards that have never been moved read in the order they were created, so the fixture
    // says when each one was.
    for (task_id, title, created) in [
        ("write-the-parser-1111", "Write the parser", 1700000000),
        ("fix-the-login-page-2222", "Fix the login page", 1700000001),
        ("drop-the-old-api-3333", "Drop the old API", 1700000002),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"todo\",\n  \
                 \"created_at_unix\": {created},\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // What the board is showing, read out on every frame: the drop is answered on a worker
    // thread, so the order has to be watched for rather than counted in frames.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let order_in_ui = Arc::clone(&order);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            if let Ok(mut order) = order_in_ui.lock() {
                *order = app
                    .model
                    .board
                    .tasks
                    .iter()
                    .map(|task| task.title.clone())
                    .collect();
            }
        });

    let read = || order.lock().expect("expected the board").clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && read().len() != 3 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        read(),
        ["Write the parser", "Fix the login page", "Drop the old API"],
        "the board never read the three tasks in the order they were written"
    );
    harness.run_steps(3);

    // The title, which is the handle a card is dragged by and so the thing the pointer has
    // to press on.
    let handle_of = |harness: &Harness<'_>, task_id: &str| {
        harness
            .ctx
            .read_response(egui::Id::new(("moontask-card", &task_id.to_string())))
            .expect("expected the card to have been drawn")
            .rect
    };
    let first = handle_of(&harness, "write-the-parser-1111");
    let last = handle_of(&harness, "drop-the-old-api-3333");
    // Picked up by its title and let go of below the middle of the first card, which is the
    // gap under it.
    let start = last.center();
    let end = first.center_bottom() + egui::vec2(0.0, 12.0);

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
    for at in [start + egui::vec2(0.0, -20.0), end] {
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.step();
    }
    // A few frames with the pointer where it is: the slot a card is being held over is worked
    // out at the end of a frame and taken up by the next one, and the cards making room for
    // it walk there rather than jumping.
    harness.run_steps(12);

    // Mid-drag: the card is under the cursor, and the space being held for it is where it
    // would land — between the two cards it is being dropped between.
    harness.snapshot("moontasks-drag");

    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.step();

    // Just dropped: the card is in the slot it was held over, marked so it can be picked back
    // out of the column it landed in.
    harness.snapshot("moontasks-dropped");

    let expected = ["Write the parser", "Drop the old API", "Fix the login page"];
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && read() != expected {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        read(),
        expected,
        "the card should have landed in the gap it was dropped in"
    );
}

/// Several windows on several projects is the ordinary way to work, so the title bar has to
/// say which project each one is on.
#[test]
fn the_window_is_titled_after_the_project_it_is_open_on() {
    let fixture = seeded_fixture("window-title");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(Mutex::new(None));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .build_ui(move |ui| {
            app.draw(ui);
            if let Ok(mut opened) = opened_in_ui.lock() {
                *opened = app.model.project_path.clone();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut project = None;
    while Instant::now() < deadline && project.is_none() {
        harness.step();
        project = opened.lock().ok().and_then(|opened| opened.clone());
        std::thread::sleep(Duration::from_millis(10));
    }

    let project = project.expect("the window never learned which project it is on");
    let titled = crate::native::app::window_title(crate::cli::Frame::Review, Some(&project));
    assert!(
        titled.ends_with(&project),
        "expected the title to name the project, got {titled:?} for {project:?}"
    );
    assert!(
        titled.starts_with("🌚 moonreview — "),
        "expected the title to keep naming the executable, got {titled:?}"
    );
}

/// The home directory is written the short way, which is how a path is read at a glance.
#[test]
fn a_project_under_the_home_directory_is_titled_with_a_tilde() {
    let home = std::env::var("HOME").expect("expected a home directory");

    let titled = crate::native::app::window_title(
        crate::cli::Frame::Tasks,
        Some(&format!("{home}/prog/moonreview")),
    );

    assert_eq!(titled, "🌚 moontasks — ~/prog/moonreview");
}

/// A card's title is whatever someone typed on the way past, and some of them are a sentence.
/// The column is a fixed width, so a long title has to be cut into it rather than widen it —
/// widening one column used to push the rest of the board off the side of the window.
#[test]
fn a_long_task_title_is_cut_into_its_column() {
    let fixture = seeded_fixture("board-long-title");
    for (task_id, title, status) in [
        (
            "long-title-1111",
            "Rework the dispatch queue so held comments survive a restart, and take the \
             chance to rename everything around it while we are here",
            "todo",
        ),
        (
            "fix-the-login-page-2222",
            "Fix the login page",
            "in_progress",
        ),
    ] {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"{status}\",\n  \
                 \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 2,
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
        "the board never read the two tasks out of .moontasks"
    );

    harness.run_steps(3);
    harness.snapshot("moontasks-long-title");
}

/// The three executables are the same window opened on three different things, which is the
/// whole of what tells them apart.
#[test]
fn each_executable_opens_on_its_own_frame() {
    for (frame, expected) in [
        (crate::cli::Frame::Review, PaneKind::Review),
        (crate::cli::Frame::Tasks, PaneKind::Tasks),
        // `moonshell` has to start a shell before it has a pane, so this one also checks that
        // a window which opens empty does not stay empty.
        (crate::cli::Frame::Shell, PaneKind::Terminal),
    ] {
        let fixture = seeded_fixture(&format!("frame-{expected:?}").to_lowercase());
        let app = app_for_frame(&fixture.root, ThemeMode::Dark, frame);
        let opened = Arc::new(Mutex::new(None));
        let opened_in_ui = Arc::clone(&opened);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .wgpu()
            .build_ui({
                let mut app = app;
                move |ui| {
                    app.draw(ui);
                    *opened_in_ui.lock().expect("poisoned") =
                        app.model.layout.active_pane().map(|(_, pane)| pane.kind());
                }
            });

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            harness.step();
            if *opened.lock().expect("poisoned") == Some(expected) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            *opened.lock().expect("poisoned"),
            Some(expected),
            "{frame:?} should have opened on {expected:?}"
        );
    }
}

/// A card is moved between columns by dragging it, which is the only way to move one — the
/// arrows that used to do it are gone.
#[test]
fn dragging_a_card_moves_it_to_the_column_it_is_dropped_on() {
    let fixture = seeded_fixture("board-drag");
    let task_id = "write-the-parser-1111";
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // Where the card is drawn, and which column it is in, both read back out of the window.
    let seen = Arc::new(Mutex::new((egui::Rect::NOTHING, String::new())));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            if let Some(task) = app.model.board.tasks.first() {
                ready_in_ui.store(true, Ordering::Relaxed);
                let title = ui
                    .ctx()
                    .read_response(egui::Id::new(("moontask-card", &task.id)));
                *seen_in_ui.lock().expect("poisoned") = (
                    title
                        .map(|response| response.rect)
                        .unwrap_or(egui::Rect::NOTHING),
                    task.status.to_string(),
                );
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if ready.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let (handle, status) = seen.lock().expect("poisoned").clone();
    assert_eq!(status, "todo", "the card starts in TODO");
    assert!(
        handle.is_positive(),
        "the card's drag handle was never drawn"
    );

    // One column to the right, which is IN PROGRESS.
    let onto = handle.center() + egui::vec2(COLUMN_STRIDE, 40.0);
    drag_from_to(&mut harness, handle.center(), onto);

    // The move is written to `.moontasks` and read back, so the board has to poll to see it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if seen.lock().expect("poisoned").1 == "in_progress" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        seen.lock().expect("poisoned").1,
        "in_progress",
        "dropping the card on IN PROGRESS should have moved it there"
    );
}

/// How far apart two columns of the board are drawn, which is what a drag has to cover to
/// reach the next one.
const COLUMN_STRIDE: f32 = 298.0;

/// A column is moved by dragging its heading, and its cards go with it — a card names the
/// column it is in rather than a place on the board, so nothing about the card changes.
#[test]
fn dragging_a_heading_moves_the_column_and_its_cards() {
    let fixture = seeded_fixture("column-drag");
    let task_id = "write-the-parser-1111";
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);

    /// The order of the columns, where TODO's heading is, and which column the card is in.
    #[derive(Clone)]
    struct Seen {
        order: Vec<String>,
        handle: egui::Rect,
        card_status: String,
    }
    let seen = Arc::new(Mutex::new(Seen {
        order: Vec::new(),
        handle: egui::Rect::NOTHING,
        card_status: String::new(),
    }));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            let heading = ui
                .ctx()
                .read_response(egui::Id::new(("moontask-column", "todo")));
            if let Ok(mut seen) = seen_in_ui.lock() {
                seen.order = app
                    .model
                    .board
                    .columns
                    .iter()
                    .map(|column| column.id.to_string())
                    .collect();
                seen.handle = heading
                    .map(|response| response.rect)
                    .unwrap_or(egui::Rect::NOTHING);
                seen.card_status = app
                    .model
                    .board
                    .tasks
                    .first()
                    .map(|task| task.status.to_string())
                    .unwrap_or_default();
            }
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if seen.lock().expect("poisoned").handle.is_positive() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);

    let before = seen.lock().expect("poisoned").clone();
    assert_eq!(
        before.order.first().map(String::as_str),
        Some("todo"),
        "TODO starts on the left, got {:?}",
        before.order
    );
    assert_eq!(before.card_status, "todo");
    assert!(
        before.handle.is_positive(),
        "the column's drag handle was never drawn"
    );

    // Carried past the middle of IN PROGRESS, which is what puts TODO on the far side of it.
    // The column travels on the cursor, so what has to clear that middle is the cursor.
    let onto = egui::pos2(
        before.handle.center().x + COLUMN_STRIDE * 1.5,
        before.handle.center().y,
    );
    drag_from_to(&mut harness, before.handle.center(), onto);

    // The move is written to `.moontasks` and read back, so the board has to poll to see it.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if seen
            .lock()
            .expect("poisoned")
            .order
            .first()
            .map(String::as_str)
            == Some("in_progress")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let after = seen.lock().expect("poisoned").clone();
    assert_eq!(
        after.order,
        ["in_progress", "todo", "done"],
        "dropping the heading one place right should have moved the column there"
    );
    assert_eq!(
        after.card_status, "todo",
        "the card should have travelled with its column, unchanged"
    );
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
    "\u{23F5}\u{23F7}",         // collapse arrows
    "+",                        // open a pane
    "\u{00B7}\u{2212}",         // separator, minus sign
    "\u{2039}\u{203A}\u{00D7}", // the find bar's previous, next and close
    "\u{23F4}\u{23F5}",         // the board's move-a-card-along arrows
    // The command key is the one modifier the bundled fonts have a glyph for; the rest of a
    // chord is spelled out, which is what `bindings::describe` does.
    "\u{2318}",
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
                    .map(|selection| selection.line_range().count())
                    .unwrap_or(0);
                seen.draft_selection = review.drafts.first().map(|draft| draft.selection.clone());
                seen.draft_is_focused = review
                    .drafts
                    .first()
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
        assert!(
            state.draft_is_focused,
            "the composer should be ready to type in"
        );
    }

    // And the composer is on screen, not merely in the model.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("comment-composer");
}

/// cmd+c over the diff copies what is selected — and copies the code, without the `+` that
/// says it was added. A clicked line is selected whole, so that is what arrives.
#[test]
fn copy_takes_the_selected_diff_lines_without_their_diff_markers() {
    let fixture = seeded_fixture("copy-diff");
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
            state.hunk_id.clone().expect("expected a hunk to copy from"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, raw) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.text.clone()))
        .expect("expected an added line to copy");

    let target = crate::native::review::hunks::diff_line_id(&hunk_id, line_index);
    let rect = harness
        .ctx
        .read_response(target)
        .expect("expected the diff line to have been drawn")
        .rect;
    click_at(&mut harness, rect.center());
    harness.run_steps(2);

    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    harness.run_steps(2);

    let copied = seen
        .lock()
        .expect("poisoned")
        .copied
        .clone()
        .expect("cmd+c over a selected diff line should have copied it");
    assert_eq!(
        copied,
        raw[1..],
        "the code should arrive without the `+` in front of it"
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

/// Whether the window asked to be closed, which is what quitting looks like from in here.
fn asked_to_close(harness: &Harness<'_>) -> bool {
    harness.output().viewport_output.values().any(|viewport| {
        viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
    })
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
                seen.selected = review.selection.map(|selection| {
                    (
                        *selection.line_range().start(),
                        *selection.line_range().end(),
                    )
                });
                seen.draft_selection = review.drafts.first().map(|draft| draft.selection.clone());
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
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
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
    let expected: Vec<&str> = (from..=to)
        .map(|index| lines[index].text.as_str())
        .collect();
    assert_eq!(selection, expected.join("\n"));
    drop(state);

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("multi-line-selection");
}

/// A comment being typed survives everything short of deliberately cancelling it: sweeping
/// a new run of lines parks the typed composer where it is and opens a fresh one, and an
/// Escape — which may have been aimed at a palette or a terminal in the next split — never
/// throws typed text away.
#[test]
fn reselecting_lines_keeps_the_note_being_typed() {
    let fixture = seeded_fixture("keep-note");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        notes: Vec<String>,
        selected_lines: usize,
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
                seen.notes = review
                    .drafts
                    .iter()
                    .map(|draft| draft.note.clone())
                    .collect();
                seen.selected_lines = review
                    .selection
                    .map(|selection| selection.line_range().count())
                    .unwrap_or(0);
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
    assert!(changed.len() >= 3, "the fixture needs lines to sweep over");

    let rect_of = |harness: &Harness<'_>, index: usize| {
        harness
            .ctx
            .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, index))
            .expect("expected the diff line to have been drawn")
            .rect
    };

    // Open the composer on the first changed line and type into it.
    let first = rect_of(&harness, changed[0]).center();
    click_at(&mut harness, first);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("needs work".to_string()));
    harness.step();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").notes,
        ["needs work"],
        "typing should land in the composer"
    );

    // Sweep a different run of lines: the typed composer stays parked with its text, and a
    // fresh one opens on the new run.
    let start = rect_of(&harness, changed[1]).center();
    let end = rect_of(&harness, changed[2]).center();
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
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
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

    {
        let state = seen.lock().expect("poisoned");
        assert_eq!(
            state.notes,
            ["needs work", ""],
            "the typed composer stays parked, and a fresh one opens on the new run"
        );
        assert_eq!(state.selected_lines, 2, "the new run is what is selected");
    }

    // Escape closes the fresh, empty composer — the one holding the keyboard — and leaves
    // the typed one alone, wherever the Escape was actually aimed.
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);
    assert_eq!(
        seen.lock().expect("poisoned").notes,
        ["needs work"],
        "escape must never discard typed text"
    );
}

/// Double-clicking a word in a diff selects just that word, and cmd+c copies exactly it.
#[test]
fn double_clicking_a_word_selects_and_copies_it() {
    let fixture = seeded_fixture("word-select");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    #[derive(Default)]
    struct Seen {
        hunk_id: Option<String>,
        patch: String,
        selection: Option<crate::native::model::LineSelection>,
        copied: Option<String>,
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
                seen.selection = review.selection;
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
            state.hunk_id.clone().expect("expected a hunk"),
            state.patch.clone(),
        )
    };
    let lines = crate::native::review::diff::build_diff_lines(&patch);
    let (line_index, body) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.kind == crate::native::review::diff::LineKind::Added)
        .map(|(index, line)| (index, line.body().to_string()))
        .expect("expected an added line");

    let rect = harness
        .ctx
        .read_response(crate::native::review::hunks::diff_line_id(&hunk_id, line_index))
        .expect("expected the diff line to have been drawn")
        .rect;
    // A few pixels into the line's first word — the row is as wide as the pane, and a
    // double-click past the end of the text falls back to the whole line.
    let at = egui::pos2(
        crate::native::review::hunks::body_text_x(rect) + 10.0,
        rect.center().y,
    );
    // Two clicks one step apart: the harness steps a quarter second at a time, and egui
    // counts a double-click only inside 0.3s, so anything looser reads as two single clicks.
    let press_and_release = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(at),
        press_and_release(true),
        press_and_release(false),
    ]);
    harness.step();
    harness
        .input_mut()
        .events
        .extend([press_and_release(true), press_and_release(false)]);
    harness.step();
    harness.run_steps(2);

    let selection = seen
        .lock()
        .expect("poisoned")
        .selection
        .expect("the double-click should have selected");
    assert_eq!(
        selection.line_range().count(),
        1,
        "a word lives on one line"
    );
    let (from, to) = selection
        .columns_on(line_index)
        .expect("the selection is on the clicked line");
    assert!(
        to < crate::native::model::LINE_END && to <= body.chars().count(),
        "a word selection ends inside the line"
    );
    assert!(from < to, "a word selection covers characters");

    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    harness.run_steps(2);

    let copied = seen
        .lock()
        .expect("poisoned")
        .copied
        .clone()
        .expect("cmd+c should have copied the word");
    let expected: String = body
        .chars()
        .skip(from)
        .take(to - from)
        .collect();
    assert_eq!(copied, expected, "what copies is exactly the selected word");
    assert!(
        !copied.trim().is_empty(),
        "the middle of a code line is a word, not blank space"
    );
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

    let comment =
        crate::comments::build_anchored_comment_value(&[crate::comments::AnchoredComment {
            selection: anchor,
            comment: "this needs a second look".to_string(),
            resolved: false,
        }]);

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
    assert_eq!(
        held.review_comments.len(),
        1,
        "the comment should be stored"
    );
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

/// The sidebar's staging dot is also the control for it, the way the web sidebar's status
/// badge is: one click stages the whole file, the next one takes it back out of the index.
#[test]
fn clicking_a_file_staging_dot_stages_the_whole_file() {
    let fixture = seeded_fixture("stage-dot");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    /// What the file's hunks say about the index, read from inside the UI closure.
    #[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
    struct Staging {
        hunks: usize,
        staged: usize,
    }

    let staging = Arc::new(Mutex::new(Staging::default()));
    let staging_in_ui = Arc::clone(&staging);
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
            let of_file = review
                .hunks()
                .iter()
                .filter(|hunk| hunk.file_path == "src/lib.rs")
                .fold(Staging::default(), |mut seen, hunk| {
                    seen.hunks += 1;
                    seen.staged += usize::from(hunk.staged);
                    seen
                });
            *staging_in_ui.lock().expect("poisoned") = of_file;
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

    let before = *staging.lock().expect("poisoned");
    assert!(before.hunks > 0, "the fixture edits src/lib.rs");
    assert_eq!(before.staged, 0, "the fixture's edits start unstaged");

    let dot = harness
        .ctx
        .read_response(crate::native::review::sidebar::stage_dot_id("src/lib.rs"))
        .expect("expected the file row's staging dot to have been drawn")
        .rect;
    click_at(&mut harness, dot.center());

    // Staging runs on a worker thread and the review is refetched after it, so the model
    // catches up over the next few frames rather than on the click itself.
    let all_staged = settle(&mut harness, || {
        let seen = *staging.lock().expect("poisoned");
        seen.hunks > 0 && seen.staged == seen.hunks
    });
    assert!(
        all_staged,
        "clicking the dot should have staged every hunk of the file, saw {:?}",
        *staging.lock().expect("poisoned")
    );

    // The dot now reads staged, so the same click has to be the way back out.
    click_at(&mut harness, dot.center());
    let all_unstaged = settle(&mut harness, || {
        let seen = *staging.lock().expect("poisoned");
        seen.hunks > 0 && seen.staged == 0
    });
    assert!(
        all_unstaged,
        "clicking the dot again should have unstaged the file, saw {:?}",
        *staging.lock().expect("poisoned")
    );
}

/// Step frames until the condition holds, which is how a background task's result is waited on.
fn settle(harness: &mut Harness<'_>, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Tab belongs to the shell — it is how a path gets completed — not to egui's focus
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
        serves_web: false,
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

/// `C-x o` walks the keyboard round the workspace's frames. The prefix has to survive the
/// frame it was pressed in — it is two presses, and each one arrives in a pass of its own.
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

/// Press and release a key, then let the UI settle.
fn press_key(harness: &mut Harness<'_>, key: egui::Key, modifiers: egui::Modifiers) {
    harness.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    harness.step();
    harness.run_steps(2);
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
        serves_web: false,
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

/// Press at one point, sweep to another, release — one pointer gesture, several frames.
fn drag_from_to(harness: &mut Harness<'_>, from: egui::Pos2, to: egui::Pos2) {
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

    // A few steps along the way, so the drag is a sweep rather than a jump.
    for step in 1..=4 {
        let towards = from + (to - from) * (step as f32 / 4.0);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(towards));
        harness.step();
    }

    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.run_steps(2);
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

/// ⌘F over a review searches every hunk it is showing, not only the lines on screen, and
/// stepping through the matches moves the current one.
#[test]
fn find_searches_a_whole_review_and_steps_through_the_matches() {
    let fixture = seeded_fixture("find-review");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut app = app;

    /// What the test reads back out of the window each frame.
    #[derive(Default, Clone)]
    struct Seen {
        open: bool,
        query: String,
        total: usize,
        at: usize,
        current_hunk: Option<String>,
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 880.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let session_id = app.model.root_session_id.clone();
            let current_hunk = app
                .model
                .review_ref(&session_id)
                .and_then(|review| review.find_match.as_ref())
                .map(|found| found.hunk_id.clone());
            *seen_in_ui.lock().expect("poisoned") = Seen {
                open: app.model.find.is_some(),
                query: app
                    .model
                    .find
                    .as_ref()
                    .map(|find| find.query.clone())
                    .unwrap_or_default(),
                total: app.model.find.as_ref().map(|find| find.total).unwrap_or(0),
                at: app.model.find.as_ref().map(|find| find.at).unwrap_or(0),
                current_hunk,
            };
            ready_in_ui.store(
                app.model
                    .review_ref(&session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    let loaded = settle(&mut harness, || ready.load(Ordering::Relaxed));
    assert!(loaded, "the review never loaded");
    harness.run_steps(2);
    assert!(!seen.lock().expect("poisoned").open, "no bar before ⌘F");

    press_key(&mut harness, egui::Key::F, egui::Modifiers::COMMAND);
    assert!(
        seen.lock().expect("poisoned").open,
        "⌘F should have opened the find bar"
    );

    // Typed into the bar, which took the keyboard when it opened.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("values".to_string()));
    harness.step();
    harness.run_steps(3);

    let after_typing = seen.lock().expect("poisoned").clone();
    assert_eq!(after_typing.query, "values");
    // `values` is all over the fixture's second function, on both sides of the diff.
    assert!(
        after_typing.total >= 2,
        "the search should have found every hunk's matches, got {}",
        after_typing.total
    );
    assert_eq!(after_typing.at, 0, "and started on the first one");
    assert!(
        after_typing.current_hunk.is_some(),
        "the review should know which match it is on"
    );

    // What the bar and the marked matches actually look like over a review.
    harness.snapshot("find-bar");

    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    let after_step = seen.lock().expect("poisoned").clone();
    assert_eq!(after_step.at, 1, "Enter steps to the next match");
    assert_eq!(
        after_step.total, after_typing.total,
        "stepping does not change what was found"
    );

    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(
        !seen.lock().expect("poisoned").open,
        "Escape should have put the bar away"
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

/// A line of code longer than the pane is wide. It has to stop at the edge of its hunk card:
/// before this, a long line carried on over the card's border and across the pane beside it.
#[test]
fn a_diff_line_longer_than_the_pane_stops_at_the_card() {
    let fixture = Fixture::new("long-diff-line");
    fixture.write("src/lib.rs", "pub fn short() {}\n");
    fixture.commit("Add the library");
    fixture.write(
        "src/lib.rs",
        "pub fn short() {}\npub fn a_line_far_wider_than_any_pane(first_parameter: &str, second_parameter: &str, third_parameter: &str, fourth_parameter: &str, fifth_parameter: &str) -> String { String::new() }\n",
    );

    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);

    harness.snapshot("long-diff-line");
}

/// A recent project is a link across its whole row, not only the sliver under its text.
/// Selectable labels take the click for themselves, which used to leave the row live only
/// at its top and bottom edges.
#[test]
fn a_recent_project_opens_from_the_middle_of_its_row() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("recent-row-click");
    fixture.write("src/lib.rs", "fn one() {}\n");
    fixture.commit("first");

    let mut saved = crate::settings::Settings::default();
    saved.remember_project(&fixture.root.display().to_string());
    crate::settings::store(&saved).expect("expected the settings to be written");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
            serves_web: false,
            frame: crate::cli::Frame::Review,
        },
    );
    app.set_theme(ThemeMode::Dark);

    let left_the_launch_screen = Arc::new(AtomicBool::new(false));
    let seen_in_ui = Arc::clone(&left_the_launch_screen);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 560.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            if !matches!(app.model.stage, crate::native::model::Stage::Prompt { .. }) {
                seen_in_ui.store(true, Ordering::SeqCst);
            }
        });
    harness.run_steps(3);

    // The fixture's own directory name, which the row shows in bold. The picker button
    // beside it says "Choose a repo…", so the label is the one to take the rect from.
    let row = harness
        .query_by_role_and_label(egui::accesskit::Role::Label, "repo")
        .expect("expected the launch screen to list the fixture project")
        .rect();
    let middle = row.center();

    harness.input_mut().events.extend([
        egui::Event::PointerMoved(middle),
        egui::Event::PointerButton {
            pos: middle,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: middle,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.run_steps(3);

    if let Some(path) = crate::settings::path() {
        let _ = fs::remove_file(path);
    }
    assert!(
        left_the_launch_screen.load(Ordering::SeqCst),
        "expected clicking the middle of a recent project's row to open it"
    );
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
        serves_web: false,
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
    assert!(before.contains("195"), "expected the shell to have printed: {before}");

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
