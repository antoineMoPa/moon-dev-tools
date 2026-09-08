//! Asking the server what could be typed in a pane: what would finish the word being typed,
//! and what could go after a `.`.
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
//!
//! Two questions rather than one, and they are asked on completely different rhythms. What
//! could finish this word is asked whenever the typing stops. What opens a list on its own -
//! the characters the server itself named, which is what makes a `.` worth a list - is asked
//! once per pane, once that pane's server is ready, and kept: it is the server's own answer,
//! said as it started and unchanged for as long as it runs, and on a `--remote` review every
//! one of these is a round trip.

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
    let wants_the_triggers = editor.wants_to_know_what_opens_a_list();
    let asking = {
        let (completing, at_the_caret, can_answer) =
            editor.completing_at_the_caret(output.caret.as_ref());
        match completing.follow(output, at_the_caret, can_answer, Instant::now()) {
            CompletingNext::Nothing => None,
            CompletingNext::Wait => {
                ctx.request_repaint_after(TYPING_SETTLES_IN);
                None
            }
            CompletingNext::Ask(asked) => Some(asked),
        }
    };
    if wants_the_triggers {
        app.ask_what_opens_a_list(pane_id, session_id, file_path.clone());
    }
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
    /// Ask what opens a completion list in this file on its own.
    ///
    /// Once per pane - see
    /// [`FileEditor::wants_to_know_what_opens_a_list`](crate::native::file_pane::FileEditor::wants_to_know_what_opens_a_list),
    /// which is what
    /// says so and says it only once its server is ready. A pane that hears nothing back,
    /// because the call did not land, goes on offering to finish words and offers no list
    /// after a `.`; it is not asked again, since a question that failed once is a question
    /// this pane would otherwise put on every frame for the rest of the session.
    fn ask_what_opens_a_list(&mut self, pane_id: PaneId, session_id: &str, file_path: String) {
        let for_call = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(format!("lsp-triggers:{pane_id}")),
            move |backend| {
                Ok(SessionLanguages::new(backend, &for_call).trigger_characters(&file_path))
            },
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                editor.opens_a_list_on(result.unwrap_or_default());
            },
        );
    }

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
                editor.word_answered(&asked, result.ok());
            },
        );
    }
}
