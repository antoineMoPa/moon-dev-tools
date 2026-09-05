//! What the pointer is doing to the board's cards.
//!
//! A card is not a widget. It is a box with widgets in it - a title, its notes, its runs, the
//! `[start]` menu, the mark that deletes it - and what the board needs to know about a press is
//! whether it was a click on one of those, a click on the card itself, or the card being picked
//! up and carried somewhere. `egui` answers the first of those, which is its job; the other two
//! are answered here, from the pointer itself: where the press went down, on what, and how far
//! it has carried since.
//!
//! Reading the pointer rather than laying invisible widgets over the cards is what keeps the
//! rules the board's own. A wide drag target takes the clicks of everything beneath it, is
//! dragged the instant a press lands on it, and takes the keyboard while it is at it - three
//! rules that are right for a handle and wrong for a card.

use egui::{Modifiers, Pos2};

use crate::native::model::BoardState;

/// How far the pointer carries from where it went down before the press is a card being picked
/// up rather than a click on it. Further than `egui`'s own few points: a careful click is a
/// slow press by a hand that is not quite still, and every one of those is a click.
const GRAB_TRAVEL: f32 = 10.0;

/// The buttons drawn inside something the board is watching for presses - a card, a column -
/// and whether one of them has the pointer down on it.
///
/// A card is a box with buttons in it, and the board has to tell a press on one of them from a
/// press on the card itself. Every button on a card answers through [`Controls::pressed`],
/// which is the whole of the bookkeeping.
pub(crate) struct Controls {
    /// Whether cmd or shift is held, when every part of a card belongs to the card.
    marking: bool,
    /// Whether one of the buttons has the pointer down on it.
    took_the_press: bool,
}

impl Controls {
    pub(crate) fn new(ui: &egui::Ui) -> Self {
        let modifiers = ui.input(|input| input.modifiers);
        Self {
            marking: modifiers.command || modifiers.shift,
            took_the_press: false,
        }
    }

    /// The same buttons drawn where no card is watching - on the task's own page, where there
    /// is nothing to mark and nothing to pick up.
    pub(crate) fn elsewhere() -> Self {
        Self {
            marking: false,
            took_the_press: false,
        }
    }

    /// A button: whether it was pressed *as a button*.
    ///
    /// While cmd or shift is held it never is. Every part of a card belongs to the card then,
    /// and the button neither acts nor takes the press away from it.
    ///
    /// Otherwise the press is recorded, so what is watching knows to keep its hands off a
    /// press that was never for it. Both halves of the press are worth recording: the pointer
    /// held down on the button, and the click a press and a release in the same breath make,
    /// which is over before "held down" was ever true.
    pub(crate) fn pressed(&mut self, response: &egui::Response) -> bool {
        if self.marking {
            return false;
        }
        self.took_the_press |= response.is_pointer_button_down_on() || response.clicked();
        response.clicked()
    }

    pub(crate) fn took_the_press(&self) -> bool {
        self.took_the_press
    }
}

/// A press in progress.
#[derive(Clone)]
pub(crate) struct Press {
    /// The card it went down on, or `None` for the board beside the cards.
    pub(crate) on: Option<String>,
    /// Whether it went down on that card's title, which is what a rename opens from.
    pub(crate) on_title: bool,
    /// Whether it went down on one of the card's own buttons. A press that stays there is the
    /// button's - it acts on the release, and `egui` sees to that - and one that carries is
    /// the card being picked up, because a press that travels was never a button being
    /// pressed.
    pub(crate) on_a_button: bool,
    pub(crate) origin: Pos2,
    /// The keys held when it went down. A gesture is read by the keys it began with, so
    /// letting go of cmd on the way to the drop changes nothing about what is being carried.
    pub(crate) modifiers: Modifiers,
    /// Whether it has carried far enough to be a card being picked up.
    pub(crate) travelled: bool,
}

/// What a press turned out to be, once the button comes back up.
pub(crate) enum Ended {
    /// Let go of where it went down: a click, on a card or on the board beside them.
    Click {
        on: Option<String>,
        on_title: bool,
        on_a_button: bool,
        modifiers: Modifiers,
    },
    /// Carried somewhere: the cards were being dragged, and this is them being let go of.
    Dropped,
}

/// Take the press, if one has just begun inside `within` and nothing else has claimed it.
///
/// Cards claim their own as they are drawn, and the board claims what is left - so a press that
/// lands on no card is a press on the board, wherever on it the pointer was. `title` is the
/// part of a card a rename opens from, and `on_a_button` says the press went down on something
/// the card carries.
///
/// Nothing is claimed through a menu or a modal standing over the board: those are drawn in
/// their own layer, and a press that lands on one belongs to it.
pub(crate) fn claim(
    board: &mut BoardState,
    ui: &egui::Ui,
    within: egui::Rect,
    on: Option<String>,
    title: egui::Rect,
    on_a_button: bool,
) {
    if board.press.is_some() {
        return;
    }
    let origin = ui.input(|input| {
        input
            .pointer
            .any_pressed()
            .then(|| input.pointer.interact_pos())
            .flatten()
    });
    // Only where the press can be seen to land: a card scrolled out of sight, or laid out past
    // the board's own edge, still has a place, and a press over there is not that card being
    // pressed. See [`BoardState::showing`].
    let Some(board_rect) = board.showing else {
        return;
    };
    let showing = within.intersect(board_rect).intersect(ui.clip_rect());
    let Some(origin) = origin.filter(|at| showing.contains(*at)) else {
        return;
    };
    // Read outside the input, which holds a lock the layers are behind.
    let over_a_menu = ui
        .ctx()
        .layer_id_at(origin)
        .is_some_and(|layer| layer.order > egui::Order::Middle);
    if over_a_menu {
        return;
    }

    board.press = Some(Press {
        on,
        on_title: title.contains(origin),
        on_a_button,
        origin,
        modifiers: ui.input(|input| input.modifiers),
        travelled: false,
    });
}

/// Follow the press: how far it has carried, and what it was when it ends.
///
/// Answers `Some` on the frame the button comes back up, and takes the press away with it.
pub(crate) fn settle(board: &mut BoardState, input: &egui::InputState) -> Option<Ended> {
    if stale(
        board.press.as_ref(),
        input.pointer.press_origin(),
        input.pointer.any_released(),
    ) {
        board.press = None;
        board.carrying = None;
        return None;
    }
    let press = board.press.as_mut()?;
    if let Some(at) = input.pointer.interact_pos().or(input.pointer.latest_pos()) {
        // Once it has carried it has carried: a hand that wanders back does not put the card
        // down again.
        press.travelled |= press.origin.distance(at) > GRAB_TRAVEL;
    }
    if !input.pointer.any_released() {
        return None;
    }

    let press = board.press.take()?;
    // A press that carried is only a drag if it had hold of a card. One that began on the board
    // beside the cards is the marks being let go of, wherever it wandered before it was.
    Some(if press.travelled && press.on.is_some() {
        Ended::Dropped
    } else {
        Ended::Click {
            on: press.on,
            on_title: press.on_title,
            on_a_button: press.on_a_button,
            modifiers: press.modifiers,
        }
    })
}

/// Whether the press being held is not the press the pointer is making.
///
/// The pointer says where the button it is holding went down; a press that says anywhere else
/// is one that ended without the board hearing about it. The board only settles a press while
/// it is being drawn, and its tab can be behind another for as long as you like - long enough
/// for the press it was holding to be let go of, another gesture made somewhere else, and that
/// one mistaken for the card being carried off.
fn stale(press: Option<&Press>, pointer_began_at: Option<Pos2>, released: bool) -> bool {
    let Some(press) = press else {
        return false;
    };
    match pointer_began_at {
        // A button is down: ours, if it went down about where we say it did. About, because the
        // pointer's account of the press and the board's are taken a moment apart and a hand
        // moves in between; further than a card is picked up by is another gesture entirely.
        Some(origin) => origin.distance(press.origin) > GRAB_TRAVEL,
        // None is down: the release is this frame's, or it happened while we were not looking.
        None => !released,
    }
}

/// The card the press has hold of, once it has carried far enough to have hold of anything.
pub(crate) fn grabbed(board: &BoardState) -> Option<(&str, Modifiers)> {
    let press = board.press.as_ref()?;
    let task_id = press.on.as_deref()?;
    press.travelled.then_some((task_id, press.modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press_on(card: &str) -> Press {
        Press {
            on: Some(card.to_string()),
            on_title: false,
            on_a_button: false,
            origin: Pos2::new(100.0, 100.0),
            modifiers: Modifiers::NONE,
            travelled: false,
        }
    }

    /// A press the pointer is no longer making is not a press at all. The board settles its
    /// press only while it is drawn, and its tab can be behind another for as long as you
    /// like - long enough for that press to be let go of and another gesture made somewhere
    /// else, which must not be mistaken for the card being carried off.
    #[test]
    fn a_press_the_pointer_is_not_making_is_let_go_of() {
        let press = press_on("one");
        let elsewhere = Pos2::new(700.0, 400.0);

        assert!(
            !stale(Some(&press), Some(press.origin), false),
            "the button is down where the press began, so it is ours"
        );
        assert!(
            !stale(
                Some(&press),
                Some(press.origin + egui::vec2(3.0, 0.0)),
                false
            ),
            "and a few points apart is the same press, told a moment later"
        );
        assert!(
            stale(Some(&press), Some(elsewhere), false),
            "a button held down from somewhere else is another gesture entirely"
        );
        assert!(
            !stale(Some(&press), None, true),
            "and the release we were waiting for is not stale, it is the answer"
        );
        assert!(
            stale(Some(&press), None, false),
            "while no button down and no release is a press that ended out of sight"
        );
    }

    /// The travel is measured from where the press went down, and remembered once it is far
    /// enough - the two questions the rest of the board asks of a press.
    #[test]
    fn a_press_becomes_a_grab_only_once_it_has_carried_far_enough() {
        let mut press = press_on("one");
        let carried_to = |press: &mut Press, x: f32| {
            press.travelled |= press.origin.distance(Pos2::new(x, 100.0)) > GRAB_TRAVEL;
            press.travelled
        };

        assert!(!carried_to(&mut press, 106.0), "a few points is a click");
        assert!(
            carried_to(&mut press, 130.0),
            "and a hand's width is a grab"
        );
        assert!(
            carried_to(&mut press, 100.0),
            "and coming back does not put the card down again"
        );
    }
}
