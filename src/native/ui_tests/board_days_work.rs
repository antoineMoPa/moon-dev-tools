//! The lines a queue is split into days by: where they stand, and when there are none.
//!
//! Every board here is of a project whose day has ended, whatever the clock says while the
//! test runs: none of its queue is today's, and the lines start at tomorrow. What today has
//! room for as its hours go by is the board's own arithmetic, tested where it is done.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::theme::ThemeMode;

use super::{Fixture, app_for, seeded_fixture, settle};

const TOMORROW: &str = "tomorrow";

/// A project whose day ends as it starts, so the day is over at any hour a test runs.
fn end_the_day(fixture: &Fixture) {
    fixture.write(".moonreview.json", "{ \"day_ends_at\": \"0:00\" }\n");
}

/// Noon UTC on a day in November 2023, which is that day in every zone a board is read in.
fn noon_on(day_of_november: u64) -> u64 {
    1_698_840_000 + (day_of_november - 1) * 86_400
}

fn write_card(fixture: &Fixture, title: &str, column: &str, position: usize, arrived: u64) {
    let task_id = format!("{}-0000", title.to_lowercase().replace(' ', "-"));
    fixture.write(
        &format!(".moontasks/{task_id}/metadata.json"),
        &format!(
            "{{\n  \"title\": \"{title}\",\n  \"status\": \"{column}\",\n  \
             \"created_at_unix\": 1700000000,\n  \"entered_column_at_unix\": {arrived},\n  \
             \"position\": {position},\n  \"resources\": []\n}}\n"
        ),
    );
}

/// The board on screen once it has read `cards` cards, and the project's file with them.
fn board_of(fixture: &Fixture, cards: usize) -> Harness<'static> {
    end_the_day(fixture);
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // How many cards the board has read, so the picture is only asked about once it is whole.
    // Nothing is read until the project's file is: the lines are drawn from when its day ends.
    let read = Arc::new(Mutex::new(0usize));
    let read_in_ui = Arc::clone(&read);

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
            if app.model.project.day_ends_at.is_some() {
                *read_in_ui.lock().expect("poisoned") = app.model.board.tasks.len();
            }
        });

    assert!(
        settle(&mut harness, || *read.lock().expect("poisoned") == cards),
        "the board should have read its {cards} cards"
    );
    harness.run_steps(2);
    harness
}

fn top_of(harness: &Harness<'_>, label: &str) -> f32 {
    let mut found: Vec<egui::Rect> = harness
        .query_all_by_label(label)
        .map(|node| node.rect())
        .collect();
    assert_eq!(found.len(), 1, "expected one {label:?}, found {found:?}");
    found.pop().expect("one rect").min.y
}

/// Three days finished one, two and nine cards, so a day's work is two: reading down TODO,
/// tomorrow's line stands over the first card and the next day's over the third. DONE holds
/// cards too and draws no such lines - it is TODO that marks a day's work on a board that
/// started from the defaults.
#[test]
fn todo_draws_a_line_over_the_first_card_of_each_day() {
    let fixture = seeded_fixture("board-days-work");
    let finished = [(1, 1), (2, 2), (3, 9)];
    let mut done = 0;
    for (day, cards) in finished {
        for _ in 0..cards {
            write_card(
                &fixture,
                &format!("Shipped {done}"),
                "done",
                done,
                noon_on(day),
            );
            done += 1;
        }
    }
    for (position, title) in ["First up", "Second up", "Third up", "Fourth up"]
        .into_iter()
        .enumerate()
    {
        write_card(&fixture, title, "todo", position, noon_on(4));
    }

    let harness = board_of(&fixture, done + 4);

    assert!(
        top_of(&harness, TOMORROW) < top_of(&harness, "First up"),
        "the day is over, so the queue starts at tomorrow"
    );
    let next_day = top_of(&harness, "in 2 days");
    assert!(
        top_of(&harness, "Second up") < next_day,
        "a day's work is the two cards under tomorrow"
    );
    assert!(
        next_day < top_of(&harness, "Third up"),
        "and the third card is for the day after"
    );
    assert!(harness.query_by_label("today").is_none());
}

/// A queue that does not fill a day has its day's line over it and nothing under it: how much
/// more the day could take is not said, since saying it invites making work up to fill it.
#[test]
fn a_short_queue_is_not_told_how_much_more_its_day_takes() {
    let fixture = seeded_fixture("board-days-work-short-queue");
    for position in 0..12 {
        write_card(
            &fixture,
            &format!("Shipped {position}"),
            "done",
            position,
            // Four cards on each of three days.
            noon_on(1 + position as u64 / 4),
        );
    }
    write_card(&fixture, "First up", "todo", 0, noon_on(4));
    write_card(&fixture, "Second up", "todo", 1, noon_on(4));

    let harness = board_of(&fixture, 14);

    assert!(
        top_of(&harness, TOMORROW) < top_of(&harness, "First up"),
        "the day is over, so the queue starts at tomorrow"
    );
    assert!(harness.query_by_label_contains("has room").is_none());
    assert!(harness.query_by_label("in 2 days").is_none());
}

/// A board that has finished fewer than ten cards has no pace worth reading, so it draws no
/// line - neither between its cards nor under the last of them.
#[test]
fn the_line_is_not_drawn_on_a_board_that_has_finished_under_ten_cards() {
    let fixture = seeded_fixture("board-days-work-under-ten");
    for position in 0..9 {
        write_card(
            &fixture,
            &format!("Shipped {position}"),
            "done",
            position,
            // Three cards on each of three days.
            noon_on(1 + position as u64 / 3),
        );
    }
    for (position, title) in ["First up", "Second up", "Third up", "Fourth up"]
        .into_iter()
        .enumerate()
    {
        write_card(&fixture, title, "todo", position, noon_on(4));
    }

    let harness = board_of(&fixture, 13);
    assert!(
        harness.query_by_label(TOMORROW).is_none() && harness.query_by_label("in 2 days").is_none(),
        "nine finished cards are not a pace to draw a line from"
    );
}

/// The lines are the column's own setting, turned off from the heading's right-click menu.
#[test]
fn the_headings_menu_turns_the_line_off() {
    let fixture = seeded_fixture("board-days-work-menu");
    for day in 1..=10 {
        write_card(
            &fixture,
            &format!("Shipped {day}"),
            "done",
            day as usize,
            noon_on(day),
        );
    }
    write_card(&fixture, "First up", "todo", 0, noon_on(4));
    write_card(&fixture, "Second up", "todo", 1, noon_on(4));

    let mut harness = board_of(&fixture, 12);
    assert!(
        top_of(&harness, "First up") < top_of(&harness, "in 2 days"),
        "each day finished one card, so the second is for the day after tomorrow"
    );

    let on_the_heading = harness.get_by_label("TODO").rect().center();
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: on_the_heading,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(3);
    harness.get_by_label("mark a day's work").click();

    // The board is redrawn from the server's answer, which is a worker thread and a poll away.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while harness.query_by_label(TOMORROW).is_some() && std::time::Instant::now() < deadline {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        harness.query_by_label(TOMORROW).is_none() && harness.query_by_label("in 2 days").is_none(),
        "the column was told to stop marking a day's work"
    );
}
