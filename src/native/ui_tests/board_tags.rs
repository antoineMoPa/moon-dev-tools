//! A card's tags: the pills at its foot, and the box they are edited in.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::native::theme::ThemeMode;

use super::{app_for, press_key, seeded_fixture, settle};

const PARSER: &str = "write-the-parser-1111";
const LOGIN: &str = "fix-the-login-page-2222";

/// The tags a task has, read off its `metadata.json` - which is where the box writes, and so
/// what says whether a press did anything at all.
fn tags_of(path: &std::path::Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("expected the task's metadata");
    let metadata: serde_json::Value =
        serde_json::from_str(&text).expect("expected the metadata to be json");
    metadata["tags"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .map(|tag| tag.as_str().expect("expected a tag").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A tag is a pill at the foot of the card, in a color the tag's own letters settle on, and
/// `[tags]` opens the box it is edited in, inside the card: a pill apiece with the mark that
/// takes it off, a place to type the next one, and the rest of the board's tags under it to be
/// pressed on.
///
/// Every way in and out of that box is exercised here, because they are what the box is: a
/// tag pressed on from the board's own, tags typed and put on by a space or by Enter - faster
/// than the board answers - backspace on an empty box taking the last one off, the mark on a
/// pill taking that one off, and a press off the card shutting the box.
#[test]
fn a_cards_tags_are_edited_in_the_box_its_tags_button_opens() {
    let fixture = seeded_fixture("board-tags");
    fixture.write(
        &format!(".moontasks/{PARSER}/metadata.json"),
        "{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"position\": 0,\n  \
         \"tags\": [\"parser\", \"needs-tests\"],\n  \"resources\": []\n}\n",
    );
    fixture.write(
        &format!(".moontasks/{LOGIN}/metadata.json"),
        "{\n  \"title\": \"Fix the login page\",\n  \"status\": \"in_progress\",\n  \
         \"created_at_unix\": 1700000001,\n  \"position\": 0,\n  \
         \"tags\": [\"bug\"],\n  \"resources\": []\n}\n",
    );
    let metadata_path = fixture
        .root
        .join(format!(".moontasks/{PARSER}/metadata.json"));

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    app.set_theme(ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_in_ui = Arc::clone(&loaded);
    // What the board last answered this card's tags are, watched from out here. Every press
    // in the box is made against these, so each one waits for the answer to the one before
    // it: a hand cannot press twice inside a frame and a round trip, and a test can.
    let shown: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let shown_in_ui = Arc::clone(&shown);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            loaded_in_ui.store(
                app.model.board.loaded && app.model.board.tasks.len() == 2,
                Ordering::Relaxed,
            );
            if let Some(task) = app.model.board.tasks.iter().find(|task| task.id == PARSER) {
                task.tags
                    .clone_into(&mut shown_in_ui.lock().expect("poisoned"));
            }
        });

    /// Wait for the board to be drawing these tags, and check they are what was written.
    macro_rules! expect_tags {
        ($want:expr, $said:literal) => {{
            let want: &[&str] = &$want;
            assert!(
                settle(&mut harness, || *shown.lock().expect("poisoned") == want),
                concat!($said, ", the board is drawing {:?}"),
                shown.lock().expect("poisoned")
            );
            assert_eq!(tags_of(&metadata_path), want, concat!($said, " on disk"));
        }};
    }

    assert!(
        settle(&mut harness, || loaded.load(Ordering::Relaxed)),
        "the board never read the two tasks out of .moontasks"
    );
    harness.run_steps(3);
    harness.snapshot("moontasks-tags");

    // The pointer onto the card, which is what brings out the row `[tags]` stands in.
    let card = harness
        .ctx
        .read_response(crate::native::board::cards::card_drag_id(PARSER))
        .expect("expected the parser card to have been drawn")
        .rect;
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(
            card.right() - 12.0,
            card.bottom() + 12.0,
        )));
    harness.run_steps(3);
    harness
        .get_all_by_label("[tags]")
        .next()
        .expect("expected the first card to offer [tags]")
        .click();
    harness.run_steps(3);

    // The box, in the card: its two tags as pills, and the other card's `bug` offered under
    // them.
    assert!(harness.query_by_label("parser").is_some());
    assert!(harness.query_by_label("needs-tests").is_some());
    assert!(
        harness
            .query_all_by_label("bug")
            .any(|node| node.rect().center().x < card.right()),
        "the rest of the board's tags should be offered under the box"
    );
    // A caret blinking in the box would make the image differ run to run.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(2);
    harness.snapshot("moontasks-tags-menu");

    // A tag the board already uses goes on with one press of the pill under the box - the
    // `bug` in this card's column, not the one at the foot of the login card beside it.
    harness
        .get_all_by_label("bug")
        .find(|node| node.rect().center().x < card.right())
        .expect("expected `bug` to be offered under the parser card's box")
        .click();
    expect_tags!(
        ["parser", "needs-tests", "bug"],
        "pressing a tag under the box should have put it on the card"
    );

    // Tags nobody has used yet are typed. The keyboard is still in the box after the press
    // before, so they are typed without reaching for the box. A space finishes a word the way
    // Enter does, and each lands in the spelling the board keeps tags in rather than the one
    // it was typed in. None of it waits for the board to answer the tag before: each is put
    // after the last, not over it.
    // One frame apart, which is quicker than any write comes back.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("Review ".to_string()));
    harness.step();
    harness.input_mut().events.extend([
        egui::Event::Text("Urgent".to_string()),
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.step();
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("ui".to_string()));
    harness.run_steps(2);
    expect_tags!(
        ["parser", "needs-tests", "bug", "review", "urgent"],
        "tags typed one after the other should all have gone on"
    );
    assert!(
        harness.query_by_label("ui").is_none(),
        "a word still being typed should not have gone on as a pill"
    );
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    expect_tags!(
        ["parser", "needs-tests", "bug", "review", "urgent", "ui"],
        "Enter should have put the word being typed on"
    );

    // Backspace with nothing typed takes the last tag off, the way it does in every field
    // that holds things rather than letters.
    press_key(&mut harness, egui::Key::Backspace, egui::Modifiers::NONE);
    expect_tags!(
        ["parser", "needs-tests", "bug", "review", "urgent"],
        "backspace in an empty box should have taken the last tag off"
    );

    // And the mark on a pill takes that one off, whichever of them it is.
    harness
        .get_all_by_label("×")
        .next()
        .expect("expected the first pill to carry the mark that takes it off")
        .click();
    expect_tags!(
        ["needs-tests", "bug", "review", "urgent"],
        "the mark on the first pill should have taken that tag off"
    );

    // A press off the card shuts the box, and the card is back to its pills.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(900.0, 600.0)));
    harness.run_steps(2);
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: egui::pos2(900.0, 600.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(2);
    }
    assert!(
        harness.query_by_label("×").is_none(),
        "a press off the card should have shut its tag box"
    );

    // A press on one of the card's pills opens the box again, with the keyboard in it.
    harness.get_by_label("urgent").click();
    harness.run_steps(3);
    assert!(
        harness.query_all_by_label("×").next().is_some(),
        "a press on a card's tag should have opened its tag box"
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("later ".to_string()));
    harness.run_steps(2);
    expect_tags!(
        ["needs-tests", "bug", "review", "urgent", "later"],
        "the box a tag opened should have had the keyboard in it"
    );
}
