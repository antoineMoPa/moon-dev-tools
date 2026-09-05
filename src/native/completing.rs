//! Asking the server what would finish the word being typed in a pane.
//!
//! The editor draws the list and puts the chosen row into the text; when the question is worth
//! asking at all, and whether an answer that has just landed is still an answer to what is
//! being typed, is [`egui_moon_code_ide::Completing`]'s. Both of those are the same
//! wherever this widget is used, so neither is here any more.
//!
//! What is left is this window's: the state lives on the pane, because two file panes each
//! have their own caret and their own question out about it, and the call goes through
//! [`crate::backend::Backend`] on [`crate::native::tasks`] so a `--remote` session asks the
//! server sitting beside the repo.

use std::time::Instant;

use egui_frames::PaneId;
use egui_moon_code_ide::{Asked, CompletingNext, LanguageSource, TYPING_SETTLES_IN};
use egui_moon_editor::EditorOutput;

use crate::native::{app::App, language_source::SessionLanguages};

/// What the pane does about completions on the frame it has just drawn: hand the editor's
/// output to the state machine, and ask when it says the word is worth a question.
///
/// Called from the file pane with the editor's output in hand, because that output is where
/// the word being typed, the caret and the fate of the last list all come from.
pub(crate) fn follow_the_caret(
    app: &mut App,
    pane_id: PaneId,
    session_id: &str,
    ctx: &egui::Context,
    output: &EditorOutput,
) {
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    // A file nothing serves never asks anything, and this is the whole of what it costs.
    if !editor.offers_completions() {
        return;
    }
    let file_path = editor.file_path.clone();

    // Worked out while the pane is borrowed and acted on once it is not: spawning a call
    // takes the whole window.
    let asking = {
        let (completing, can_answer) = editor.completing_and_server();
        match completing.follow(output, can_answer, Instant::now()) {
            CompletingNext::Nothing => None,
            CompletingNext::Wait => {
                ctx.request_repaint_after(TYPING_SETTLES_IN);
                None
            }
            CompletingNext::Ask(asked) => Some(asked),
        }
    };
    if let Some(asked) = asking {
        app.ask_what_finishes_the_word(pane_id, session_id, file_path, asked);
    }
}

impl App {
    /// Ask the server what could finish the word the caret is on.
    ///
    /// Keyed by pane the way every other call about a file is, so a second question cannot go
    /// out over the first: the pane's record of what it asked is what an answer is checked
    /// against, and it only holds one.
    fn ask_what_finishes_the_word(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: String,
        asked: Asked,
    ) {
        let for_call = session_id.to_string();
        let at = asked.at();
        self.tasks.spawn_keyed(
            Some(format!("lsp-completion:{pane_id}")),
            move |backend| SessionLanguages::new(backend, &for_call).completion(&file_path, at),
            move |model, result| {
                // The pane may have been closed while the question was out, and the answer
                // belongs to nobody else.
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                editor.completing_mut().answered(&asked, result.ok());
            },
        );
    }
}
