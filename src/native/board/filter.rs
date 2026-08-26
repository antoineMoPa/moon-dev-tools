//! The filter over the board: one query, and the cards it leaves showing.
//!
//! A board grows past what a screen holds long before any of its columns do, and the query is
//! how one task is found among them. It filters rather than jumps: every column keeps its
//! place and shows the cards that match, so a task is still where it was and can be dragged
//! from there while the filter is on.
//!
//! The query lives on the board rather than in the window's find bar. The find bar steps
//! through matches inside one document, which is not what is wanted here - the board's answer
//! is the cards themselves, drawn where they belong.

use egui::{Key, Modifiers, RichText, Ui};

use crate::{
    moontasks::TaskView,
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide the query box is. Enough for a few words of a task title, which is what is typed
/// into it, and narrow enough to sit over one column rather than across the board.
const BOX_WIDTH: f32 = 240.0;

/// The query the board is being filtered by, ready to match cards against: trimmed and folded
/// to lowercase once here rather than once per card.
pub(crate) struct Filter(String);

impl Filter {
    pub(crate) fn of(query: &str) -> Self {
        Self(query.trim().to_lowercase())
    }

    /// Whether the query is leaving anything out at all.
    pub(crate) fn is_on(&self) -> bool {
        !self.0.is_empty()
    }

    /// Whether this task is one of the ones the query asks for.
    ///
    /// A card shows a title and the first lines of its `notes.md`, so those are what is looked
    /// through: what a card says is what it can be found by. Everything matches an empty
    /// query - a board nobody has typed into shows all of its cards.
    pub(crate) fn matches(&self, task: &TaskView) -> bool {
        !self.is_on()
            || task.title.to_lowercase().contains(&self.0)
            || task.notes.to_lowercase().contains(&self.0)
    }
}

/// The bar over the columns: the query, and how much of the board it is leaving out.
pub(super) fn draw(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let mut cleared = false;

    ui.horizontal(|ui| {
        let entry = ui.add(
            egui::TextEdit::singleline(&mut app.model.board.filter)
                .hint_text("Filter tasks")
                .desired_width(BOX_WIDTH)
                .margin(egui::Margin::symmetric(6, 3)),
        );
        if std::mem::take(&mut app.model.board.filter_focus) {
            entry.request_focus();
        }

        // Escape empties the box rather than leaving a filter on that the board has stopped
        // being asked about. egui takes the keyboard off whatever had it when Escape is
        // pressed, which is what `lost_focus` is here - and it leaves the key itself alone, so
        // it is taken here, before an open new-task box further down the board reads it and
        // closes along with the query.
        if entry.lost_focus()
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            cleared = true;
        }

        let filter = Filter::of(&app.model.board.filter);
        if !filter.is_on() {
            return;
        }

        ui.label(
            RichText::new(tally(&app.model.board.tasks, &filter))
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted),
        );
        if widgets::close_button(ui, palette)
            .on_hover_text("Show every task again")
            .clicked()
        {
            cleared = true;
        }
    });

    if cleared {
        app.model.board.filter.clear();
    }
    ui.add_space(6.0);
}

/// What the query is showing, out of what the board holds - so a board that has gone quiet is
/// a filter that matched little rather than a repo that lost its tasks.
fn tally(tasks: &[TaskView], filter: &Filter) -> String {
    let showing = tasks.iter().filter(|task| filter.matches(task)).count();
    if showing == 0 {
        return "no task matches".to_string();
    }
    format!("{showing} of {} tasks", tasks.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moontasks::ColumnId;

    fn task(title: &str, notes: &str) -> TaskView {
        TaskView {
            id: format!("{title}-1111"),
            title: title.to_string(),
            status: ColumnId::new("todo"),
            created_at_unix: 1700000000,
            dir_path: String::new(),
            repo_path: String::new(),
            notes: notes.to_string(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn an_empty_query_leaves_every_card_showing() {
        let filter = Filter::of("   ");

        assert!(!filter.is_on());
        assert!(filter.matches(&task("Write the parser", "")));
    }

    #[test]
    fn a_query_finds_a_card_by_its_title_whatever_case_it_is_typed_in() {
        assert!(Filter::of("PARSER").matches(&task("Write the parser", "")));
        assert!(Filter::of("write").matches(&task("Write the parser", "")));
        assert!(!Filter::of("login").matches(&task("Write the parser", "")));
    }

    /// The notes are on the card, so they are searched too: a task is often remembered by what
    /// it is about rather than by the words its title happened to use.
    #[test]
    fn a_query_finds_a_card_by_its_notes() {
        let task = task("Write the parser", "the lexer chokes on nested comments");

        assert!(Filter::of("nested comments").matches(&task));
        assert!(!Filter::of("nested parser").matches(&task));
    }

    #[test]
    fn the_tally_says_how_much_of_the_board_is_showing() {
        let tasks = vec![
            task("Write the parser", ""),
            task("Fix the login page", ""),
            task("Drop the old API", ""),
        ];

        assert_eq!(tally(&tasks, &Filter::of("the")), "3 of 3 tasks");
        assert_eq!(tally(&tasks, &Filter::of("login")), "1 of 3 tasks");
        assert_eq!(tally(&tasks, &Filter::of("parsnip")), "no task matches");
    }
}
