//! The palette's rows when what they have to say is wider than the palette.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;

use crate::native::theme::ThemeMode;

use super::{app_for, press_key, seeded_fixture, settle, type_letter};

/// A file far enough down the tree that its path is wider than the palette.
const DEEP_FILE: &str = "src/a_folder_with_quite_a_long_name/another_folder_with_quite_a_long_name/yet_another_folder_with_a_long_name/the_last_folder_before_the_file/wide.rs";

/// A text the window painted: the glyphs that made it to the screen, and where they are once
/// the clip they were painted under has had its say.
struct Painted {
    text: String,
    rect: egui::Rect,
}

fn painted_texts(shape: &egui::Shape, clip_rect: egui::Rect, painted: &mut Vec<Painted>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                painted_texts(shape, clip_rect, painted);
            }
        }
        egui::Shape::Text(text) => painted.push(Painted {
            text: text.galley.rows.iter().map(|row| row.text()).collect(),
            rect: text.visual_bounding_rect().intersect(clip_rect),
        }),
        _ => {}
    }
}

/// A matching line longer than the palette is wide, in a file whose path is too: both are cut
/// to the palette instead of running out of its right edge over the window behind it. The
/// line keeps its start, and the path keeps its end - the file and the line number, which are
/// what the row is read for.
#[test]
fn a_row_wider_than_the_palette_is_cut_to_fit_inside_it() {
    let fixture = seeded_fixture("palette-long-rows");
    let long_line = format!(
        "pub const WIDEST: &str = \"{}\";\n",
        "a line that goes on for a good while longer than the palette is wide ".repeat(3)
    );
    fixture.write(DEEP_FILE, &long_line);
    // Committed, so the review behind the palette has no diff of the same line to paint.
    fixture.commit("Add a file far down the tree");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);

    // Whether the review is up, what the search answered for, and where the palette drew.
    let state = Arc::new(Mutex::new((false, None::<String>, None::<egui::Rect>)));
    let state_in_ui = Arc::clone(&state);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            app.draw(ui);
            *state_in_ui.lock().expect("poisoned") = (
                app.model
                    .review_ref(&app.model.root_session_id)
                    .is_some_and(|review| review.payload.is_some()),
                app.model
                    .palette
                    .contents
                    .searched
                    .as_ref()
                    .filter(|_| app.model.palette.contents.done)
                    .map(|asked| asked.query.clone()),
                app.model.palette.rect,
            );
        });

    assert!(
        settle(&mut harness, || state.lock().expect("poisoned").0),
        "the review never loaded"
    );
    harness.run_steps(2);

    press_key(
        &mut harness,
        egui::Key::F,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    );
    for (key, letter) in [
        (egui::Key::W, "w"),
        (egui::Key::I, "i"),
        (egui::Key::D, "d"),
        (egui::Key::E, "e"),
        (egui::Key::S, "s"),
        (egui::Key::T, "t"),
    ] {
        type_letter(&mut harness, key, letter);
    }
    assert!(
        settle(&mut harness, || {
            state.lock().expect("poisoned").1.as_deref() == Some("widest")
        }),
        "the search never answered for what was typed - is ag installed?"
    );
    // The palette sizes itself to its rows over a frame or two.
    harness.run_steps(3);

    let palette_rect = state
        .lock()
        .expect("poisoned")
        .2
        .expect("the palette should be on screen");
    let mut painted = Vec::new();
    for clipped in &harness.output().shapes {
        painted_texts(&clipped.shape, clipped.clip_rect, &mut painted);
    }

    let line = painted
        .iter()
        .find(|painted| painted.text.starts_with("pub const WIDEST"))
        .expect("the matching line should be the row's first line");
    assert!(
        palette_rect.contains_rect(line.rect),
        "the matching line runs out of the palette: {:?} is not inside {palette_rect:?}",
        line.rect
    );
    assert!(
        line.text.ends_with('…'),
        "a line that was cut should say so: {}",
        line.text
    );

    let place = painted
        .iter()
        .find(|painted| painted.text.ends_with("wide.rs:1"))
        .expect("the row's second line should end on the file and the line number");
    assert!(
        palette_rect.contains_rect(place.rect),
        "the path runs out of the palette: {:?} is not inside {palette_rect:?}",
        place.rect
    );
    assert!(
        place.text.starts_with('…'),
        "a path that was cut should say so: {}",
        place.text
    );
}
