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
//! There is one exception, in [`completion`], and it is marked `#[ignore]` for exactly that
//! reason: `typing_in_a_real_crate_offers_what_rust_analyzer_knows` starts rust-analyzer on a
//! throwaway cargo crate and types into it. (The ⌘-click's own real-server test lives beside
//! the review that makes it, in `crate::native::ui_tests::diff_selection`, and is ignored for
//! the same reason.) Everything above proves each half in isolation -
//! the pane's side here, the protocol's side in the `moon_lsp` crate, the popup's in the editor
//! crate - and none of it proves the whole of it works in one window, which is the only thing
//! anyone is actually shipping. So that test exists, and is run on purpose with
//! `cargo test --lib -- --ignored` rather than as part of a suite that has to stay fast.

mod completion;
mod edits;
mod renaming;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{Fixture, app_for, press_key};

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
