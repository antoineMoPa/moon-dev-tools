//! The switch at the end of the palette's line, between the file finder and the content
//! search.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::native::{palette::PaletteMode, theme::ThemeMode};

use super::{app_for, press_key, seeded_fixture, settle, type_letter};

/// `content` at the end of the finder's line turns it into the content search, keeping what
/// is typed: the name that found no file is looked for in the files' text instead. `files`
/// turns it back, and so does Tab, either way.
#[test]
fn the_switch_on_the_line_turns_the_finder_into_the_content_search_and_back() {
    use egui_kittest::kittest::Queryable as _;

    let fixture = seeded_fixture("palette-search-switch");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    let ready = Arc::new(AtomicBool::new(false));
    let ready_in_ui = Arc::clone(&ready);
    // What the line is on, and what its two searches have found once over.
    let found = Arc::new(Mutex::new((
        PaletteMode::Commands,
        Vec::<String>::new(),
        Vec::<(String, usize)>::new(),
    )));
    let found_in_ui = Arc::clone(&found);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            let palette = &app.model.palette;
            *found_in_ui.lock().expect("poisoned") = (
                palette.mode,
                palette
                    .files
                    .matches
                    .clone()
                    .into_iter()
                    .filter(|_| palette.files.done)
                    .collect(),
                palette
                    .contents
                    .matches
                    .iter()
                    .filter(|_| palette.contents.done)
                    .map(|line| (line.file_path.clone(), line.line_number))
                    .collect(),
            );
            ready_in_ui.store(
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || ready.load(Ordering::Relaxed)),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(&mut harness, egui::Key::P, egui::Modifiers::COMMAND);
    for (key, letter) in [
        (egui::Key::A, "a"),
        (egui::Key::N, "n"),
        (egui::Key::S, "s"),
        (egui::Key::W, "w"),
        (egui::Key::E, "e"),
        (egui::Key::R, "r"),
    ] {
        type_letter(&mut harness, key, letter);
    }
    assert!(
        settle(&mut harness, || {
            let found = found.lock().expect("poisoned");
            found.0 == PaletteMode::Files && found.1.is_empty()
        }),
        "no file of the repo is named after the constant - is ag installed?"
    );

    harness.get_by_label("content").click();
    assert!(
        settle(&mut harness, || {
            let found = found.lock().expect("poisoned");
            found.0 == PaletteMode::Contents && found.2 == vec![("src/extra.rs".to_string(), 1)]
        }),
        "the switch should have looked for the same text in the files: {:?}",
        found.lock().expect("poisoned")
    );

    harness.get_by_label("files").click();
    assert!(
        settle(&mut harness, || {
            let found = found.lock().expect("poisoned");
            found.0 == PaletteMode::Files && found.2.is_empty()
        }),
        "the switch should have put the line back on the files: {:?}",
        found.lock().expect("poisoned")
    );

    press_key(&mut harness, egui::Key::Tab, egui::Modifiers::NONE);
    assert!(
        settle(&mut harness, || {
            let found = found.lock().expect("poisoned");
            found.0 == PaletteMode::Contents && found.2 == vec![("src/extra.rs".to_string(), 1)]
        }),
        "tab should have flipped the switch to the content search: {:?}",
        found.lock().expect("poisoned")
    );
}
