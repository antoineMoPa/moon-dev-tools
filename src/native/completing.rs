//! Finishing a word as it is typed, out of what the language server behind the file knows.
//!
//! The editor draws the list and puts the chosen row into the text - it owns the buffer, and
//! the keyboard while a list is up. What is here is the other half: deciding when the question
//! is worth asking, asking it, and handing back the rows. Nothing here ever touches the text.
//!
//! Three things make that a decision rather than a call. A file no server serves - markdown,
//! configuration, and most of a repo - asks nothing, ever, and costs a match on an enum per
//! frame. A question is only worth asking once the typing has stopped, because a call a
//! keystroke floods the server and, on a `--remote` session, the link. And a question is only
//! answerable about text the server has already been told about, by a server that has finished
//! reading the project - which is why this sits on top of [`crate::native::lsp_document`]
//! rather than beside it. Asking about a caret in text the server has never heard of gets a
//! confident answer about the wrong file, and asking a server that is still indexing gets an
//! empty one, which reads exactly like a real answer of "there is nothing to finish this
//! with". Both are waited out rather than answered, and waiting leaves the word askable: the
//! word half typed while a server was starting is offered its rows the moment it is ready,
//! without another letter being typed to unstick it.
//!
//! The last of the three is what most of the state below is for. Typing does not stop while a
//! request is out, so an answer routinely lands for a word that is no longer being typed, and
//! a list of names for a word the person has already finished is worse than no list at all.
//! Every request remembers what it was about and every answer is checked against what is being
//! typed when it lands.

use std::time::Instant;

use egui_frames::PaneId;
use egui_moon_editor::{Completion, EditorOutput};

use crate::{
    api::{LspCompletion, LspPosition},
    native::{
        app::App,
        lsp_document::{CanAnswer, TYPING_SETTLES_IN},
    },
};

/// The most rows offered at once, however many the server named.
///
/// A server answers a bare prefix with everything in scope, which for rust-analyzer is
/// thousands of items. Only a handful are ever on screen, and the rest are a list nobody
/// scrolls, cloned into the editor's request every frame.
const MOST_ROWS: usize = 50;

/// A question about one word: what to ask, and what to check the answer against when it lands.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Asked {
    /// The word being typed. Two words that read the same in two places are still two
    /// questions, so where it is counts as much as what it says.
    word: String,
    /// Where the caret sits - the end of the word, which is the place a server is asked what
    /// could finish it rather than what could stand in front of it.
    ///
    /// Straight from the editor: the line counts from zero and the column is bytes into that
    /// line, which is exactly what [`LspPosition`] is. What the server counts in is settled
    /// inside `src/lsp`, and there is nothing to convert here.
    line: usize,
    column: usize,
}

impl Asked {
    fn at(&self) -> LspPosition {
        LspPosition {
            line: self.line,
            column: self.column,
        }
    }
}

/// What is on offer under the caret, and the word it finishes.
struct Offering {
    word: String,
    rows: Vec<Completion>,
}

/// What one file pane is doing about finishing the word being typed in it.
///
/// It lives on the pane rather than on the window because two file panes each have their own
/// caret, their own word and their own question out about it.
#[derive(Default)]
pub(crate) struct Completing {
    /// The word the caret is on, and the moment it became that word. Together they are how
    /// long the typing has been stopped for, which is what the pause is measured against.
    typing: Option<Asked>,
    typing_since: Option<Instant>,
    /// The question that is out, if there is one. One at a time per pane, so what comes back
    /// is always the answer to this.
    asked: Option<Asked>,
    /// What the editor is being offered this frame.
    offering: Option<Offering>,
    /// The word there is nothing more to offer for: Escape was pressed over it, a row was
    /// taken on it, or it was asked about and the answer was nothing at all. It is never asked
    /// about again - Escape means stop offering, and a list that pops straight back up is the
    /// most annoying possible outcome. Typing another letter makes it a different word and a
    /// fair question again.
    nothing_more_for: Option<String>,
}

/// What the pane does about asking, on the frame it has just drawn.
#[derive(PartialEq, Eq, Debug)]
enum Next {
    /// No word under the caret, the answer already in hand, a question already out, or one
    /// already put away.
    Nothing,
    /// There is a word worth asking about, but not yet: the typing has not stopped for long
    /// enough, or the server cannot answer about this text yet. Either way the frame it
    /// becomes worth asking on has to be drawn, and a window nobody is typing in draws no
    /// more of them - so the caller asks for one.
    ///
    /// Waiting is deliberately not answering: nothing is asked, so nothing comes back empty,
    /// so the word is not written off as one with nothing to offer.
    Wait,
    /// Ask what finishes the word the caret is on.
    Ask,
}

impl Completing {
    /// What the editor is offered this frame. Empty offers nothing, which is the usual state.
    pub(crate) fn on_offer(&self) -> &[Completion] {
        match &self.offering {
            Some(offering) => &offering.rows,
            None => &[],
        }
    }

    /// Take note of the word the caret is on, so the pause is measured from the last time it
    /// changed rather than from the first time it was worth asking about.
    fn saw(&mut self, word: Option<&Asked>, now: Instant) {
        if self.typing.as_ref() != word {
            self.typing = word.cloned();
            self.typing_since = Some(now);
        }
    }

    /// Put the list away once it has stopped being an answer to what is being typed.
    ///
    /// Four things end a list, and they are all here rather than spread over the frame: a row
    /// taken, Escape, the caret leaving the word the list finishes, and the pane losing the
    /// keyboard - a popup left hanging over a pane that has moved on is the thing to avoid.
    fn put_away(&mut self, output: &EditorOutput, word: Option<&Asked>, focused: bool) {
        if output.completion_taken.is_some() || output.completion_dismissed {
            // The word under the caret *now*, which after a taken row is the word the take
            // just put there: that is the one nothing more is offered for.
            self.nothing_more_for = word.map(|word| word.word.clone());
            self.offering = None;
            return;
        }
        let finishes_something_else = self
            .offering
            .as_ref()
            .is_some_and(|offering| word.is_none_or(|word| word.word != offering.word));
        if !focused || finishes_something_else {
            self.offering = None;
        }
        if self.nothing_more_for.as_deref() != word.map(|word| word.word.as_str()) {
            self.nothing_more_for = None;
        }
    }

    /// Whether to ask. Pure, so the pause and every reason not to ask are tested without a
    /// clock, a server or a window.
    fn next(&self, can_answer: CanAnswer, now: Instant) -> Next {
        let (Some(typing), Some(since)) = (&self.typing, self.typing_since) else {
            return Next::Nothing;
        };
        if self.nothing_more_for.as_deref() == Some(typing.word.as_str()) {
            return Next::Nothing;
        }
        if self.asked.is_some() {
            return Next::Nothing;
        }
        if self
            .offering
            .as_ref()
            .is_some_and(|offering| offering.word == typing.word)
        {
            return Next::Nothing;
        }
        if now.duration_since(since) < TYPING_SETTLES_IN {
            return Next::Wait;
        }
        match can_answer {
            CanAnswer::Yes => Next::Ask,
            // Both of the other two are waits rather than answers. The server's copy being a
            // word behind is a wait of a moment - the document sync is already bringing it
            // up. A server still reading the project is a wait of tens of seconds. Neither is
            // asked, so neither can come back empty and write the word off.
            CanAnswer::NotThisText | CanAnswer::StillReadingTheProject => Next::Wait,
        }
    }

    /// An answer has come back. It is only offered if it is still an answer to the word being
    /// typed: the caret has kept moving the whole time the question was out.
    fn answered(&mut self, asked: &Asked, answer: anyhow::Result<Vec<LspCompletion>>) {
        if self.asked.as_ref() == Some(asked) {
            self.asked = None;
        }
        // The word has moved on, or has been put away since. Either way this is a list of
        // names for text that is no longer there, which is worse than no list.
        if self.typing.as_ref() != Some(asked) {
            return;
        }
        let rows = match answer {
            Ok(rows) => rows_for(&asked.word, rows),
            // A server that could not answer offers nothing, and says nothing about it: a
            // completion list is an offer, and an offer that did not come is not a fault.
            Err(_) => Vec::new(),
        };
        if rows.is_empty() {
            // Asked and answered with nothing. Nothing is gained by asking again about the
            // same word every time the typing stops.
            self.nothing_more_for = Some(asked.word.clone());
            return;
        }
        self.offering = Some(Offering {
            word: asked.word.clone(),
            rows,
        });
    }
}

/// The rows a server's answer comes to, as the editor takes them.
///
/// The three fields are the same three on purpose, so the mapping is a mapping. What is a
/// decision is which rows survive it: the protocol leaves the filtering to whoever asked, and
/// a server handed a position answers with everything that could stand there, most of which
/// does not begin with what has been typed. Offering those would put a row nobody typed
/// towards at the top of the list, where Enter takes it.
fn rows_for(word: &str, answered: Vec<LspCompletion>) -> Vec<Completion> {
    answered
        .into_iter()
        .filter(|row| starts_the_same(&row.label, word))
        .take(MOST_ROWS)
        .map(|row| Completion {
            label: row.label,
            detail: row.detail,
            insert: row.insert,
        })
        .collect()
}

/// Whether a row reads as a way of finishing the word, ignoring case the way every list in
/// this product does - someone typing `str` means `String` too.
fn starts_the_same(label: &str, word: &str) -> bool {
    let mut label = label.chars();
    word.chars()
        .all(|typed| label.next().is_some_and(|row| row.eq_ignore_ascii_case(&typed)))
}

/// What the pane does about completions on the frame it has just drawn: take in what the
/// editor reported, put the list away if it has stopped being an answer, and ask when the
/// typing has stopped over a word nothing has been asked about yet.
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
    let focused = output.response.has_focus();
    // The word and the place to ask about it both come straight from the editor - see the
    // fields of [`Asked`].
    let word = output
        .word_being_typed
        .as_ref()
        .zip(output.caret.as_ref())
        .map(|(word, caret)| Asked {
            word: word.text.clone(),
            line: caret.line,
            column: caret.column,
        });

    // Worked out while the pane is borrowed and acted on once it is not: spawning a call
    // takes the whole window.
    let asking = {
        let now = Instant::now();
        let (completing, can_answer) = editor.completing_and_server();
        completing.put_away(output, word.as_ref(), focused);
        completing.saw(word.as_ref(), now);
        match completing.next(can_answer, now) {
            Next::Nothing => None,
            Next::Wait => {
                ctx.request_repaint_after(TYPING_SETTLES_IN);
                None
            }
            Next::Ask => {
                let asked = completing
                    .typing
                    .clone()
                    .expect("a word to ask about is what makes this an ask");
                completing.asked = Some(asked.clone());
                Some(asked)
            }
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
            move |backend| backend.lsp_completion(&for_call, &file_path, at),
            move |model, result| {
                // The pane may have been closed while the question was out, and the answer
                // belongs to nobody else.
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                editor.completing_mut().answered(&asked, result);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn asked(word: &str) -> Asked {
        Asked {
            word: word.to_string(),
            line: 3,
            column: word.len(),
        }
    }

    fn offered(labels: &[&str]) -> Vec<LspCompletion> {
        labels
            .iter()
            .map(|label| LspCompletion {
                label: label.to_string(),
                detail: None,
                insert: label.to_string(),
            })
            .collect()
    }

    /// The point of the pause: a burst of typing asks once at the end of it rather than once a
    /// keystroke, which on a remote session is a round trip a letter.
    #[test]
    fn a_word_is_asked_about_once_the_typing_has_stopped_rather_than_once_a_keystroke() {
        let start = Instant::now();
        let mut completing = Completing::default();

        let mut typed = String::new();
        for (key, letter) in "greet".chars().enumerate() {
            typed.push(letter);
            let at = start + Duration::from_millis(20 * key as u64);
            completing.saw(Some(&asked(&typed)), at);
            assert_eq!(completing.next(CanAnswer::Yes, at), Next::Wait, "at key {key}");
        }

        let last = start + Duration::from_millis(20 * 4);
        assert_eq!(
            completing.next(CanAnswer::Yes, last + TYPING_SETTLES_IN / 2),
            Next::Wait
        );
        assert_eq!(completing.next(CanAnswer::Yes, last + TYPING_SETTLES_IN), Next::Ask);
    }

    /// Nothing is asked about a caret sitting on a space, and nothing is asked against a
    /// server that has not yet been told the text the caret is in.
    #[test]
    fn there_is_nothing_to_ask_with_no_word_and_nothing_to_ask_against_a_stale_document() {
        let now = Instant::now();
        let mut completing = Completing::default();
        completing.saw(None, now);
        assert_eq!(completing.next(CanAnswer::Yes, now), Next::Nothing);

        completing.saw(Some(&asked("greet")), now - TYPING_SETTLES_IN);
        assert_eq!(completing.next(CanAnswer::NotThisText, now), Next::Wait);
        assert_eq!(completing.next(CanAnswer::Yes, now), Next::Ask);
    }

    /// A server that has not finished reading the project answers every question with
    /// nothing, which reads exactly like a real answer of "there is nothing to finish this
    /// with". Nothing is asked while it is starting - and, the part that matters, the word
    /// half typed while it was starting is asked about the moment it is ready, rather than
    /// having been written off on the strength of a reply that never really answered.
    #[test]
    fn a_word_typed_while_the_server_is_starting_is_asked_about_once_it_is_ready() {
        let start = Instant::now();
        let long_enough = start + TYPING_SETTLES_IN;
        let mut completing = Completing::default();
        completing.saw(Some(&asked("greet")), start);

        // Tens of seconds of this, at a frame apiece, and not one question out of any of them.
        assert_eq!(
            completing.next(CanAnswer::StillReadingTheProject, long_enough),
            Next::Wait
        );

        // Nothing was asked, so nothing came back empty, so nothing was written off: the same
        // word, still askable, without another letter being typed to unstick it.
        assert!(completing.nothing_more_for.is_none());
        assert_eq!(completing.next(CanAnswer::Yes, long_enough), Next::Ask);
    }

    /// An answer that arrives for a word the person has already finished typing offers names
    /// for text that is no longer there, so it is dropped rather than shown.
    #[test]
    fn an_answer_for_a_word_that_has_moved_on_is_dropped() {
        let now = Instant::now();
        let mut completing = Completing::default();
        completing.saw(Some(&asked("gre")), now);
        completing.asked = Some(asked("gre"));

        // Two more letters went in while the question was out.
        completing.saw(Some(&asked("greet")), now);
        completing.answered(&asked("gre"), Ok(offered(&["greet", "greeting"])));
        assert!(completing.on_offer().is_empty());
        // And the pane is free to ask about the word that is really being typed.
        assert!(completing.asked.is_none());
        assert_eq!(completing.next(CanAnswer::Yes, now + TYPING_SETTLES_IN), Next::Ask);

        // The answer to that one lands while it is still the word being typed, and is shown.
        completing.asked = Some(asked("greet"));
        completing.answered(&asked("greet"), Ok(offered(&["greet", "greeting"])));
        assert_eq!(completing.on_offer().len(), 2);
    }

    /// Escape means stop offering. Nothing is asked about that word again, so the list cannot
    /// pop straight back up over it; another letter is another word and a fair question.
    #[test]
    fn a_dismissed_word_is_never_asked_about_again_and_the_next_one_is() {
        let now = Instant::now();
        let long_enough = now + TYPING_SETTLES_IN;
        // As an Escape over `greet` leaves the pane.
        let mut completing = Completing {
            nothing_more_for: Some("greet".to_string()),
            ..Default::default()
        };

        completing.saw(Some(&asked("greet")), now);
        assert_eq!(completing.next(CanAnswer::Yes, long_enough), Next::Nothing);

        completing.saw(Some(&asked("greeting")), now);
        assert_eq!(completing.next(CanAnswer::Yes, long_enough), Next::Ask);
    }

    /// A server that answered with nothing is not asked the same question again every time
    /// the typing stops.
    #[test]
    fn a_word_the_server_had_nothing_for_is_not_asked_about_again() {
        let now = Instant::now();
        let mut completing = Completing::default();
        completing.saw(Some(&asked("greet")), now);
        completing.asked = Some(asked("greet"));
        completing.answered(&asked("greet"), Ok(Vec::new()));

        assert!(completing.on_offer().is_empty());
        assert_eq!(
            completing.next(CanAnswer::Yes, now + TYPING_SETTLES_IN),
            Next::Nothing
        );
    }

    /// The protocol leaves the filtering to whoever asked: only the rows that could finish
    /// what has been typed are offered, and never more than a screenful of lists.
    #[test]
    fn only_the_rows_that_could_finish_the_word_are_offered() {
        let rows = rows_for("str", offered(&["String", "str", "as_ref", "Struct", "u32"]));
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["String", "str", "Struct"]);

        let many: Vec<String> = (0..200).map(|number| format!("greet{number}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(rows_for("greet", offered(&many)).len(), MOST_ROWS);
    }
}
