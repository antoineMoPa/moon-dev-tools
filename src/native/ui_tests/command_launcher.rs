//! The line `⌘⇧!` puts up in the status bar: what it remembers of what it started, and the two
//! ways a command comes back into it - the arrows, and ctrl+r.
//!
//! Driven through the real window from the chord onwards, so the history these read is the one
//! the line itself wrote when Enter was pressed.

use std::sync::{Arc, Mutex};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::theme::ThemeMode;

use super::{app_for, press_key, seeded_fixture, settle};

/// What the line holds, or `None` while the strip is the status bar again.
type Line = Arc<Mutex<Option<String>>>;

fn line_of(line: &Line) -> Option<String> {
    line.lock().expect("poisoned").clone()
}

/// A window on a repo that has been read, and what its line holds after each frame.
fn window(name: &str) -> (Harness<'static>, Line, super::Fixture) {
    let fixture = seeded_fixture(name);
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let line: Line = Arc::new(Mutex::new(None));
    let line_in_ui = Arc::clone(&line);
    let read = Arc::new(Mutex::new(false));
    let read_in_ui = Arc::clone(&read);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_ui(move |ui| {
            app.draw(ui);
            *line_in_ui.lock().expect("poisoned") = app
                .model
                .command_launcher
                .as_ref()
                .map(|launcher| launcher.text.clone());
            // A command is started from the repo's root, which the window knows once it has
            // read the repo.
            *read_in_ui.lock().expect("poisoned") = app.model.root_repo_path().is_some();
        });
    assert!(
        settle(&mut harness, || *read.lock().expect("poisoned")),
        "the repo was never read"
    );
    (harness, line, fixture)
}

/// `⌘⇧!`, and the frames it takes the line to have the keyboard.
fn open_the_line(harness: &mut Harness<'_>) {
    press_key(
        harness,
        egui::Key::Num1,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
}

fn type_text(harness: &mut Harness<'_>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_string()));
    harness.step();
    harness.run_steps(2);
}

/// Type a command into a fresh line and start it.
fn start(harness: &mut Harness<'_>, line: &Line, command: &str) {
    open_the_line(harness);
    type_text(harness, command);
    assert_eq!(line_of(line).as_deref(), Some(command));
    press_key(harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(line_of(line), None, "Enter should have put the line away");
}

/// Arrow up brings back what was started, newest first, and arrow down walks forward again to
/// the line that was being typed.
#[test]
fn the_arrows_walk_through_what_the_line_started_before() {
    let (mut harness, line, _fixture) = window("launcher-arrows");
    start(&mut harness, &line, "echo one");
    start(&mut harness, &line, "echo two");

    open_the_line(&mut harness);
    type_text(&mut harness, "half");
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("echo two"));
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("echo one"));
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("echo one"), "the oldest");
    press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("echo two"));
    press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("half"));

    // A command brought back is typed onto the end of, the way it is in a shell.
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    type_text(&mut harness, " three");
    assert_eq!(line_of(&line).as_deref(), Some("echo two three"));
}

/// ctrl+r searches what was started for what is typed, a second ctrl+r looks further back,
/// and Enter starts what was found - which makes it the newest command in the history.
#[test]
fn ctrl_r_searches_what_the_line_started_before() {
    let (mut harness, line, _fixture) = window("launcher-search");
    start(&mut harness, &line, "echo one");
    start(&mut harness, &line, "true");
    start(&mut harness, &line, "echo two");

    open_the_line(&mut harness);
    type_text(&mut harness, "half");
    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    type_text(&mut harness, "echo");
    assert!(
        harness.query_by_label("echo two").is_some(),
        "the newest command with `echo` in it should be what the search shows"
    );
    assert_eq!(
        line_of(&line).as_deref(),
        Some("half"),
        "the line waits behind the search"
    );
    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    assert!(harness.query_by_label("echo one").is_some());

    // Escape leaves the search for the line as it was, and the line stays up.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("half"));
    assert!(harness.query_by_label("echo one").is_none());

    // An arrow along the line takes what was found into it, to be changed first.
    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    type_text(&mut harness, "tr");
    assert!(harness.query_by_label("true").is_some());
    press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("true"));

    // Enter on a search starts what it found.
    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    type_text(&mut harness, "one");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(line_of(&line), None, "Enter should have started it");

    open_the_line(&mut harness);
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(
        line_of(&line).as_deref(),
        Some("echo one"),
        "what was just started is the newest command"
    );
}

/// A line that has never started anything still has the login shell's history under it, for
/// the arrows and for ctrl+r alike.
#[test]
fn a_line_that_started_nothing_brings_back_the_shells_history() {
    let (mut harness, line, _fixture) = window("launcher-shell-history");
    crate::native::command_launcher::write_shell_history_for_test(
        "git status\n: 1700000000:0;cargo build --release\n",
    );

    open_the_line(&mut harness);
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("cargo build --release"));
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some("git status"));

    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    type_text(&mut harness, "cargo");
    assert!(harness.query_by_label("cargo build --release").is_some());
}

/// Enter on a search that found nothing starts nothing, and leaves the search up to be retyped.
#[test]
fn enter_on_a_search_that_found_nothing_starts_nothing() {
    let (mut harness, line, _fixture) = window("launcher-search-nothing");
    start(&mut harness, &line, "echo one");

    open_the_line(&mut harness);
    press_key(&mut harness, egui::Key::R, egui::Modifiers::CTRL);
    type_text(&mut harness, "zzz");
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(line_of(&line).as_deref(), Some(""), "the line is still up");

    // Still searching, with the keyboard: one more letter is one more letter of the query.
    press_key(&mut harness, egui::Key::Backspace, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::Backspace, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::Backspace, egui::Modifiers::NONE);
    type_text(&mut harness, "one");
    assert!(harness.query_by_label("echo one").is_some());
}
