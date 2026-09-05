//! What a language server behind a file tab adds, tested on files nothing serves.
//!
//! The suite must never start a real language server - a machine running this has
//! rust-analyzer on it, and a cold index would turn a forty-second suite into a minutes-long
//! one. So every test here switches the language-server side of the pane on and then points
//! it at a file no server has ever heard of, which is both the honest way to test the wiring
//! without a server and the state most of a repo is really in: the pane asks once, hears that
//! nothing serves the file, and everything carries on exactly as it did before there was
//! anything to ask.
//!
//! There is one exception, at the bottom, and it is marked `#[ignore]` for exactly that
//! reason: `typing_in_a_real_crate_offers_what_rust_analyzer_knows` starts rust-analyzer on a
//! throwaway cargo crate and types into it. Everything above proves each half in isolation -
//! the pane's side here, the protocol's side in `src/lsp/tests.rs`, the popup's in the editor
//! crate - and none of it proves the whole of it works in one window, which is the only thing
//! anyone is actually shipping. So that test exists, and is run on purpose with
//! `cargo test --lib -- --ignored` rather than as part of a suite that has to stay fast.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::{panes::Pane, theme::ThemeMode};

use super::{Fixture, app_for, press_key, settle};

/// ⌘-clicking a name in a file lands on where it is defined: the repo is searched for the
/// name, the one line that reads as its definition wins over the line that only calls it, and
/// that file opens at it with the name marked - all without a language server anywhere.
#[test]
fn a_command_clicked_name_opens_the_file_that_defines_it() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-definition");
    fixture.write(
        "src/lib.rs",
        "pub fn greet(name: &str) -> String {\n    format!(\"hello {name}\")\n}\n",
    );
    // The call is the first thing on the first line, so the click has a name to land on at a
    // spot that does not depend on how the text was laid out.
    fixture.write(
        "src/main.rs",
        "greet(\"moon\");\n\nfn main() {\n    println!(\"{}\", greet(\"moon\"));\n}\n",
    );
    fixture.commit("Add the library and the program");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let defining = Arc::new(AtomicBool::new(false));
    let defining_in_ui = Arc::clone(&defining);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "src/main.rs");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            loaded_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
            // The whole landing, not just the tab appearing: the file that defines the name
            // is open, its text has arrived, and the find bar has laid the name out and been
            // told how many it found. Settling on the tab alone snapshots a frame in the
            // middle of all that, where the mark is not yet the current one.
            let defined = app.model.layout.panes().find_map(|(pane_id, pane)| {
                matches!(pane, Pane::File { file_path, .. } if file_path == "src/lib.rs")
                    .then_some(pane_id)
            });
            defining_in_ui.store(
                defined.is_some_and(|pane_id| {
                    app.model
                        .file_editors
                        .get(&pane_id)
                        .is_some_and(|editor| editor.content_for_test().is_some())
                }) && app
                    .model
                    .find
                    .as_ref()
                    .is_some_and(|find| find.total > 0 && !find.pending),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file tab never opened"
    );
    harness.run_steps(2);

    // The first name on the first line of the text, a couple of points in from the corner of
    // the text area so the click is inside the word rather than on the margin beside it.
    let word = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    super::click_like_a_hand(&mut harness, word, egui::Modifiers::COMMAND);
    super::press_modifiers(&mut harness, egui::Modifiers::NONE);

    assert!(
        settle(&mut harness, || defining.load(Ordering::Relaxed)),
        "the file that defines the name never opened"
    );

    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    // Long enough for the scroll bar the landing brought up to fade back out. Three frames
    // catches it mid-fade, which is a different picture every time the machine is a different
    // speed.
    harness.run_steps(30);
    harness.snapshot("file-definition-jump");
}

/// The same click in a file no language server serves, with the language-server side of the
/// pane switched on: the pane asks whether anything serves the file, hears that nothing does,
/// and the repo search answers exactly as it does with the servers switched off. Nothing is
/// said about it either - a file with no server behind it is the normal state of most of a
/// repo, not a fault - and no server is started, which is why the file is a `.txt` and the one
/// that defines the name is a `.sh`.
#[test]
fn a_command_clicked_name_still_lands_when_no_language_server_serves_the_file() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-definition-unserved");
    fixture.write("scripts/deploy.sh", "function greet() {\n  echo hello\n}\n");
    // The name is the first thing on the first line, so the click has something to land on at
    // a spot that does not depend on how the text was laid out.
    fixture.write(
        "notes/plan.txt",
        "greet is what the script says at the end\nand nothing else here says it\n",
    );
    fixture.commit("Add the script and the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    let defining = Arc::new(AtomicBool::new(false));
    let defining_in_ui = Arc::clone(&defining);
    let said_anything = Arc::new(AtomicBool::new(false));
    let said_anything_in_ui = Arc::clone(&said_anything);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "notes/plan.txt");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            // The tests keep the language-server side of a file pane off, so that a `.rs`
            // pane in a test does not start the rust-analyzer this machine really has. This
            // one is about that side of it, on files nothing serves.
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            loaded_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
            if !app.model.toasts.is_empty() {
                said_anything_in_ui.store(true, Ordering::Relaxed);
            }
            // The whole landing, not just the tab appearing: the file that defines the name
            // is open, its text has arrived, and the find bar has laid the name out and been
            // told how many it found.
            let defined = app.model.layout.panes().find_map(|(pane_id, pane)| {
                matches!(pane, Pane::File { file_path, .. } if file_path == "scripts/deploy.sh")
                    .then_some(pane_id)
            });
            defining_in_ui.store(
                defined.is_some_and(|pane_id| {
                    app.model
                        .file_editors
                        .get(&pane_id)
                        .is_some_and(|editor| editor.content_for_test().is_some())
                }) && app
                    .model
                    .find
                    .as_ref()
                    .is_some_and(|find| find.total > 0 && !find.pending),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file tab never opened"
    );
    harness.run_steps(2);

    let word = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::press_modifiers(&mut harness, egui::Modifiers::COMMAND);
    super::click_like_a_hand(&mut harness, word, egui::Modifiers::COMMAND);
    super::press_modifiers(&mut harness, egui::Modifiers::NONE);

    assert!(
        settle(&mut harness, || defining.load(Ordering::Relaxed)),
        "the file that defines the name never opened"
    );
    assert!(
        !said_anything.load(Ordering::Relaxed),
        "a file with no language server behind it should be searched quietly"
    );
}

/// Typing in a file no language server serves, with the language-server side of the pane
/// switched on: the pane asks whether anything serves the file, hears that nothing does, and
/// nothing is ever offered to finish the word being typed - which is the state most of a repo
/// is in. What matters as much is that the typing itself is untouched: the completion box
/// takes the arrows, Enter, Tab and Escape only while a list is on screen, and a file with no
/// list must type exactly as it did before there was one.
///
/// It is a `.txt` deliberately. The suite must never start a real language server, and a file
/// nothing serves is the honest way to test this half without one.
#[test]
fn typing_in_a_file_no_language_server_serves_offers_nothing_and_still_types() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-completion-unserved");
    fixture.write("notes/plan.txt", "greet\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // The whole of the end state, not a stage on the way to it: the text has arrived, the
    // pane has heard back that nothing serves the file, the letters that were typed are in
    // the text, and nothing is being offered to finish them with.
    let typed = Arc::new(Mutex::new(String::new()));
    let typed_in_ui = Arc::clone(&typed);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let offered_anything = Arc::new(AtomicBool::new(false));
    let offered_anything_in_ui = Arc::clone(&offered_anything);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let session_id = app.model.root_session_id.clone();
                app.open_file_pane(&session_id, "notes/plan.txt");
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            // The tests keep this side of a file pane off, so a `.rs` pane in a test does not
            // start the rust-analyzer this machine really has. This one is about that side of
            // it, on a file nothing serves.
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            let pane = app.model.file_editors.values().next();
            loaded_in_ui.store(
                pane.is_some_and(|editor| editor.content_for_test().is_some()),
                Ordering::Relaxed,
            );
            if let Some(editor) = pane {
                *typed_in_ui.lock().expect("the typed text is not shared") =
                    editor.text_for_test().to_string();
                if editor.rows_offered_for_test() > 0 {
                    offered_anything_in_ui.store(true, Ordering::Relaxed);
                }
                settled_in_ui.store(
                    editor.heard_no_server_for_test()
                        && editor.text_for_test().starts_with("greeting"),
                    Ordering::Relaxed,
                );
            }
        });

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );
    harness.run_steps(2);

    // Into the first line of the text - a couple of points in from its corner, so the click
    // lands in the word rather than on the blank line under it - and then to the end of the
    // word already there.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    for (key, letter) in [
        (egui::Key::I, "i"),
        (egui::Key::N, "n"),
        (egui::Key::G, "g"),
    ] {
        super::type_letter(&mut harness, key, letter);
    }

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the letters should have been typed into a file nothing serves, saw {:?}",
        typed.lock().expect("the typed text is not shared").clone()
    );
    // Well past the pause a word is asked about after, so a question that was going to go out
    // has had every chance to.
    harness.run_steps(60);
    assert!(
        !offered_anything.load(Ordering::Relaxed),
        "a file with no language server behind it should offer nothing to finish a word with"
    );
}

/// A real crate, a real rust-analyzer, and a word half typed: the one test that proves the
/// whole of this works in one window rather than each half working on its own.
///
/// Marked `#[ignore]` the way the tests in `src/lsp/tests.rs` are, and for the same reason:
/// it starts a language server and waits for it to read a project, which is tens of seconds
/// on a cold one and belongs nowhere near a suite that has to stay fast. Everything it needs
/// is in the fixture - a `Cargo.toml`, one file, no dependencies - so the only thing it asks
/// of the machine is that rust-analyzer is installed on it.
///
/// It waits on states rather than on frame counts, because there are three of them in a row
/// and every one of them takes as long as the machine takes: the server has to finish
/// indexing, the pane's document sync has to land the typed text on it, and only then is the
/// question about the caret asked at all.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn typing_in_a_real_crate_offers_what_rust_analyzer_knows() {
    use egui_kittest::kittest::Queryable as _;

    /// How long the server is given to read the fixture. rust-analyzer on a crate this small
    /// is seconds rather than tens of them, but a cold toolchain on a busy machine is not.
    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    /// How long the typed word is given to reach the server and come back answered: the
    /// document sync's pause, then the completion's, then a round trip.
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-completion-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    // The half-typed member access is on the first line, so the caret is put on it by clicking
    // into the corner of the text and pressing End - no counting of lines, and no dependence
    // on how the text was laid out. What it is half-typing is two lines further down.
    fixture.write(
        "src/lib.rs",
        "pub fn call_it(greeter: &Greeter) -> String { greeter.\n}\n\npub struct Greeter {\n    pub name: String,\n}\n\nimpl Greeter {\n    pub fn greet_loudly(&self) -> String {\n        format!(\"HELLO {}\", self.name)\n    }\n\n    pub fn greet_quietly(&self) -> String {\n        format!(\"hello {}\", self.name)\n    }\n}\n",
    );
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // What the server says about the file, read out every frame so a failure says how far it
    // got rather than only that it never arrived.
    let status = Arc::new(Mutex::new(String::from("not asked yet")));
    let status_in_ui = Arc::clone(&status);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let labels = Arc::new(Mutex::new(Vec::<String>::new()));
    let labels_in_ui = Arc::clone(&labels);

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
            // The one place in the ui suite that means this: a real `.rs` file, with the
            // pane's language-server side on, really does start rust-analyzer.
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);

            let session_id = app.model.root_session_id.clone();
            if let Ok(said) = app.tasks.backend().lsp_status(&session_id, "src/lib.rs") {
                *status_in_ui.lock().expect("the status is not shared") = format!("{said:?}");
                ready_in_ui.store(said == crate::api::LspStatus::Ready, Ordering::Relaxed);
            }
            if let Some(editor) = app.model.file_editors.values().next() {
                loaded_in_ui.store(editor.content_for_test().is_some(), Ordering::Relaxed);
                let offered = editor.labels_offered_for_test();
                if !offered.is_empty() {
                    *labels_in_ui.lock().expect("the labels are not shared") = offered;
                }
            }
        });

    /// Step the window until something is true, or give up after a while. `settle`'s own
    /// twenty seconds is nowhere near long enough for a server reading a project.
    fn wait(harness: &mut Harness<'_>, patience: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the file never loaded"
    );

    let started = Instant::now();
    let indexed = wait(&mut harness, INDEXING_TAKES_AT_MOST, || {
        ready.load(Ordering::Relaxed)
    });
    println!(
        "rust-analyzer was {} after {:.1}s",
        status.lock().expect("the status is not shared"),
        started.elapsed().as_secs_f32()
    );
    assert!(
        indexed,
        "rust-analyzer never finished reading the crate - it got as far as {}",
        status.lock().expect("the status is not shared")
    );

    // Into the first line and to the end of it, which is the dot the member access is waiting
    // on, then the first two letters of the method's name.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    for (key, letter) in [(egui::Key::G, "g"), (egui::Key::R, "r")] {
        super::type_letter(&mut harness, key, letter);
    }

    let typing = Instant::now();
    let offered = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        !labels.lock().expect("the labels are not shared").is_empty()
    });
    let offered_labels = labels.lock().expect("the labels are not shared").clone();
    println!(
        "offered {} rows {:.1}s after the typing stopped: {:?}",
        offered_labels.len(),
        typing.elapsed().as_secs_f32(),
        offered_labels
    );
    assert!(
        offered,
        "typing in a served file offered nothing at all, with the server {}",
        status.lock().expect("the status is not shared")
    );
    assert!(
        offered_labels
            .iter()
            .any(|label| label.starts_with("greet_loudly")),
        "expected the crate's own method among what was offered, saw {offered_labels:?}"
    );
}
