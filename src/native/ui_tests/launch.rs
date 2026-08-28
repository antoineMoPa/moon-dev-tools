//! What a window opens on: the launch screen, the title bar, and the first draw.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;
use egui_kittest::SnapshotOptions;

use crate::{
    backend::local::LocalBackend,
    native::{Launch, app::App, panes::PaneKind, theme::ThemeMode},
};

use super::{Fixture, app_for, app_for_frame, harness_with_loaded_review, seeded_fixture};

#[test]
fn the_review_window_draws_the_diff_it_was_opened_on() {
    let fixture = seeded_fixture("review");
    let app = app_for(&fixture.root, ThemeMode::Dark);

    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);

    // The window is one image; if the diff failed to draw, this is where it shows.
    harness.snapshot_options(
        "review-dark",
        &SnapshotOptions::new().output_path("docs/assets"),
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

/// A row of fixed height cannot grow, so text too long for it is cut rather than wrapped -
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

/// A window opened from a desktop launcher starts outside every repo, so it has to ask which
/// one to review - with the folder picker of the OS, since the repo is on this machine.
#[test]
fn a_window_with_no_repo_asks_which_one_to_review() {
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
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
/// front of actually opens - a board is not a review.
#[test]
fn the_launch_screen_of_the_board_does_not_offer_a_review() {
    use egui_kittest::kittest::Queryable as _;

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let mut app = App::new(
        egui::Context::default(),
        Launch {
            backend: Arc::new(LocalBackend::new(state)),
            open: None,
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
        titled.starts_with("🌚 moonreview | "),
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

    assert_eq!(titled, "🌚 moontasks | ~/prog/moonreview");
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
