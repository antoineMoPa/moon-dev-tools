//! Bringing a command back into the line, the two ways a shell does: the arrows walk the
//! history a command at a time, and ctrl+r searches it for what is typed.
//!
//! Neither draws anything or reads a key. They answer what the line holds after one, which is
//! what makes them testable without a window.

use super::history::History;

/// Walking the history with the arrows.
#[derive(Default)]
pub(crate) struct Walk {
    /// Which command the line is showing, counted from the oldest. `None` while it is the
    /// line being typed.
    at: Option<usize>,
    /// The line as it was typed before the walk began, which is where arrow down ends up.
    typed: String,
}

impl Walk {
    /// Arrow up: the command before the one showing. `line` is what the box holds now, kept
    /// if this is the first step back. `None` at the oldest, and for an empty history.
    pub(crate) fn older(&mut self, history: &History, line: &str) -> Option<String> {
        let older = match self.at {
            Some(at) => at.checked_sub(1)?,
            None => history.len().checked_sub(1)?,
        };
        if self.at.is_none() {
            self.typed = line.to_string();
        }
        self.at = Some(older);
        Some(history.command(older).to_string())
    }

    /// Arrow down: the command after the one showing, and after the newest of them the line
    /// that was being typed. `None` when that line is what is showing already.
    pub(crate) fn newer(&mut self, history: &History) -> Option<String> {
        let newer = self.at? + 1;
        if newer < history.len() {
            self.at = Some(newer);
            return Some(history.command(newer).to_string());
        }
        self.at = None;
        Some(std::mem::take(&mut self.typed))
    }
}

/// A search back through the history, which is what ctrl+r opens.
#[derive(Default)]
pub(crate) struct Search {
    /// What is being looked for: the box holds this while the search is open.
    pub(crate) query: String,
    /// The command found, counted from the oldest.
    at: Option<usize>,
}

impl Search {
    /// The command the search has found, if it has found one.
    pub(crate) fn found<'history>(&self, history: &'history History) -> Option<&'history str> {
        self.at.map(|at| history.command(at))
    }

    /// The query changed: look again from the newest command.
    pub(crate) fn look_again(&mut self, history: &History) {
        self.at = history.newest_containing(&self.query, history.len());
    }

    /// ctrl+r again: the next match further back. The one already found stays when there is
    /// none older, which is what a shell does too.
    pub(crate) fn look_further_back(&mut self, history: &History) {
        let Some(at) = self.at else {
            return;
        };
        if let Some(older) = history.newest_containing(&self.query, at) {
            self.at = Some(older);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::history::tests::history_of, *};

    #[test]
    fn the_arrows_walk_back_through_the_history_and_return_to_what_was_typed() {
        let history = history_of(&["cargo build", "make deploy"]);
        let mut walk = Walk::default();

        assert_eq!(
            walk.older(&history, "half typed").as_deref(),
            Some("make deploy")
        );
        assert_eq!(
            walk.older(&history, "make deploy").as_deref(),
            Some("cargo build")
        );
        assert_eq!(walk.older(&history, "cargo build"), None, "nothing older");
        assert_eq!(walk.newer(&history).as_deref(), Some("make deploy"));
        assert_eq!(walk.newer(&history).as_deref(), Some("half typed"));
        assert_eq!(walk.newer(&history), None, "nothing newer than the line");
    }

    #[test]
    fn an_arrow_up_with_no_history_leaves_the_line_alone() {
        let mut walk = Walk::default();

        assert_eq!(walk.older(&History::default(), "half typed"), None);
        assert_eq!(walk.newer(&History::default()), None);
    }

    #[test]
    fn a_search_steps_back_through_the_commands_that_match() {
        let history = history_of(&["cargo build", "make deploy", "cargo test"]);
        let mut search = Search::default();
        assert_eq!(search.found(&history), None);

        search.query = "cargo".to_string();
        search.look_again(&history);
        assert_eq!(search.found(&history), Some("cargo test"));

        search.look_further_back(&history);
        assert_eq!(search.found(&history), Some("cargo build"));
        search.look_further_back(&history);
        assert_eq!(search.found(&history), Some("cargo build"), "the oldest");

        search.query = "cargo t".to_string();
        search.look_again(&history);
        assert_eq!(search.found(&history), Some("cargo test"));

        search.query = "nothing like it".to_string();
        search.look_again(&history);
        assert_eq!(search.found(&history), None);
    }
}
