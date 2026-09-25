//! The board under a finger: a touch screen scrolls it rather than carrying its cards.

use egui_kittest::kittest::Queryable as _;

use super::{CARDS, board_of, click_at, finger, title_of};

/// A phone has room for a column and a bit: a finger carried sideways across a card scrolls
/// the board to the columns past the edge, and neither picks the card up nor marks it.
#[test]
fn a_finger_carried_across_a_card_scrolls_the_board() {
    let (mut harness, seen, _fixture) = board_of("board-touch-swipe", &[]);
    harness.set_size(egui::vec2(390.0, 800.0));
    harness.run_steps(4);

    let (card, ..) = CARDS[0];
    let from = title_of(&harness, card);
    let to = from - egui::vec2(200.0, 0.0);
    finger(&mut harness, from, egui::TouchPhase::Start);
    for step in 1..=10 {
        let at = from + (to - from) * (step as f32 / 10.0);
        finger(&mut harness, at, egui::TouchPhase::Move);
    }
    finger(&mut harness, to, egui::TouchPhase::End);
    harness.run_steps(4);

    let now = title_of(&harness, card);
    // Less than the whole 200: the board ends a column and a bit past the phone's edge.
    assert!(
        from.x - now.x > 100.0,
        "the board should have scrolled under the finger, the card went from {from:?} to {now:?}"
    );
    assert!(
        (from.y - now.y).abs() < 1.0,
        "and only sideways, the card went from {from:?} to {now:?}"
    );
    let seen = seen.lock().expect("poisoned").clone();
    assert_eq!(seen.column_of(card), "todo", "the card was not carried off");
    assert!(seen.marked.is_empty(), "nor marked: {:?}", seen.marked);
    assert!(!seen.page_open, "nor opened");
}

/// A tap is still a tap: the card's page opens.
#[test]
fn a_tap_on_a_card_opens_it() {
    let (mut harness, seen, _fixture) = board_of("board-touch-tap", &[]);
    harness.set_size(egui::vec2(390.0, 800.0));
    harness.run_steps(4);

    let (card, ..) = CARDS[0];
    let at = title_of(&harness, card);
    finger(&mut harness, at, egui::TouchPhase::Start);
    finger(&mut harness, at, egui::TouchPhase::End);
    harness.run_steps(4);

    let seen = seen.lock().expect("poisoned").clone();
    assert_eq!(seen.pages_open, vec![card.to_string()]);
}

/// A finger rested on a card for a moment has hold of it: carried to the next column, the card
/// goes with it. The board's clock steps a quarter of a second a frame, so three frames are a
/// rest long enough.
#[test]
fn a_finger_rested_on_a_card_carries_it_to_another_column() {
    let (mut harness, seen, _fixture) = board_of("board-touch-hold", &[]);
    harness.set_size(egui::vec2(700.0, 800.0));
    harness.run_steps(4);

    let (card, ..) = CARDS[0];
    let from = title_of(&harness, card);
    let to = from + egui::vec2(330.0, 0.0);
    finger(&mut harness, from, egui::TouchPhase::Start);
    for _ in 0..3 {
        finger(&mut harness, from, egui::TouchPhase::Move);
    }
    for step in 1..=6 {
        let at = from + (to - from) * (step as f32 / 6.0);
        finger(&mut harness, at, egui::TouchPhase::Move);
    }
    harness.run_steps(4);
    finger(&mut harness, to, egui::TouchPhase::End);

    let read = std::sync::Arc::clone(&seen);
    assert!(
        super::settle(&mut harness, || read
            .lock()
            .expect("poisoned")
            .column_of(card)
            != "todo"),
        "the card should have been carried out of its column"
    );
}

/// A finger rested on a card and lifted without carrying it opens the card's menu - what a
/// right click opens on a desk, which a phone has no button for - and the menu can send the
/// card to DONE from there.
#[test]
fn a_finger_rested_on_a_card_and_lifted_opens_its_menu() {
    let (mut harness, seen, _fixture) = board_of("board-touch-hold-menu", &[]);
    harness.set_size(egui::vec2(390.0, 800.0));
    harness.run_steps(4);

    let (card, ..) = CARDS[0];
    let at = title_of(&harness, card);
    finger(&mut harness, at, egui::TouchPhase::Start);
    for _ in 0..3 {
        finger(&mut harness, at, egui::TouchPhase::Move);
    }
    finger(&mut harness, at, egui::TouchPhase::End);
    harness.run_steps(2);

    let move_to_done = harness
        .query_by_label("move to DONE")
        .expect("the held card's menu should be up")
        .rect()
        .center();
    {
        let seen = seen.lock().expect("poisoned");
        assert!(!seen.page_open, "a hold is not a tap: the card's page stays shut");
        assert_eq!(seen.column_of(card), "todo", "nor was the card carried anywhere");
    }

    click_at(&mut harness, move_to_done);
    let read = std::sync::Arc::clone(&seen);
    assert!(
        super::settle(&mut harness, || read.lock().expect("poisoned").column_of(card) == "done"),
        "the menu should have sent the card to DONE"
    );
}
