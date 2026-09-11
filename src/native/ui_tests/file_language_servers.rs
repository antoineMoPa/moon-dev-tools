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
//! throwaway cargo crate and types into it. (The ⌘-click's own real-server test lives beside
//! the review that makes it, in `crate::native::ui_tests::diff_selection`, and is ignored for
//! the same reason.) Everything above proves each half in isolation -
//! the pane's side here, the protocol's side in the `moon_lsp` crate, the popup's in the editor
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

/// ⌘-clicking a name in a file no language server serves says so and jumps nowhere.
///
/// The only thing that answers a ⌘-click is a language server - see
/// [`crate::native::definition`] - so a file nothing serves has no answer to give, and the
/// click says which file and which name rather than going quiet. Silence would read as the
/// gesture being broken, which for most of a repo it would be a lie about.
///
/// The pane's language-server side is switched on for this - the suite keeps it off, so that a
/// `.rs` pane in a test does not start the rust-analyzer the machine really has - and pointed
/// at a `.txt` file, which nothing serves. That is both the honest way to test this without a
/// server and the state most of a repo is really in.
///
/// There is no picture of it, where the jump this replaced had one: what the click produces is a
/// message, and the window stamps every message it logs with the time of day, so a snapshot of
/// one is a different image every second.
#[test]
fn a_command_clicked_name_in_a_file_no_server_serves_says_so_and_jumps_nowhere() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-definition-unserved");
    // The name is the first thing on the first line, so the click has something to land on at
    // a spot that does not depend on how the text was laid out.
    fixture.write(
        "notes/plan.txt",
        "greet is what the script says at the end\nand nothing else here says it\n",
    );
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // Every message the window has up, and how many file tabs are open: the whole of what the
    // click is allowed to have done.
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);
    let file_panes = Arc::new(Mutex::new(0usize));
    let file_panes_in_ui = Arc::clone(&file_panes);

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
            // The tests keep the language-server side of a file pane off, so that a `.rs` pane
            // in a test does not start the rust-analyzer this machine really has. This one is
            // about that side of it, on a file nothing serves.
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
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
            *file_panes_in_ui.lock().expect("the count is not shared") = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| matches!(pane, Pane::File { .. }))
                .count();
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

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| text.contains("no language server serves notes/plan.txt"))
    };
    assert!(
        settle(&mut harness, told),
        "the click should have said which file nothing serves, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert_eq!(
        *file_panes.lock().expect("the count is not shared"),
        1,
        "a click nothing could answer should not have opened a tab"
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
/// Marked `#[ignore]` the way `moon_lsp`'s real-server tests are, and for the same reason:
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
    // What the pane was told opens a list on its own, read out the same way: it is asked once
    // the server is ready and carried back over the backend like every other answer.
    let triggers = Arc::new(Mutex::new(Vec::<char>::new()));
    let triggers_in_ui = Arc::clone(&triggers);
    // What is in the buffer, read out every frame: what taking a row put there is the other
    // half of what this test is about.
    let typed = Arc::new(Mutex::new(String::new()));
    let typed_in_ui = Arc::clone(&typed);

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
                *triggers_in_ui.lock().expect("the triggers are not shared") =
                    editor.triggers_for_test();
                *typed_in_ui.lock().expect("the text is not shared") =
                    editor.text_for_test().to_string();
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

    // What rust-analyzer itself said opens a list, carried from its `initialize` reply through
    // the backend to this pane. Asked once the server is ready, so it lands a frame or two
    // after the indexing does.
    let carried = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        !triggers
            .lock()
            .expect("the triggers are not shared")
            .is_empty()
    });
    let carried_triggers = triggers
        .lock()
        .expect("the triggers are not shared")
        .clone();
    println!("the pane was told {carried_triggers:?} open a list");
    assert!(
        carried,
        "the pane was never told what opens a list, with the server {}",
        status.lock().expect("the status is not shared")
    );
    assert!(
        carried_triggers.contains(&'.') && carried_triggers.contains(&':'),
        "expected rust-analyzer's own triggers to reach the pane, saw {carried_triggers:?}"
    );

    // Into the first line and to the end of it, which is the dot the member access is waiting
    // on. That dot is the whole of the card: nothing has been typed towards a name, and the
    // list has to come up anyway, on what the server said the dot means.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    harness.run_steps(2);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);

    let after_the_dot = Instant::now();
    let opened_on_the_dot = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        !labels.lock().expect("the labels are not shared").is_empty()
    });
    let on_the_dot = labels.lock().expect("the labels are not shared").clone();
    println!(
        "the dot alone offered {} rows {:.1}s later: {:?}",
        on_the_dot.len(),
        after_the_dot.elapsed().as_secs_f32(),
        on_the_dot
    );
    assert!(
        opened_on_the_dot,
        "a caret behind a dot offered nothing at all, with the server {}",
        status.lock().expect("the status is not shared")
    );
    assert!(
        on_the_dot
            .iter()
            .any(|label| label.starts_with("greet_loudly")),
        "expected the members of what is left of the dot, saw {on_the_dot:?}"
    );

    // And then the first two letters of the method's name, which is the other half: the same
    // place asked about again, with something to filter the answer against this time.
    labels.lock().expect("the labels are not shared").clear();
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

    // And taking one writes the call rather than the bare name: a method is called, so the
    // parentheses go in with it - see [`egui_moon_code_ide::calling`]. Which of the two methods
    // is highlighted is the list's business, so the assertion is about the shape of what landed
    // rather than about which name it is.
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    harness.run_steps(4);
    let landed = typed.lock().expect("the text is not shared").clone();
    let called = landed.lines().next().unwrap_or_default().to_string();
    assert!(
        called.starts_with("pub fn call_it(greeter: &Greeter) -> String { greeter.greet_")
            && called.ends_with("()"),
        "taking a method left the line reading {called:?}"
    );
}

/// F2 in a file no language server serves says so, and the palette does not offer a rename
/// there at all. A rename is a direct request, and a key that does nothing reads as a broken
/// key - so it is answered, and names the file.
#[test]
fn f2_in_a_file_no_server_serves_says_nothing_in_it_can_be_renamed() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-rename-unserved");
    fixture.write("notes/plan.txt", "greet is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let offered_a_rename = Arc::new(AtomicBool::new(false));
    let offered_in_ui = Arc::clone(&offered_a_rename);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            if crate::native::palette::commands_for(&app)
                .iter()
                .any(|command| command.title == "rename symbol")
            {
                offered_in_ui.store(true, Ordering::Relaxed);
            }
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .toasts
                .iter()
                .map(|toast| toast.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::F2, egui::Modifiers::NONE);

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text.contains(
                    "no language server serves notes/plan.txt, so nothing in it can be renamed",
                )
            })
    };
    assert!(
        settle(&mut harness, told),
        "F2 should have said nothing in the file can be renamed, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert!(
        !offered_a_rename.load(Ordering::Relaxed),
        "the palette offered a rename in a file nothing serves"
    );
}

/// A real crate, a real rust-analyzer, and a rename across two files: the one test that proves
/// the whole of it works in one window. The file open in a tab takes the edit in its buffer and
/// is left unsaved; the file nobody has open is written.
///
/// `#[ignore]`d for the reason the completion test above is: it starts a language server and
/// waits for it to read a project.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn renaming_in_a_real_crate_edits_the_open_tab_and_writes_the_file_nobody_has_open() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-rename-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    // The name is on the first line, so the caret is put on it by clicking into the corner of
    // the text and stepping right - no dependence on how the text was laid out.
    fixture.write(
        "src/lib.rs",
        "pub fn greet() -> u32 {\n    1\n}\n\nmod other;\n",
    );
    fixture.write(
        "src/other.rs",
        "pub fn twice() -> u32 {\n    crate::greet() * 2\n}\n",
    );
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let naming = Arc::new(Mutex::new(None::<String>));
    let naming_in_ui = Arc::clone(&naming);
    let text = Arc::new(Mutex::new(String::new()));
    let text_in_ui = Arc::clone(&text);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);

            // Ready as the tab has it, which is what a rename is started against: the tab
            // asks after its server now and then rather than every frame, so it hears a moment
            // after the server itself says so.
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.server_heard().status() == crate::api::LspStatus::Ready),
                Ordering::Relaxed,
            );
            let palette = &app.model.palette;
            *naming_in_ui.lock().expect("the palette is not shared") = (palette.open
                && palette.mode == crate::native::palette::PaletteMode::Rename)
                .then(|| palette.query.clone());
            if let Some(editor) = app.model.file_editors.values().next() {
                *text_in_ui.lock().expect("the text is not shared") =
                    editor.text_for_test().to_string();
            }
            // The whole log rather than the toasts: a toast fades long before a wait on a
            // server gives up, and the failure has to say what was said.
            let front_is_the_file = crate::native::renaming::front_tab_renames(&app);
            let step = app
                .model
                .renaming
                .as_ref()
                .map_or("no rename", |renaming| renaming.step_for_test());
            let mut logged: Vec<String> = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
            logged.push(format!(
                "[rename: {step}; front tab renames: {front_is_the_file}]"
            ));
            *said_in_ui.lock().expect("the messages are not shared") = logged;
        });

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
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );

    // Onto the `r` of `greet`, and F2.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::Home, egui::Modifiers::NONE);
    for _ in 0.."pub fn g".len() {
        press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    }
    press_key(&mut harness, egui::Key::F2, egui::Modifiers::NONE);

    let offered = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        naming.lock().expect("the palette is not shared").is_some()
    });
    assert!(
        offered,
        "the palette never opened on the name, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert_eq!(
        naming.lock().expect("the palette is not shared").clone(),
        Some("greet".to_string()),
        "the palette opens on the name the server gave"
    );

    // The name is selected, so what is typed replaces it.
    for (key, letter) in [
        (egui::Key::H, "h"),
        (egui::Key::E, "e"),
        (egui::Key::L, "l"),
        (egui::Key::L, "l"),
        (egui::Key::O, "o"),
    ] {
        super::type_letter(&mut harness, key, letter);
    }
    assert_eq!(
        naming.lock().expect("the palette is not shared").clone(),
        Some("hello".to_string())
    );
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    let other = fixture.root.join("src/other.rs");
    let renamed = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        text.lock()
            .expect("the text is not shared")
            .starts_with("pub fn hello() -> u32 {")
            && std::fs::read_to_string(&other).is_ok_and(|other| other.contains("crate::hello()"))
            // Said once the file nobody had open is written, which is a moment after it is.
            && said
                .lock()
                .expect("the messages are not shared")
                .iter()
                .any(|said| said.starts_with("renamed greet to hello: 2 places in 2 files"))
    });
    assert!(
        renamed,
        "the rename never landed: the tab reads {:?}, other.rs reads {:?}, and the window said {:?}",
        text.lock().expect("the text is not shared").clone(),
        std::fs::read_to_string(&other),
        said.lock().expect("the messages are not shared").clone()
    );
    // The open tab is edited and not saved: its file on disk still has the old name.
    assert!(
        std::fs::read_to_string(fixture.root.join("src/lib.rs"))
            .expect("the file is there")
            .starts_with("pub fn greet()"),
        "the open tab's file should be left for the person to save"
    );
    assert!(
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|said| said.starts_with("renamed greet to hello: 2 places in 2 files")),
        "saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// Shift-F12 in a file no language server serves says so, and names what could not be found -
/// the same answer F2 gives, for the uses of a name.
#[test]
fn shift_f12_in_a_file_no_server_serves_says_the_uses_cannot_be_found() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-references-unserved");
    fixture.write("notes/plan.txt", "greet is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::F12, egui::Modifiers::SHIFT);

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text == "no language server serves notes/plan.txt, so the uses of a name cannot be found"
            })
    };
    assert!(
        settle(&mut harness, told),
        "shift-F12 should have said the uses cannot be found, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// A real crate, a real rust-analyzer: shift-F12 on a name lists everywhere it is used in the
/// palette, each row reading what its line says - the declaration among them.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn finding_the_uses_of_a_name_in_a_real_crate_lists_them_in_the_palette() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-references-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn greet() -> u32 {\n    1\n}\n\nmod other;\n",
    );
    fixture.write(
        "src/other.rs",
        "pub fn twice() -> u32 {\n    crate::greet() * 2\n}\n",
    );
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let rows = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let rows_in_ui = Arc::clone(&rows);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.server_heard().status() == crate::api::LspStatus::Ready),
                Ordering::Relaxed,
            );
            let palette = &app.model.palette;
            *rows_in_ui.lock().expect("the rows are not shared") = match &palette.places {
                Some(found) if palette.open => found
                    .places
                    .iter()
                    .map(|place| {
                        (
                            format!("{}:{}", place.file_path, place.line_number),
                            place.line_text.clone().unwrap_or_default(),
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            };
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

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
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::Home, egui::Modifiers::NONE);
    for _ in 0.."pub fn g".len() {
        press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    }
    press_key(&mut harness, egui::Key::F12, egui::Modifiers::SHIFT);

    let listed = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        rows.lock().expect("the rows are not shared").len() == 2
    });
    let seen = rows.lock().expect("the rows are not shared").clone();
    println!("listed {seen:?}");
    assert!(
        listed,
        "expected the declaration and the one use, saw {seen:?} and the window said {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
    assert!(
        seen.contains(&(
            "src/other.rs:2".to_string(),
            "    crate::greet() * 2".to_string()
        )),
        "the use in other.rs reads its line, saw {seen:?}"
    );
}

/// ⌥⇧F in a file no language server serves says so rather than leaving the text as it was
/// without a word - which would read as the file already being formatted.
#[test]
fn alt_shift_f_in_a_file_no_server_serves_says_it_cannot_be_formatted() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-format-unserved");
    fixture.write("notes/plan.txt", "greet   is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::ALT | egui::Modifiers::SHIFT,
    );

    let told = || {
        said.lock()
            .expect("the messages are not shared")
            .iter()
            .any(|text| {
                text == "no language server serves notes/plan.txt, so it cannot be formatted"
            })
    };
    assert!(
        settle(&mut harness, told),
        "⌥⇧F should have said the file cannot be formatted, saw {:?}",
        said.lock().expect("the messages are not shared").clone()
    );
}

/// A real crate, a real rust-analyzer: ⌥⇧F lays the tab's text out the way rustfmt does, in
/// the buffer and unsaved.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn formatting_a_file_in_a_real_crate_lays_the_tab_out_the_way_rustfmt_does() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-format-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write("src/lib.rs", "pub fn   greet( )->u32{1}\n");
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let text = Arc::new(Mutex::new(String::new()));
    let text_in_ui = Arc::clone(&text);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            ready_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.server_heard().status() == crate::api::LspStatus::Ready),
                Ordering::Relaxed,
            );
            if let Some(editor) = app.model.file_editors.values().next() {
                *text_in_ui.lock().expect("the text is not shared") =
                    editor.text_for_test().to_string();
            }
            *said_in_ui.lock().expect("the messages are not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

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
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::ALT | egui::Modifiers::SHIFT,
    );

    let formatted = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        *text.lock().expect("the text is not shared") == "pub fn greet() -> u32 {\n    1\n}\n"
    });
    assert!(
        formatted,
        "the tab reads {:?}, and the window said {:?}",
        text.lock().expect("the text is not shared").clone(),
        said.lock().expect("the messages are not shared").clone()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("src/lib.rs")).expect("the file is there"),
        "pub fn   greet( )->u32{1}\n",
        "the format is the tab's to save"
    );
}

/// A real crate, a real rust-analyzer: a type error in a saved file comes back as a diagnostic
/// on the tab, and resting the pointer on a word brings back what the server says about it.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn a_real_crate_s_errors_reach_the_tab_and_a_resting_pointer_is_told_about_the_word() {
    use egui_kittest::kittest::Queryable as _;

    const INDEXING_TAKES_AT_MOST: Duration = Duration::from_secs(120);
    const CHECKING_TAKES_AT_MOST: Duration = Duration::from_secs(90);
    const ANSWERING_TAKES_AT_MOST: Duration = Duration::from_secs(30);

    let fixture = Fixture::new("file-diagnostics-real");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    );
    fixture.write("src/lib.rs", "pub fn greet() -> u32 {\n    \"one\"\n}\n");
    fixture.commit("Add the crate");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    let found = Arc::new(Mutex::new(Vec::<String>::new()));
    let found_in_ui = Arc::clone(&found);
    let told = Arc::new(Mutex::new(None::<String>));
    let told_in_ui = Arc::clone(&told);

    let pub_keyword = egui_moon_editor::Word {
        text: "pub".to_string(),
        at: egui_moon_editor::TextPoint {
            offset: 0,
            line: 0,
            column: 0,
        },
    };
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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            if let Some(editor) = app.model.file_editors.values().next() {
                ready_in_ui.store(
                    editor.server_heard().status() == crate::api::LspStatus::Ready,
                    Ordering::Relaxed,
                );
                *found_in_ui.lock().expect("not shared") = editor.diagnostics_for_test();
                *told_in_ui.lock().expect("not shared") = editor
                    .hovering()
                    .showing(Some(&pub_keyword))
                    .map(str::to_string);
            }
        });

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
        wait(&mut harness, INDEXING_TAKES_AT_MOST, || ready
            .load(Ordering::Relaxed)),
        "rust-analyzer never finished reading the crate"
    );

    // A space at the end of the first line and a save, which is what sets `cargo check` off.
    press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::End, egui::Modifiers::NONE);
    super::type_letter(&mut harness, egui::Key::Space, " ");
    press_key(&mut harness, egui::Key::S, egui::Modifiers::COMMAND);

    let checked = wait(&mut harness, CHECKING_TAKES_AT_MOST, || {
        found
            .lock()
            .expect("not shared")
            .iter()
            .any(|message| message.contains("u32"))
    });
    println!("found {:?}", found.lock().expect("not shared"));
    assert!(
        checked,
        "the type error never reached the tab, saw {:?}",
        found.lock().expect("not shared").clone()
    );

    // The pointer rests on `pub`, the first word of the file.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(first_line));
    let answered = wait(&mut harness, ANSWERING_TAKES_AT_MOST, || {
        told.lock().expect("not shared").is_some()
    });
    println!("told {:?}", told.lock().expect("not shared"));
    assert!(answered, "resting on `pub` brought nothing back");
}

/// ⌘. in a file no language server serves says it has no code actions, rather than opening an
/// empty list or nothing at all.
#[test]
fn command_period_in_a_file_no_server_serves_says_it_has_no_code_actions() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = Fixture::new("file-actions-unserved");
    fixture.write("notes/plan.txt", "greet is what the script says\n");
    fixture.commit("Add the note");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let settled = Arc::new(AtomicBool::new(false));
    let settled_in_ui = Arc::clone(&settled);
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let said_in_ui = Arc::clone(&said);

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
            for editor in app.model.file_editors.values_mut() {
                editor.asks_language_servers_for_test();
            }
            app.draw(ui);
            settled_in_ui.store(
                app.model
                    .file_editors
                    .values()
                    .any(|editor| editor.heard_no_server_for_test()),
                Ordering::Relaxed,
            );
            *said_in_ui.lock().expect("not shared") = app
                .model
                .messages
                .iter()
                .map(|message| message.text.clone())
                .collect();
        });

    assert!(
        settle(&mut harness, || settled.load(Ordering::Relaxed)),
        "the pane never heard that nothing serves its file"
    );
    let first_line = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .min
        + egui::vec2(12.0, 10.0);
    super::click_at(&mut harness, first_line);
    press_key(&mut harness, egui::Key::Period, egui::Modifiers::COMMAND);

    let told = || {
        said.lock().expect("not shared").iter().any(|text| {
            text == "no language server serves notes/plan.txt, so it has no code actions"
        })
    };
    assert!(
        settle(&mut harness, told),
        "⌘. should have said the file has no code actions, saw {:?}",
        said.lock().expect("not shared").clone()
    );
}

/// Shared by the real-server tests below: a crate with one file open in a tab, stepped until
/// rust-analyzer has read it, with what the tests read out of the window each frame.
struct RealTab {
    harness: Harness<'static>,
    text: Arc<Mutex<String>>,
    palette_rows: Arc<Mutex<Vec<String>>>,
    signature: Arc<Mutex<Option<(String, Option<String>)>>>,
    said: Arc<Mutex<Vec<String>>>,
    _fixture: Fixture,
}

impl RealTab {
    fn open(name: &str, lib: &str) -> Self {
        let fixture = Fixture::new(name);
        fixture.write(
            "Cargo.toml",
            "[package]\nname = \"greeting\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        );
        fixture.write("src/lib.rs", lib);
        fixture.commit("Add the crate");

        let mut app = app_for(&fixture.root, ThemeMode::Dark);
        let opened = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let text = Arc::new(Mutex::new(String::new()));
        let palette_rows = Arc::new(Mutex::new(Vec::new()));
        let signature = Arc::new(Mutex::new(None));
        let said = Arc::new(Mutex::new(Vec::new()));
        let (opened_in_ui, ready_in_ui, text_in_ui, rows_in_ui, signature_in_ui, said_in_ui) = (
            Arc::clone(&opened),
            Arc::clone(&ready),
            Arc::clone(&text),
            Arc::clone(&palette_rows),
            Arc::clone(&signature),
            Arc::clone(&said),
        );
        let harness = Harness::builder()
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
                for editor in app.model.file_editors.values_mut() {
                    editor.asks_language_servers_for_test();
                }
                app.draw(ui);
                if let Some(editor) = app.model.file_editors.values().next() {
                    ready_in_ui.store(
                        editor.server_heard().status() == crate::api::LspStatus::Ready,
                        Ordering::Relaxed,
                    );
                    *text_in_ui.lock().expect("not shared") = editor.text_for_test().to_string();
                    *signature_in_ui.lock().expect("not shared") =
                        editor.signing().showing().map(|signature| {
                            (
                                signature.label.clone(),
                                signature
                                    .active_parameter
                                    .clone()
                                    .map(|range| signature.label[range].to_string()),
                            )
                        });
                }
                *rows_in_ui.lock().expect("not shared") =
                    match crate::native::code_actions::offered(&app.model) {
                        Some(actions) if app.model.palette.open => {
                            actions.iter().map(|action| action.title.clone()).collect()
                        }
                        _ => Vec::new(),
                    };
                *said_in_ui.lock().expect("not shared") = app
                    .model
                    .messages
                    .iter()
                    .map(|message| message.text.clone())
                    .collect();
            });
        let mut tab = Self {
            harness,
            text,
            palette_rows,
            signature,
            said,
            _fixture: fixture,
        };
        assert!(
            tab.wait(Duration::from_secs(120), || ready.load(Ordering::Relaxed)),
            "rust-analyzer never finished reading the crate"
        );
        tab
    }

    fn wait(&mut self, patience: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            self.harness.step();
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Click into the first line, go to its start, and step right `columns` times.
    fn caret_to(&mut self, line: usize, columns: usize) {
        use egui_kittest::kittest::Queryable as _;
        press_key(&mut self.harness, egui::Key::Escape, egui::Modifiers::NONE);
        let first_line = self
            .harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .rect()
            .min
            + egui::vec2(12.0, 10.0);
        super::click_at(&mut self.harness, first_line);
        press_key(&mut self.harness, egui::Key::Home, egui::Modifiers::NONE);
        for _ in 0..line {
            press_key(
                &mut self.harness,
                egui::Key::ArrowDown,
                egui::Modifiers::NONE,
            );
        }
        press_key(&mut self.harness, egui::Key::Home, egui::Modifiers::NONE);
        for _ in 0..columns {
            press_key(
                &mut self.harness,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
            );
        }
    }

    fn said(&self) -> Vec<String> {
        self.said.lock().expect("not shared").clone()
    }
}

/// A real crate, a real rust-analyzer: ⌘. on a `let` lists what rust-analyzer offers there,
/// and picking "insert explicit type" puts the type into the tab.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn a_code_action_picked_from_the_palette_edits_the_tab() {
    let mut tab = RealTab::open(
        "file-actions-real",
        "pub fn five() -> i32 {\n    let x = 5;\n    x\n}\n",
    );
    tab.caret_to(1, "    let ".len());
    press_key(
        &mut tab.harness,
        egui::Key::Period,
        egui::Modifiers::COMMAND,
    );

    let rows = Arc::clone(&tab.palette_rows);
    let offered = tab.wait(Duration::from_secs(30), || {
        rows.lock()
            .expect("not shared")
            .iter()
            .any(|title| title.to_lowercase().contains("explicit type"))
    });
    println!("offered {:?}", rows.lock().expect("not shared"));
    assert!(
        offered,
        "rust-analyzer's explicit type action was not offered, saw {:?} and the window said {:?}",
        rows.lock().expect("not shared").clone(),
        tab.said()
    );

    // Narrowed down to that one row, and taken.
    for (key, letter) in [
        (egui::Key::E, "e"),
        (egui::Key::X, "x"),
        (egui::Key::P, "p"),
        (egui::Key::L, "l"),
        (egui::Key::I, "i"),
        (egui::Key::C, "c"),
        (egui::Key::I, "i"),
        (egui::Key::T, "t"),
    ] {
        super::type_letter(&mut tab.harness, key, letter);
    }
    press_key(&mut tab.harness, egui::Key::Enter, egui::Modifiers::NONE);

    let text = Arc::clone(&tab.text);
    let typed = tab.wait(Duration::from_secs(10), || {
        text.lock().expect("not shared").contains("let x: i32 = 5;")
    });
    assert!(
        typed,
        "the tab reads {:?}, and the window said {:?}",
        text.lock().expect("not shared").clone(),
        tab.said()
    );
}

/// A real crate, a real rust-analyzer: the caret inside a call's parentheses brings up the
/// function's signature, with the parameter being typed picked out.
#[test]
#[ignore = "starts a real rust-analyzer and waits for it to index a crate"]
fn the_signature_of_the_call_being_typed_comes_up_with_its_parameter_picked_out() {
    let mut tab = RealTab::open(
        "file-signature-real",
        "pub fn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n\npub fn three() -> u32 {\n    add(1, )\n}\n",
    );
    tab.caret_to(5, "    add(1, ".len());

    let signature = Arc::clone(&tab.signature);
    let shown = tab.wait(Duration::from_secs(30), || {
        signature.lock().expect("not shared").is_some()
    });
    let seen = signature.lock().expect("not shared").clone();
    println!("signature {seen:?}");
    assert!(
        shown,
        "no signature came up, the window said {:?}",
        tab.said()
    );
    let (label, active) = seen.expect("shown");
    assert!(label.contains("fn add(a: u32, b: u32)"), "{label}");
    assert_eq!(active.as_deref(), Some("b: u32"));
}
