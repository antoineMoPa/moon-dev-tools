//! The dated lines DONE draws between its days: where they stand, and what they say.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::{moontasks::store, native::theme::ThemeMode};

use super::{app_for, seeded_fixture, settle};

/// Reading down DONE, a line stands above the first card of each day and says which day it
/// is; a card written before the board kept arrival days gets no line and falls under the
/// day above it.
#[test]
fn done_draws_a_dated_line_wherever_the_day_changes() {
    let fixture = seeded_fixture("board-day-lines");
    let today = store::now_unix();
    // Noon UTC on the 14th of November 2023, which is that day in every zone a board is read
    // in; the test does not get to pick the machine's zone.
    let november_14_2023 = 1_699_963_200;
    for (task_id, title, position, arrived) in [
        ("ship-it-1111", "Ship it", 0, Some(today)),
        ("test-it-2222", "Test it", 1, Some(today)),
        ("plan-it-3333", "Plan it", 2, Some(november_14_2023)),
        ("old-one-4444", "Old one", 3, None),
    ] {
        let arrived = arrived.map_or(String::new(), |arrived| {
            format!("  \"entered_column_at_unix\": {arrived},\n")
        });
        fixture.write(
            &format!(".moontasks/{task_id}/metadata.json"),
            &format!(
                "{{\n  \"title\": \"{title}\",\n  \"status\": \"done\",\n  \
                 \"created_at_unix\": 1700000000,\n{arrived}  \"position\": {position},\n  \
                 \"resources\": []\n}}\n"
            ),
        );
    }

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // How many cards the board has read, so the picture is only asked about once it is whole.
    let cards = Arc::new(Mutex::new(0usize));
    let cards_in_ui = Arc::clone(&cards);

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
            *cards_in_ui.lock().expect("poisoned") = app.model.board.tasks.len();
        });

    assert!(
        settle(&mut harness, || *cards.lock().expect("poisoned") == 4),
        "the board should have read its four cards"
    );
    harness.run_steps(2);

    let top_of = |label: &str| -> f32 {
        let mut found: Vec<egui::Rect> = harness
            .query_all_by_label(label)
            .map(|node| node.rect())
            .collect();
        assert_eq!(found.len(), 1, "expected one {label:?}, found {found:?}");
        found.pop().expect("one rect").min.y
    };

    // Two lines, one per day - and none for the card with no day, nor a second `today`.
    let today_line = top_of("today");
    let november_line = top_of("November 14, 2023");
    assert!(
        harness.query_by_label("yesterday").is_none(),
        "no card arrived yesterday"
    );

    // Each line stands above the first card of its day and below the last of the day before.
    assert!(
        today_line < top_of("Ship it"),
        "the day's line is above its first card"
    );
    assert!(
        top_of("Test it") < november_line,
        "and below the last card of the day above"
    );
    assert!(november_line < top_of("Plan it"));
    assert!(
        top_of("Plan it") < top_of("Old one"),
        "the undated card reads under the day above it"
    );
}
