//! A card's link to its issue in a tracker kept elsewhere.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, seeded_fixture, settle};

/// A task whose `metadata.json` links it to a Linear issue shows that issue's id on its card,
/// and a click on the id hands the link to the browser.
#[test]
fn a_cards_tracker_link_reads_as_the_issue_id_and_opens_the_issue() {
    use egui_kittest::kittest::Queryable as _;

    const ISSUE: &str = "https://linear.app/acme/issue/BM-3343/remote-tracker-link-field";
    let fixture = seeded_fixture("board-tracker-link");
    fixture.write(
        ".moontasks/link-the-tracker-1111/metadata.json",
        &format!(
            "{{\n  \"title\": \"Link the tracker\",\n  \"status\": \"todo\",\n  \
             \"created_at_unix\": 1700000000,\n  \"remote_task_tracker_url\": \"{ISSUE}\",\n  \
             \"resources\": []\n}}\n"
        ),
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // What the window asked the browser to open, watched from out here.
    let handed_to_the_browser = Arc::new(Mutex::new(None::<String>));
    let handed_in_ui = Arc::clone(&handed_to_the_browser);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            let asked = ui.ctx().output(|output| {
                output.commands.iter().find_map(|command| match command {
                    egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                    _ => None,
                })
            });
            if asked.is_some() {
                *handed_in_ui.lock().expect("poisoned") = asked;
            }
        });

    // The card waits on the board being read on a worker thread.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && harness.query_by_label("BM-3343").is_none() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    harness.get_by_label("BM-3343").click();
    let handed = || handed_to_the_browser.lock().expect("poisoned").clone();
    assert!(
        settle(&mut harness, || handed().is_some()),
        "clicking the id should have opened the issue"
    );
    assert_eq!(handed().as_deref(), Some(ISSUE));
}
