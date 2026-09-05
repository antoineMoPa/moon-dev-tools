//! Opening a file in a tab of its own: what it opens on, and what it draws.
//!
//! The text tab with its fringe of line numbers, the rendered page a markdown file opens on
//! and the way back to the text, a file nobody changed opened as the file itself rather than
//! as an empty diff, and a changed image drawn as before and after. What is done to the text
//! once it is open is [`super::file_editing`]'s; searching it is [`super::finding`]'s; and
//! what a language server behind it adds is [`super::file_language_servers`]'.

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

use super::{Fixture, app_for, harness_with_loaded_review};

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

/// A markdown file opens on the rendered page, and `[edit]` is the way back to the text.
#[test]
fn a_markdown_file_opens_rendered() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-markdown");
    fixture.write(
        "NOTES.md",
        "# The plan\n\nShip it by *Friday*, with:\n\n- a heading\n- emphasis\n- this list\n",
    );
    fixture.commit("Add the notes");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "NOTES.md");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
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
    harness.run_steps(3);

    // Rendered: the page, not the text of it - so no line-number fringe, and the way back is
    // on screen.
    assert!(
        harness.query_by_label("[edit]").is_some(),
        "a rendered markdown file should offer the way back to the text"
    );
    harness.snapshot("file-pane-markdown");

    harness.get_by_label("[edit]").click();
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(3);
    assert!(
        harness.query_by_label("[preview]").is_some(),
        "the text view should offer the rendered page back"
    );
    harness.snapshot("file-pane-markdown-source");
}

/// Pointed at a file nobody has touched, the review shows the file itself rather than an
/// empty diff - `moonreview package.json` is a request to read it.
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
    // Decoding and uploading the textures takes passes of their own after the diff arrives -
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
