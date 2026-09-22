//! `explain` on a card's `[start]` menu: a PDF about what is changed in the repo,
//! written by one of the agents - see `crate::moontasks::explainer`.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::{api::AgentKind, native::theme::ThemeMode};

use super::{app_for, seeded_fixture, settle};

/// A card offers `explain` whenever an agent is installed to write it - which agent is the
/// review's selector's to say, not the menu's. The offer is read and not pressed: pressing it
/// starts a real agent, headless, in a shell of the task.
#[test]
fn the_start_menu_offers_an_explanation_where_an_agent_is_installed() {
    const TASK: &str = "write-the-parser-4444";

    let fixture = seeded_fixture("card-explain");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = AtomicBool::new(false);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let agent_installed = Arc::new(AtomicBool::new(false));
    let agent_installed_in_ui = Arc::clone(&agent_installed);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui(move |ui| {
            if !opened.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            agent_installed_in_ui.store(
                crate::native::board::available_agents(&app)
                    .into_iter()
                    .any(|agent| agent != AgentKind::None),
                Ordering::Relaxed,
            );
            loaded_in_ui.store(app.model.board.loaded, Ordering::Relaxed);
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );

    harness.get_by_label("[start]").click();
    harness.run_steps(3);

    assert_eq!(
        harness.query_by_label("explain").is_some(),
        agent_installed.load(Ordering::Relaxed),
        "the [start] menu offers `explain` exactly when there is an agent to write it"
    );
}
