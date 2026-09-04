//! Renders the real window offscreen and checks what came out.
//!
//! These drive [`App::draw`] through `egui_kittest`, which runs the same egui passes and the
//! same wgpu renderer the window uses. That makes it possible to assert on what the review
//! actually looks like - a diff that fails to draw, or an empty pane, shows up here.

mod board;
mod board_cards;
mod board_drag;
mod board_task_pane;
mod board_selection;
mod diff_comments;
mod diff_selection;
mod files;
mod launch;
mod layout;
mod palette;
mod workspace_color;
mod project;
mod shell_input;
mod shell_lifecycle;
mod sidebar_menu;
mod submodules;
mod tab_rename;

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
    native::{Launch, app::App, panes::Pane, theme::ThemeMode},
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
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
}

/// A fixed point in time, so a fixture commit always hashes to the same sha.
///
/// The review shows short shas and the repo's own name, and both end up in the snapshots. A
/// timestamp or a process id in either would make every run differ from the last.
const FIXTURE_DATE: &str = "2024-01-02T03:04:05+00:00";

impl Fixture {
    pub(crate) fn new(name: &str) -> Self {
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

    pub(crate) fn write(&self, relative: &str, contents: &str) {
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

    pub(crate) fn commit(&self, message: &str) {
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

/// Build the window over a local backend, with no server behind it: the tests are about the UI.
pub(crate) fn app_for(repo_path: &Path, theme: ThemeMode) -> App {
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
pub(crate) fn harness_with_loaded_review(app: App, theme: ThemeMode) -> Harness<'static> {
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

pub(crate) fn seeded_fixture(name: &str) -> Fixture {
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

/// Whether the window asked to be closed, which is what quitting looks like from in here.
fn asked_to_close(harness: &Harness<'_>) -> bool {
    harness.output().viewport_output.values().any(|viewport| {
        viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
    })
}

/// Whether the window took a close back, which is what the quit warning does while it asks.
fn asked_to_stay_open(harness: &Harness<'_>) -> bool {
    harness.output().viewport_output.values().any(|viewport| {
        viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
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

/// The three cards the marking and dragging tests below all start from, in TODO.
const CARDS: [(&str, &str, u64); 3] = [
    ("write-the-parser-1111", "Write the parser", 1700000000),
    ("fix-the-login-page-2222", "Fix the login page", 1700000001),
    ("drop-the-old-api-3333", "Drop the old API", 1700000002),
];

/// What those tests read back out of the window on every frame: where each card is, what is
/// marked, and whether a task's own page has opened.
#[derive(Clone, Default)]
struct Seen {
    columns: Vec<(String, String)>,
    marked: Vec<String>,
    /// Whether any task's own page is open, and the tasks whose pages those are.
    page_open: bool,
    pages_open: Vec<String>,
}

impl Seen {
    fn column_of(&self, task_id: &str) -> String {
        self.columns
            .iter()
            .find(|(id, _)| id == task_id)
            .map(|(_, status)| status.clone())
            .unwrap_or_default()
    }
}

/// A window open on a board of those three cards, read on every frame, with `notes_on` naming
/// the cards that have notes - which is what makes a card's description a thing to click.
///
/// The repo comes back with it: it is a folder that lives as long as the value holding it, and
/// a board whose repo has been swept up under it reads as a board with nothing on it.
fn board_of(name: &str, notes_on: &[&str]) -> (Harness<'static>, Arc<Mutex<Seen>>, Fixture) {
    let fixture = seeded_fixture(name);
    for (task_id, title, created) in CARDS {
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"todo\",\n  \
                 \"created_at_unix\": {created},\n  \"resources\": []\n}}\n"
            ),
        );
        if notes_on.contains(&task_id) {
            fixture.write(
                &format!(".moontasks/{task_id}/notes.md"),
                "Something worth writing down.\n",
            );
        }
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);

            let mut seen = seen_in_ui.lock().expect("poisoned");
            seen.columns = app
                .model
                .board
                .tasks
                .iter()
                .map(|task| (task.id.clone(), task.status.to_string()))
                .collect();
            seen.marked = marked_tasks(&app);
            seen.pages_open = CARDS
                .iter()
                .map(|(task_id, ..)| task_id)
                .filter(|wanted| {
                    app.model
                        .layout
                        .find_pane(|pane| {
                            matches!(pane, Pane::Start { task_id, .. } if task_id == *wanted)
                        })
                        .is_some()
                })
                .map(|task_id| task_id.to_string())
                .collect();
            seen.page_open = !seen.pages_open.is_empty();
        });

    let read = Arc::clone(&seen);
    assert!(
        settle(&mut harness, || read.lock().expect("poisoned").columns.len() == 3),
        "the board never read the three tasks"
    );
    harness.run_steps(3);
    (harness, seen, fixture)
}

/// Where a card's title is drawn, which is the middle of the card as far as a hand is
/// concerned.
fn title_of(harness: &Harness<'_>, task_id: &str) -> egui::Pos2 {
    harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(task_id))
        .expect("expected the card to have been drawn")
        .rect
        .center()
}

/// Just under a card's title, where its description is - a click there opens the task, so a
/// press that carries from there is a card being picked up by something that is not a handle.
fn notes_of(harness: &Harness<'_>, task_id: &str) -> egui::Pos2 {
    let title = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(task_id))
        .expect("expected the card to have been drawn")
        .rect;
    egui::pos2(title.center().x, title.bottom() + 18.0)
}

/// The cards the board has marked, in a settled order.
fn marked_tasks(app: &crate::native::app::App) -> Vec<String> {
    let mut marked: Vec<String> = app.model.board.marked.iter().cloned().collect();
    marked.sort();
    marked
}

/// The one card the board has marked, for the tests that only ever mark one.
fn marked_task(app: &crate::native::app::App) -> Option<String> {
    let mut marked = marked_tasks(app);
    assert!(marked.len() <= 1, "expected one card marked, got {marked:?}");
    marked.pop()
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

/// A letter as the platform sends one: the key, then the text it produced.
fn type_letter(harness: &mut Harness<'_>, key: egui::Key, text: &str) {
    harness.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_string()));
    harness.step();
    harness.run_steps(2);
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

/// A click the way a hand makes one: the pointer arrives, settles for a frame or two, the
/// button goes down, is held a moment, and comes back up.
///
/// Every step is its own frame, because that is how a window is told about a click - and a
/// press and a release crammed into one frame is a gesture no hand ever made, which is what
/// makes it worth writing this out.
fn click_like_a_hand(harness: &mut Harness<'_>, at: egui::Pos2, modifiers: egui::Modifiers) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.run_steps(2);
}

/// The same, carried somewhere before the button comes up: the pointer arrives, presses,
/// travels in steps, and lets go where it ends.
fn drag_like_a_hand(
    harness: &mut Harness<'_>,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) {
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(from));
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers,
    });
    harness.run_steps(2);

    for step in 1..=6 {
        let towards = from + (to - from) * (step as f32 / 6.0);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(towards));
        harness.step();
    }
    // A few frames held where it ends: the slot a card is over is worked out at the end of a
    // frame and taken up by the next, and the cards making room for it walk there.
    harness.run_steps(8);

    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.run_steps(2);
}

/// Hold modifier keys down, or let them up again - they stay as they are put until the next
/// call, the way a key held over a whole gesture does.
fn press_modifiers(harness: &mut Harness<'_>, modifiers: egui::Modifiers) {
    harness
        .input_mut()
        .events
        .push(egui::Event::ModifiersChanged(modifiers));
    harness.run_steps(2);
}

/// Press at one point, sweep to another, release - one pointer gesture, several frames.
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
