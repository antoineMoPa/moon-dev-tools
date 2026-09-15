//! `Tools › Work Log`: the project's journal, `.moontasks/work-log.org`, opened in a tab at a
//! new dated entry - see [`crate::native::work_log`].

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use egui_kittest::Harness;

use crate::{
    moontasks::work_log_repo_path,
    native::{app::App, panes::Pane, theme::ThemeMode, work_log::NOW_MARKER},
};

use super::{Fixture, app_for, settle};

/// A work log of two entries, ending on the marker the way one the board made does.
const JOURNAL: &str = "* Mon 27 Apr 2026 14:42:12 EDT\n==================================================\n\nfirst\n\n\n\n* Tue  1 Sep 2026 18:05:04 EDT\n==================================================\n\nsecond\n\n#now#\n";

/// What the test reads off the window after every frame: the work log's tab, if there is
/// one, and what has been said.
#[derive(Default)]
struct Seen {
    pane: Option<(egui_frames::PaneId, String)>,
    /// The review's session, read after the frame: it is settled once the review answers.
    root_session: String,
    file_panes: usize,
    loaded: bool,
    dirty: bool,
    text: String,
    toasts: Vec<String>,
}

/// A window on the project, `Tools › Work Log` pressed once the review is up, pressed again
/// whenever `again` is set, and the tab saved whenever `save` is.
fn harness_on_the_work_log(
    mut app: App,
    again: Arc<AtomicBool>,
    save: Arc<AtomicBool>,
    seen: Arc<Mutex<Seen>>,
) -> Harness<'static> {
    let pressed = Arc::new(AtomicBool::new(false));
    let file_path = work_log_repo_path();
    Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            let ready = matches!(app.model.stage, crate::native::model::Stage::Ready);
            if ready && !pressed.load(Ordering::Relaxed) {
                app.open_work_log();
                pressed.store(true, Ordering::Relaxed);
            }
            if ready && again.swap(false, Ordering::Relaxed) {
                app.open_work_log();
            }
            let pane = app
                .model
                .layout
                .panes()
                .find_map(|(pane_id, pane)| match pane {
                    Pane::File {
                        session_id,
                        file_path: open,
                        ..
                    } if *open == file_path => Some((pane_id, session_id.clone())),
                    _ => None,
                });
            if save.swap(false, Ordering::Relaxed)
                && let Some((pane_id, session_id)) = &pane
            {
                app.save_file_pane(*pane_id, session_id);
            }

            app.draw(ui);

            let editor = pane
                .as_ref()
                .and_then(|(pane_id, _)| app.model.file_editors.get(pane_id));
            *seen.lock().expect("poisoned") = Seen {
                pane: pane.clone(),
                root_session: app.model.root_session_id.clone(),
                file_panes: app
                    .model
                    .layout
                    .panes()
                    .filter(|(_, pane)| matches!(pane, Pane::File { .. }))
                    .count(),
                loaded: editor.is_some_and(|editor| editor.content_for_test().is_some()),
                dirty: pane
                    .as_ref()
                    .is_some_and(|(pane_id, _)| app.file_pane_is_dirty(*pane_id)),
                text: editor
                    .map(|editor| editor.text_for_test().to_string())
                    .unwrap_or_default(),
                toasts: app
                    .model
                    .toasts
                    .iter()
                    .map(|toast| toast.text.clone())
                    .collect(),
            };
        })
}

fn headings_in(text: &str) -> usize {
    text.lines().filter(|line| line.starts_with("* ")).count()
}

fn on_disk(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.root.join(work_log_repo_path()))
        .expect("failed to read the work log")
}

/// A project with no work log yet gets one on the first press: the file is made holding the
/// marker, and the tab opens on it with the first entry typed in and nothing saved yet.
#[test]
fn a_project_without_a_work_log_gets_one_on_the_first_press() {
    let fixture = Fixture::new("work-log-first");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");

    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut harness = harness_on_the_work_log(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&seen),
    );

    let entered = settle(&mut harness, || {
        let seen = seen.lock().expect("poisoned");
        seen.loaded && headings_in(&seen.text) == 1
    });
    let seen = seen.lock().expect("poisoned");
    assert!(
        entered,
        "no entry went in; the tab shows {:?}, said {:?}",
        seen.text, seen.toasts
    );
    let (_, session_id) = seen.pane.clone().expect("expected the work log's tab");
    assert_eq!(
        session_id, seen.root_session,
        "a file of the project's own session"
    );
    assert!(
        seen.dirty,
        "the entry is typed in, not written: the tab is dirty"
    );
    assert!(
        seen.text.starts_with("\n\n* ")
            && seen.text.ends_with(&format!(
                "\n==================================================\n\n\n{NOW_MARKER}\n"
            )),
        "the entry stands alone above the marker: {:?}",
        seen.text
    );
    assert_eq!(
        on_disk(&fixture),
        format!("{NOW_MARKER}\n"),
        "made with the marker, nothing saved until ⌘S"
    );
    assert!(
        fixture.root.join(".moontasks/.gitignore").is_file(),
        "the board's folder is made the way the board makes it, ignored by git"
    );
}

/// A work log with entries in it gets the next one above the marker, with what was there
/// untouched.
#[test]
fn the_work_log_opens_dirty_with_an_entry_above_the_marker() {
    let fixture = Fixture::new("work-log-opens");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    fixture.write(&work_log_repo_path(), JOURNAL);

    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut harness = harness_on_the_work_log(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&seen),
    );

    let entered = settle(&mut harness, || {
        let seen = seen.lock().expect("poisoned");
        seen.loaded && headings_in(&seen.text) == 3
    });
    let seen = seen.lock().expect("poisoned");
    assert!(
        entered,
        "no entry went in; the tab shows {:?}, said {:?}",
        seen.text, seen.toasts
    );
    assert!(seen.dirty);
    assert!(
        seen.text.ends_with(&format!(
            "\n==================================================\n\n\n{NOW_MARKER}\n"
        )),
        "the entry ends on its two blank lines, above the marker: {:?}",
        seen.text
    );
    assert!(
        seen.text.starts_with(JOURNAL.trim_end_matches("#now#\n")),
        "what was there is untouched"
    );
    assert_eq!(on_disk(&fixture), JOURNAL, "nothing is saved until ⌘S");
}

/// Pressed again, the same tab gets another entry rather than a second tab: it is pressed at
/// the start of every piece of work.
#[test]
fn a_second_press_adds_another_entry_to_the_same_tab() {
    let fixture = Fixture::new("work-log-again");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    fixture.write(&work_log_repo_path(), JOURNAL);

    let seen = Arc::new(Mutex::new(Seen::default()));
    let again = Arc::new(AtomicBool::new(false));
    let mut harness = harness_on_the_work_log(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::clone(&again),
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&seen),
    );
    assert!(
        settle(&mut harness, || headings_in(
            &seen.lock().expect("poisoned").text
        ) == 3),
        "the first entry never went in"
    );

    again.store(true, Ordering::Relaxed);
    let entered = settle(&mut harness, || {
        headings_in(&seen.lock().expect("poisoned").text) == 4
    });
    let seen = seen.lock().expect("poisoned");
    assert!(
        entered,
        "no second entry went in: {:?}, said {:?}",
        seen.text, seen.toasts
    );
    assert_eq!(seen.file_panes, 1, "the same tab, not a second one");
    assert!(
        seen.text.ends_with(&format!("\n\n\n{NOW_MARKER}\n")),
        "the second entry is above the marker too: {:?}",
        seen.text
    );
}

/// ⌘S writes the entry to the work log, the same as any file tab.
#[test]
fn saving_the_tab_writes_the_entry_to_the_work_log() {
    let fixture = Fixture::new("work-log-save");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    fixture.write(&work_log_repo_path(), JOURNAL);

    let seen = Arc::new(Mutex::new(Seen::default()));
    let save = Arc::new(AtomicBool::new(false));
    let mut harness = harness_on_the_work_log(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&save),
        Arc::clone(&seen),
    );
    assert!(
        settle(&mut harness, || headings_in(
            &seen.lock().expect("poisoned").text
        ) == 3),
        "the entry never went in"
    );

    save.store(true, Ordering::Relaxed);
    let written = settle(&mut harness, || headings_in(&on_disk(&fixture)) == 3);
    assert!(
        written,
        "the work log on disk still reads {:?}",
        on_disk(&fixture)
    );
    assert!(
        settle(&mut harness, || !seen.lock().expect("poisoned").dirty),
        "saved, so the tab is clean again"
    );
    let written = on_disk(&fixture);
    assert!(written.starts_with(JOURNAL.trim_end_matches("#now#\n")));
    assert!(
        written.ends_with(&format!("\n\n\n{NOW_MARKER}\n")),
        "{written:?}"
    );
}

/// A work log without the marker line has nowhere for an entry: the tab opens on it as it
/// is, and the window says what is missing rather than guessing at an end of the file.
#[test]
fn a_work_log_without_the_marker_is_opened_untouched_and_said_so() {
    let fixture = Fixture::new("work-log-no-marker");
    fixture.write("src/lib.rs", "pub fn one() {}\n");
    fixture.commit("Add the library");
    let no_marker = "* Mon 27 Apr 2026 14:42:12 EDT\n=====\n\nfirst\n";
    fixture.write(&work_log_repo_path(), no_marker);

    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut harness = harness_on_the_work_log(
        app_for(&fixture.root, ThemeMode::Dark),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&seen),
    );

    let said = settle(&mut harness, || {
        seen.lock()
            .expect("poisoned")
            .toasts
            .iter()
            .any(|toast| toast.contains("no \"#now#\" line in .moontasks/work-log.org"))
    });
    let seen = seen.lock().expect("poisoned");
    assert!(said, "nothing was said about the marker: {:?}", seen.toasts);
    assert!(seen.loaded, "the work log still opens, as it is");
    assert_eq!(seen.text, no_marker);
    assert!(!seen.dirty);
}
