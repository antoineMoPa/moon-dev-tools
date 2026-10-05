//! The board task, over the columns: where the agents and shells that are the board's own
//! rather than a card's are started and listed.

use std::sync::{Arc, Mutex};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{app_for, seeded_fixture, settle};

const CARD: &str = "write-the-parser-1111";

/// What the window has, read after every frame.
#[derive(Clone, Default)]
struct Seen {
    /// Whether the board has read its board task yet.
    board_task_read: bool,
    /// What the board task has running, by the name its row reads.
    running: Vec<String>,
    cards: Vec<String>,
    marked: Vec<String>,
    /// The task every shell tab says it belongs to.
    shell_tabs_of: Vec<Option<String>>,
}

/// `[start]` over the columns starts a shell in the board task: it is listed there, its tab
/// opens the way a task's does, and nothing about the cards changes - none is made, and none
/// is marked by the tab coming to the front.
#[test]
fn a_shell_started_over_the_columns_is_the_boards_own() {
    let fixture = seeded_fixture("board-task-shell");
    fixture.write(
        &format!(".moontasks/{CARD}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let mut opened = false;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let seen_in_ui = Arc::clone(&seen);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .build_ui(move |ui| {
            if !opened && matches!(app.model.stage, crate::native::model::Stage::Ready) {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened = true;
            }
            app.draw(ui);

            let board = &app.model.board;
            let mut shell_tabs_of = Vec::new();
            app.model.layout.find_pane(|pane| {
                if let Pane::Terminal { task_id, .. } = pane {
                    shell_tabs_of.push(task_id.clone());
                }
                false
            });
            *seen_in_ui.lock().expect("poisoned") = Seen {
                board_task_read: board.board_task.is_some(),
                running: board
                    .board_task
                    .iter()
                    .flat_map(|board_task| &board_task.resources)
                    .filter(|run| run.running)
                    .map(|run| run.label.clone())
                    .collect(),
                cards: board.tasks.iter().map(|task| task.id.clone()).collect(),
                marked: super::marked_tasks(&app),
                shell_tabs_of,
            };
        });
    let now = || seen.lock().expect("poisoned").clone();

    assert!(
        settle(&mut harness, || now().board_task_read
            && now().cards.len() == 1),
        "the board never read its board task and its card"
    );
    harness.run_steps(3);
    assert!(now().running.is_empty());

    // The board task's `[start]` is the one over the columns, above every card's.
    let mut starts: Vec<_> = harness.get_all_by_label("[start]").collect();
    starts.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    starts[0].click();
    harness.run_steps(3);
    harness.get_by_label("shell").click();

    assert!(
        settle(&mut harness, || now().running == ["board shell - 1"]
            && !now().shell_tabs_of.is_empty()),
        "the shell should be listed on the board task and open in a tab, got {:?} and {:?}",
        now().running,
        now().shell_tabs_of
    );
    harness.run_steps(3);
    let after = now();
    assert_eq!(after.shell_tabs_of, [Some("board-task".to_string())]);
    assert_eq!(after.cards, [CARD], "the board task is no card");
    assert!(
        after.marked.is_empty(),
        "the board task's tab coming forward marks no card, got {:?}",
        after.marked
    );
    // Its row over the columns: named after the board task, the way a task's shell is after its task.
    assert!(harness.query_by_label("board shell - 1").is_some());
}
