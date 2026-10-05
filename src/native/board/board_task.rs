//! The board task, over the columns: the agents and shells that are the board's own rather
//! than a card's.
//!
//! A card is a task with a place on the board, and what is started from it is about that task.
//! The board task is the one task with no place on it - no column, so no card - and what is started
//! from it is about nothing in particular: an agent to move the board's cards about, or a shell
//! for work no card is for. It is listed here instead, before the columns, by the code that
//! lists a card's runs and draws a card's `[start]`: the same rows, the same marks, the same
//! menu, laid side by side rather than down a card.
//!
//! Its runs are a task's runs in every other way. Closing the tab of one leaves it running, a
//! click on its row here brings the tab back, and an agent that has ended is offered to be
//! resumed - which is what makes it somewhere to keep an agent rather than a tab to keep open.

use egui::{RichText, Ui};

use crate::native::{
    app::App,
    board::{BoardAction, gesture::Controls, resources, start},
    theme::{Palette, SMALL_SIZE},
};

/// How wide one of the board task's rows is. A column's worth would fit two to a wide window; this
/// is room for a run's name with its marks after it, and no more.
const ROW_WIDTH: f32 = 230.0;

/// The strip: what it is, its runs side by side onto as many lines as they need, and its
/// `[start]` after the last of them. Nothing, until the board has been read and its board task with it.
pub(super) fn draw(app: &App, ui: &mut Ui, palette: &Palette, actions: &mut Vec<BoardAction>) {
    let Some(board_task) = &app.model.board.board_task else {
        return;
    };
    // Nothing here is a card, so there is no card for a press to be told apart from.
    let mut controls = Controls::elsewhere();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(&board_task.title)
                .size(SMALL_SIZE)
                .color(palette.muted),
        )
        .on_hover_text(
            "The board's own agents and shells: started from here rather than from a card, \
             to work on the board itself or on something no card is about",
        );
        resources::draw_list(
            app,
            ui,
            board_task.runs(),
            &mut controls,
            palette,
            actions,
            resources::Rows::Across { width: ROW_WIDTH },
        );
        // After the last of them, so what is running is read before what else could be.
        start::draw_button(app, ui, board_task.runs(), &mut controls, actions);
    });
    ui.add_space(4.0);
}
