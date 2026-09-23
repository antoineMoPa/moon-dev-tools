//! One file of the repo, open in a tab of its own for reading and editing.
//!
//! The text and how it is drawn belong to `egui_moon_editor`; what is here is where the text
//! came from and where it goes - fetching it through the backend, writing it back, the pane's
//! own chrome, and the rendered page a markdown file opens on. The find bar stays here too:
//! the editor takes the ranges to mark as input and says how many it laid out. A ⌘-click on a
//! name in the text is the same shape of thing: the editor says which word was clicked, and
//! [`crate::native::definition`] is what turns that into a file to open. The language server
//! behind the file is told what this pane is showing by
//! [`crate::native::lsp_document`], which holds what it has heard on the pane it belongs to,
//! and what that server offers to finish the word being typed with is
//! [`crate::native::completing`]'s business - this pane hands the list in and hands the
//! editor's answer back on. Whether the file was written under the pane by something else is
//! [`on_disk`]'s.

mod drawing;
#[cfg(test)]
mod for_tests;
mod on_disk;
mod opening;
mod serving;

use std::time::Instant;

use egui_moon_code_ide::{Completing, LspPosition, Served};
use egui_moon_editor::{Editor, Language};

use crate::api::FileContentPayload;

/// A file being read or edited, and what has happened to it since it was opened.
pub(crate) struct FileEditor {
    pub(crate) file_path: String,
    /// The text as it is on disk, as far as this window knows.
    saved: Option<String>,
    /// The editor the text is read and written in. It owns the buffer, so what gets saved
    /// is what it holds.
    code: Editor,
    error: Option<String>,
    saving: bool,
    /// Whether what the pane is showing is a file outside the repo - a dependency's source or
    /// the standard library, landed on by a jump to a definition. Those are there to be read:
    /// the header says so, and no save is offered on one. Known only once the text has
    /// arrived, because the read is what answers it - see [`crate::lsp`].
    outside_the_repo: bool,
    /// Whether the pane is showing the markdown rendered rather than the text of it. Only
    /// ever true for a markdown file, which is also the only kind offered the toggle.
    preview: bool,
    /// Set when a close was asked for while there were unsaved edits: the second press goes
    /// through, the way discarding a hunk does.
    pub(crate) close_confirmed: bool,
    /// The match a content search opened this file at, if it did. Cleared once the text is
    /// there, has been scrolled to, and has been handed to the find bar to mark.
    reveal: Option<crate::native::panes::OpenAt>,
    /// What the lookup a ⌘-click in this pane started came back with, waiting for the frame
    /// that acts on it. It waits here rather than being acted on where it arrives because
    /// opening a pane is deferred to the end of a frame - see
    /// [`crate::native::definition::follow`].
    pub(crate) looking_up: Option<crate::native::definition::LookedUp>,
    /// Whether this pane asks a language server anything, taken from the window as the pane
    /// is opened - see [`App::asks_language_servers`].
    asks_language_servers: bool,
    /// Whether a language server is behind this file, and what it has been told about it -
    /// see [`crate::native::lsp_document`].
    served: Served,
    /// What is being offered to finish the word being typed, and what has been asked about
    /// it - see [`crate::native::completing`].
    completing: Completing,
    /// The characters the server behind this file said open a completion list on their own -
    /// the `.` of `thing.`, the `:` of a path. Empty until it has been asked and empty for a
    /// server that named none, which in both cases means only a word being typed is asked
    /// about.
    triggers: Vec<char>,
    /// Whether that question has been put. Once per pane: the answer is the server's own,
    /// said once as it started and unchanged for as long as it runs, and on a `--remote`
    /// review it is a round trip - asking again every frame would be a call a frame for
    /// something already in hand.
    asked_what_opens_a_list: bool,
    /// The file as another program wrote it while this pane had edits of its own. Held
    /// rather than put on screen, since that would throw the edits away: `[reload]` takes it,
    /// `[save]` writes the edits over it. `None` while what is on disk is what was saved.
    written_elsewhere: Option<FileContentPayload>,
    /// Set by a first press of `[reload]`: the second one drops the edits, the way closing a
    /// tab with unsaved edits does.
    reload_confirmed: bool,
    /// When the file was last read to see whether it changed on disk - see [`on_disk`].
    last_disk_check: Option<Instant>,
    /// How many writes this pane has sent. A check of the disk started before a write can
    /// come back with the file from before it, and would read as somebody else's change - so
    /// a check only answers when no write went out while it was reading.
    writes_sent: u64,
    /// Whether the file is on disk at all. False for a tab opened on a path nothing is at yet -
    /// see [`FileEditor::new_file`] - until its first save creates the file: there is nothing
    /// to read back from the disk before then, and that save creates rather than writes.
    on_disk: bool,
    /// Where the caret was as the editor last drew, which is the place a rename asks about -
    /// see [`crate::native::renaming`]. Kept rather than asked for, because a rename is
    /// started from the palette and the key map as well as from the pane, and neither has
    /// the editor's frame to read it off.
    caret: Option<LspPosition>,
    /// The word the caret sat in as the editor last drew, which is the name the places of a
    /// name are asked about - see [`crate::native::places`]. Kept for the same reason.
    caret_word: Option<egui_moon_editor::Word>,
    /// When a format was asked for, while it waits for the tab's text to reach the server -
    /// see [`crate::native::formatting`].
    asked_to_format: Option<Instant>,
    /// What the server behind the file last said is wrong with it - see
    /// [`crate::native::diagnostics`].
    diagnosed: crate::native::diagnostics::Diagnosed,
    /// Where the pointer has rested and what the server said about the word under it - see
    /// [`crate::native::hover`].
    hovering: egui_moon_code_ide::Hovering,
    /// The signature of the call being typed, and what has been asked about it - see
    /// [`crate::native::signature`].
    signing: egui_moon_code_ide::Signing,
    /// Who last touched each stretch of the file, beside the lines when asked for - see
    /// [`crate::native::blame`].
    blaming: crate::native::blame::Blaming,
    /// The commit whose version of the file this tab shows, for a tab opened off a blame onto
    /// the file as it was. Read-only: the text is not the working tree's, so there is nothing
    /// to save it to, no disk to check under it, and no language server to tell about it.
    revision: Option<String>,
}

impl FileEditor {
    fn loading(file_path: String, asks_language_servers: bool) -> Self {
        let preview = is_markdown(&file_path);
        let mut code = Editor::new(String::new());
        // What the file is read as, settled here because the path is what says so and this is
        // where the pane learns it. The text arrives later; the language is the same either way.
        code.set_language(Language::of_path(&file_path));
        Self {
            file_path,
            saved: None,
            code,
            error: None,
            saving: false,
            outside_the_repo: false,
            preview,
            close_confirmed: false,
            reveal: None,
            looking_up: None,
            asks_language_servers,
            served: Served::Unknown,
            completing: Completing::default(),
            triggers: Vec::new(),
            asked_what_opens_a_list: false,
            written_elsewhere: None,
            reload_confirmed: false,
            last_disk_check: None,
            writes_sent: 0,
            on_disk: true,
            caret: None,
            caret_word: None,
            asked_to_format: None,
            diagnosed: Default::default(),
            hovering: Default::default(),
            signing: Default::default(),
            blaming: Default::default(),
            revision: None,
        }
    }

    /// A tab on the file as one commit had it - see [`FileEditor::revision`]. Its blame is up
    /// from the start, because looking at who touched what is what such a tab is opened for,
    /// and it asks no language server anything: the text is not a document of the project.
    fn at_revision(file_path: String, revision: String) -> Self {
        let mut editor = Self::loading(file_path, false);
        editor.revision = Some(revision);
        // Opened to be read line by line beside the blame, so a markdown file opens on its
        // text rather than on the rendered page.
        editor.preview = false;
        editor.blaming.toggle();
        editor
    }

    /// A tab on a path nothing is at yet: empty, and not on disk until its first save creates
    /// the file. Closing it unsaved leaves nothing behind.
    fn new_file(file_path: String, asks_language_servers: bool) -> Self {
        let mut editor = Self::loading(file_path, asks_language_servers);
        editor.saved = Some(String::new());
        // Every line of it is new, the same as a file HEAD does not have.
        editor.code.set_base(Some(String::new()));
        // Opened to be written, so a markdown file opens on its text rather than on an empty
        // rendered page.
        editor.preview = false;
        editor.on_disk = false;
        editor
    }

    /// The text on screen, unsaved edits and all.
    pub(super) fn text(&self) -> &str {
        self.code.text()
    }

    /// Whether the file's text has arrived. A pane still fetching it has nothing in it to
    /// rename, and nothing a rename could be put into.
    pub(super) fn is_loaded(&self) -> bool {
        self.saved.is_some()
    }

    /// Where the caret was as the editor last drew - see [`FileEditor::caret`].
    pub(super) fn caret(&self) -> Option<LspPosition> {
        self.caret
    }

    /// The word the caret sat in as the editor last drew - see [`FileEditor::caret_word`].
    pub(super) fn caret_word(&self) -> Option<egui_moon_editor::Word> {
        self.caret_word.clone()
    }

    /// Whether the places of a name can be asked for in this pane: a server is behind the
    /// file. A dependency's source counts - finding what uses a name is reading, not writing.
    pub(crate) fn offers_places(&self) -> bool {
        self.asks_language_servers && self.served.has_a_server()
    }

    /// Ask for this tab to be formatted once its text has reached the server.
    pub(super) fn ask_to_format(&mut self, now: Instant) {
        self.asked_to_format = Some(now);
    }

    /// When a format was asked for, while it waits to be sent.
    pub(super) fn asked_to_format(&self) -> Option<Instant> {
        self.asked_to_format
    }

    pub(super) fn stop_asking_to_format(&mut self) {
        self.asked_to_format = None;
    }

    /// Whether the file is outside the repo, and so read-only.
    pub(super) fn is_outside_the_repo(&self) -> bool {
        self.outside_the_repo
    }

    /// The commit whose version of the file the tab shows, for a tab on an old version.
    pub(super) fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    /// Whether the text can only be read: a file outside the repo, or a file as an old
    /// commit had it. Neither has anywhere for an edit to go.
    pub(super) fn is_read_only(&self) -> bool {
        self.outside_the_repo || self.revision.is_some()
    }

    /// Whether the server behind this file may be asked for edits to it - a rename, a format:
    /// a server is behind the file, and the file is one of the repo's own. Edits to a
    /// dependency's source are edits nobody may write.
    pub(crate) fn offers_edits(&self) -> bool {
        self.asks_language_servers && self.served.has_a_server() && !self.is_read_only()
    }

    /// Put a rename's edits into the text, as byte ranges of it in order - see
    /// [`egui_moon_editor::Editor::replace_ranges`]. The pane is dirty afterwards, the way it
    /// is after typing: the edit is the person's to save.
    pub(super) fn take_edits(&mut self, ranges: Vec<(std::ops::Range<usize>, String)>) {
        self.code.replace_ranges(ranges);
    }

    /// Put a new entry of the work log into the text, and leave the caret where it is to be
    /// written - see [`crate::native::work_log`]. Only for a tab with its text: there is
    /// nothing to put it into before. A text with no marker line is left alone and said so.
    pub(crate) fn add_work_log_entry(
        &mut self,
        entry: &crate::native::work_log::NewEntry,
    ) -> Result<(), String> {
        assert!(self.is_loaded(), "an entry is put into a text that has arrived");
        let Some(insertion) = crate::native::work_log::insertion(self.code.text(), entry) else {
            return Err(format!(
                "no \"{}\" line in {}: a new entry goes in above it",
                crate::native::work_log::NOW_MARKER,
                self.file_path
            ));
        };
        self.take_edits(vec![(insertion.byte..insertion.byte, insertion.header)]);
        self.code.place_caret(insertion.caret);
        self.reveal = Some(crate::native::panes::OpenAt {
            line: insertion.line,
            query: String::new(),
        });
        Ok(())
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.saved
            .as_ref()
            .is_some_and(|saved| saved != self.code.text())
    }
}

/// Whether the file is written in markdown, which is what decides if the pane opens on the
/// rendered page and offers the way back to the text.
fn is_markdown(file_path: &str) -> bool {
    std::path::Path::new(file_path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn editor_with(saved: &str, edited: &str) -> FileEditor {
        FileEditor {
            file_path: "src/lib.rs".to_string(),
            saved: Some(saved.to_string()),
            code: Editor::new(edited.to_string()),
            error: None,
            saving: false,
            outside_the_repo: false,
            preview: false,
            close_confirmed: false,
            reveal: None,
            looking_up: None,
            asks_language_servers: false,
            served: Served::Unknown,
            completing: Completing::default(),
            triggers: Vec::new(),
            asked_what_opens_a_list: false,
            written_elsewhere: None,
            reload_confirmed: false,
            last_disk_check: None,
            writes_sent: 0,
            on_disk: true,
            caret: None,
            caret_word: None,
            asked_to_format: None,
            diagnosed: Default::default(),
            hovering: Default::default(),
            signing: Default::default(),
            blaming: Default::default(),
            revision: None,
        }
    }

    #[test]
    fn a_file_is_dirty_only_once_it_differs_from_what_was_saved() {
        assert!(!editor_with("fn one() {}", "fn one() {}").is_dirty());
        assert!(editor_with("fn one() {}", "fn two() {}").is_dirty());
        // Nothing has arrived yet, so there is nothing to have changed.
        assert!(!FileEditor::loading("src/lib.rs".to_string(), false).is_dirty());
    }

    /// Markdown opens on the rendered page; everything else opens on the text, and never
    /// grows the toggle at all.
    #[test]
    fn only_markdown_opens_on_the_rendered_page() {
        assert!(is_markdown("notes.md"));
        assert!(is_markdown(".moontasks/fix-login-1234/NOTES.MD"));
        assert!(!is_markdown("src/lib.rs"));
        assert!(!is_markdown("md"));
        assert!(!is_markdown("README"));

        assert!(FileEditor::loading("Moontasks.md".to_string(), false).preview);
        assert!(!FileEditor::loading("src/lib.rs".to_string(), false).preview);
    }
}
