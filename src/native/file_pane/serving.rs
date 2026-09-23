//! What a file tab keeps of the language server behind it: what it found wrong, the hover and
//! the signature it is showing, the blame beside the text, and the list of words it offers to
//! finish the one being typed.

use egui_moon_code_ide::{
    Asked, AtTheCaret, CanAnswer, Completing, LspCompletion, LspPosition, LspStatus, Served,
    before_the_caret, follows_the_caret,
};

use super::FileEditor;

impl FileEditor {
    pub(in crate::native) fn diagnosed(&self) -> &crate::native::diagnostics::Diagnosed {
        &self.diagnosed
    }

    pub(in crate::native) fn diagnosed_mut(
        &mut self,
    ) -> &mut crate::native::diagnostics::Diagnosed {
        &mut self.diagnosed
    }

    pub(in crate::native) fn hovering(&self) -> &egui_moon_code_ide::Hovering {
        &self.hovering
    }

    pub(in crate::native) fn hovering_mut(&mut self) -> &mut egui_moon_code_ide::Hovering {
        &mut self.hovering
    }

    pub(in crate::native) fn signing(&self) -> &egui_moon_code_ide::Signing {
        &self.signing
    }

    pub(in crate::native) fn signing_mut(&mut self) -> &mut egui_moon_code_ide::Signing {
        &mut self.signing
    }

    pub(in crate::native) fn blaming(&self) -> &crate::native::blame::Blaming {
        &self.blaming
    }

    pub(in crate::native) fn blaming_mut(&mut self) -> &mut crate::native::blame::Blaming {
        &mut self.blaming
    }

    /// The signature's state and the text it is followed through, handed out together
    /// because the one is worked out from the other.
    pub(in crate::native) fn signing_and_text(
        &mut self,
    ) -> (&mut egui_moon_code_ide::Signing, &str) {
        (&mut self.signing, self.code.text())
    }

    /// Whether this pane's ⌘-click asks a language server, which is the only thing that
    /// answers one.
    pub(crate) fn asks_language_servers(&self) -> bool {
        self.asks_language_servers
    }

    /// Whether this pane has anything to tell a server yet: a window with its language
    /// servers switched off tells nothing, and neither does a file whose text has not
    /// arrived - a document is opened with what is in it.
    pub(in crate::native) fn has_a_document_to_keep_up(&self) -> bool {
        self.asks_language_servers && self.saved.is_some()
    }

    /// The text on screen and what the server has heard of it, handed out together because
    /// what to send next is worked out from the two at once.
    pub(in crate::native) fn text_and_server(&mut self) -> (&str, &mut Served) {
        (self.code.text(), &mut self.served)
    }

    /// What the server has heard about this file, for the tab that is closing and for the
    /// call that has just come back.
    pub(in crate::native) fn server_heard(&self) -> &Served {
        &self.served
    }

    pub(in crate::native) fn server_heard_mut(&mut self) -> &mut Served {
        &mut self.served
    }

    /// Whether this pane offers to finish the word being typed at all: a window with its
    /// language servers switched off does not, and neither does a file no server is behind -
    /// which is most of a repo, and is why this is the first thing asked every frame.
    pub(in crate::native) fn offers_completions(&self) -> bool {
        self.asks_language_servers && self.served.has_a_server()
    }

    /// The completion box's state, what the caret is sitting behind, and whether the server
    /// behind the file could answer a question about the text on screen at all. All three at
    /// once because what to ask next is worked out from all three, and because the character
    /// under the caret has to be read off this pane's own buffer.
    pub(in crate::native) fn completing_at_the_caret(
        &mut self,
        caret: Option<&egui_moon_editor::TextPoint>,
    ) -> (&mut Completing, AtTheCaret<'_>, CanAnswer) {
        let can_answer = self.served.can_answer_about(self.code.text());
        let typed = caret.and_then(|caret| {
            before_the_caret(
                self.code.text(),
                LspPosition {
                    line: caret.line,
                    column: caret.column,
                },
            )
        });
        let at_the_caret = AtTheCaret {
            typed,
            triggers: &self.triggers,
        };
        (&mut self.completing, at_the_caret, can_answer)
    }

    /// Whether this pane still has to be told what opens a completion list in it, and is in
    /// a position to be: it says yes once, and only once its file's server is up.
    ///
    /// Waited for rather than asked at once, because the answer comes out of that server's
    /// `initialize` reply and a server that has not started has not sent one. Asking early
    /// would keep the empty list of a server that had simply not spoken yet, and nothing
    /// would ever ask again - the list would be silently dead in this file for the rest of
    /// the session.
    pub(in crate::native) fn wants_to_know_what_opens_a_list(&mut self) -> bool {
        if self.asked_what_opens_a_list || self.served.status() != LspStatus::Ready {
            return false;
        }
        self.asked_what_opens_a_list = true;
        true
    }

    /// What the server said opens a list here, as the answer comes back off the worker.
    pub(in crate::native) fn opens_a_list_on(&mut self, triggers: Vec<char>) {
        self.triggers = triggers;
    }

    /// An answer about the word being typed, as it comes back off the worker.
    ///
    /// Handed in here rather than through the state itself because the answer is taken against
    /// the buffer as well as against the word: what the caret sits in front of is what keeps a
    /// call being completed over - `gre|(x)` taking `greet` - from being offered a second pair
    /// of parentheses. The pane owns the buffer, so the pane is what can read it.
    pub(in crate::native) fn word_answered(
        &mut self,
        asked: &Asked,
        rows: Option<Vec<LspCompletion>>,
    ) {
        let follows = follows_the_caret(self.code.text(), asked.at());
        self.completing.answered(asked, rows, follows);
    }
}
