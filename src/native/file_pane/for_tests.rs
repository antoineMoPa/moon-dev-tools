//! What the tests read off a file tab and do to it, which nothing else in the window needs.

use super::FileEditor;

impl FileEditor {
    /// What the server found wrong with this file, for the test that waits on a real one.
    pub(crate) fn diagnostics_for_test(&self) -> Vec<String> {
        self.diagnosed
            .found()
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    }

    /// Turn the language-server side of this one pane on in a test. A window built for a
    /// test has them off - see [`App::asks_language_servers`] - so a test that is about
    /// this wiring says so, on a file nothing serves, or it starts a real server.
    pub(crate) fn asks_language_servers_for_test(&mut self) {
        self.asks_language_servers = true;
    }

    /// What this pane thinks opens a list, for the test that follows one over the backend.
    pub(crate) fn triggers_for_test(&self) -> Vec<char> {
        self.triggers.clone()
    }

    /// What the pane is showing, once it has arrived.
    pub(crate) fn content_for_test(&self) -> Option<String> {
        self.saved.clone()
    }

    /// What is on screen, which after typing is not what was fetched.
    pub(crate) fn text_for_test(&self) -> &str {
        self.code.text()
    }

    /// The lines the fringe marks as new since the last commit, counted from zero.
    pub(crate) fn new_lines_for_test(&self) -> Vec<std::ops::Range<usize>> {
        self.code.new_lines().to_vec()
    }

    /// The line the tab was opened at and is still to scroll to, counted from one - held
    /// only until the text has arrived and been laid out.
    pub(crate) fn line_to_reveal_for_test(&self) -> Option<usize> {
        self.reveal.as_ref().map(|at| at.line)
    }

    /// How many rows the pane is offering to finish the word being typed with.
    pub(crate) fn rows_offered_for_test(&self) -> usize {
        self.completing.on_offer().len()
    }

    /// What those rows read as, for the end-to-end test that wants to see a real server's
    /// names come back.
    pub(crate) fn labels_offered_for_test(&self) -> Vec<String> {
        self.completing
            .on_offer()
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    /// Whether the pane has heard back that nothing serves its file, which is the end state
    /// of the language-server side of a pane on most of a repo.
    pub(crate) fn heard_no_server_for_test(&self) -> bool {
        self.served.nothing_serves_it()
    }

    /// Type into the file, as the editor widget does.
    pub(crate) fn edit_for_test(&mut self, text: &str) {
        self.code.set_text(text.to_string());
    }

    /// Whether the file is one outside the repo, which is what makes the pane a reader rather
    /// than an editor. Read in a test beside what the header drew, so that both halves of
    /// read-only are checked rather than just the one that is easy to assert on.
    pub(crate) fn is_outside_the_repo_for_test(&self) -> bool {
        self.outside_the_repo
    }
}
