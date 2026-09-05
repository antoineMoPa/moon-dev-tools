//! Keeping a language server's copy of an open file up with what a pane is showing.
//!
//! A server answers questions about a document it has been told about, and the only thing
//! that knows what is in that document is the pane the file is open in: the text on disk is
//! not it - the point of asking is to be answered about what has been typed. So the pane is
//! what tells the server, and what it has heard is kept on the pane beside the text itself.
//!
//! Three things are sent, and nothing else: the file is opened when its text lands, changed
//! once the typing has stopped, and closed when the last tab on it goes. They only ever go
//! for a file something actually serves, which is asked once and remembered - most of a repo
//! is markdown, configuration and images that no server has ever heard of, and a `--remote`
//! session pays for every question over the network.
//!
//! What the answers are then used for is [`crate::native::definition`]'s business.

use egui_frames::PaneId;

use crate::{
    api::LspStatus,
    native::{app::App, file_pane::FileEditor},
};

use std::time::{Duration, Instant};

/// How long the typing has to have stopped before the server hears the new text.
///
/// The whole text goes over on every change, and on a `--remote` session that is the whole
/// file over the network: a call per keystroke would flood the server and the link both.
/// Four hundred milliseconds is longer than the gap between two keys of ordinary typing, so
/// a sentence typed straight through sends once at the end of it rather than once a letter,
/// and short enough that a ⌘-click made after even a brief pause is answered against what is
/// on screen rather than against what was there a word ago.
///
/// It is also the pause [`crate::native::completing`] waits before it asks what could finish
/// the word being typed, and deliberately the same number rather than one of its own: that
/// question can only be asked about text the server has already heard, so a shorter pause
/// there would only end up waiting on this one, and a longer one would make the person wait
/// twice over a single lull in the typing.
pub(super) const TYPING_SETTLES_IN: Duration = Duration::from_millis(400);

/// How often a server that is still reading the project is asked whether it has finished.
///
/// It is asked at all because the answer changes on its own: rust-analyzer takes tens of
/// seconds over a cold project, and until it is done it answers every question with nothing,
/// which reads exactly like a real answer of "there is nothing". A question a second while a
/// tab waits on a server that is starting costs nothing next to that, and it stops the moment
/// the answer is yes - a server that has finished starting does not un-finish.
const STARTING_IS_ASKED_ABOUT_EVERY: Duration = Duration::from_secs(1);

/// Whether a language server is behind a pane's file, and what it has heard.
pub(crate) enum Served {
    /// Nobody has asked yet. Nothing is asked until the text has arrived: a document is
    /// opened with what is in it, and there is nothing to open it with before then.
    Unknown,
    /// The question is out.
    Asking,
    /// No server serves this file, which is the normal state of most of a repo - markdown,
    /// configuration, images, and every language nobody installed a server for. Nothing more
    /// is ever sent about it and nothing is ever said about it: it is not a fault.
    No,
    /// A server serves it, and this is what it has heard.
    Yes(Document),
}

/// Whether the server behind a file can answer a question about a place in the text on
/// screen, and when it cannot, why not.
///
/// The two reasons are told apart because they end differently. Text the server has not heard
/// is on its way to it already and will be there in a moment; a server that has not finished
/// reading the project is a wait of tens of seconds. Neither is ever mistaken for an answer -
/// that is the whole of what this is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CanAnswer {
    /// Not about this text. Either the server has never heard of the file, or what it was
    /// told is a word or two behind what is on screen - and a caret at line twelve of the
    /// text it heard a word ago is a place in a different file.
    NotThisText,
    /// Not yet at all: it has the text, but it is still reading the project. A server that is
    /// still reading answers every question with nothing, which reads exactly like a real
    /// answer of "there is nothing here" - the mistake [`LspStatus`] exists to prevent, and
    /// the one [`crate::native::definition`] already refuses to make on a ⌘-click.
    StillReadingTheProject,
    /// Yes.
    Yes,
}

impl Served {
    /// Whether the server behind this file can answer a question about a place in the text on
    /// screen right now. A file nothing serves answers about no text at all.
    pub(super) fn can_answer_about(&self, text: &str) -> CanAnswer {
        let Served::Yes(document) = self else {
            return CanAnswer::NotThisText;
        };
        if !document.opened || document.sent != text {
            return CanAnswer::NotThisText;
        }
        match document.ready {
            true => CanAnswer::Yes,
            false => CanAnswer::StillReadingTheProject,
        }
    }
}

/// One open document, as the server last heard it.
pub(crate) struct Document {
    /// Whether the open has gone through. Until it has, the server has never heard of the
    /// file and a change would be a change to nothing.
    opened: bool,
    /// Whether a call about this document is in flight. One at a time, so the record of what
    /// the server has heard is only ever written by a call that has come back.
    sending: bool,
    /// The text the server last heard, and so what a change is measured against.
    sent: String,
    /// The text as the last frame saw it, and the moment it became that. Together they are
    /// how long the typing has been stopped for, which is what the debounce waits on.
    seen: String,
    seen_at: Instant,
    /// Whether the server has finished reading the project. Kept apart from whether there is
    /// a server at all, because the two mean completely different things to whoever is about
    /// to ask it something - see [`CanAnswer`].
    ready: bool,
    /// Whether a question about that is out, and when the last one was asked. A server that
    /// is starting finishes on its own, so the only way the pane learns of it is by asking
    /// again now and then.
    asking_about_starting: bool,
    asked_about_starting_at: Instant,
}

/// What the server is owed about a document right now.
#[derive(PartialEq, Eq, Debug)]
enum Owed {
    Nothing,
    Open,
    Change,
}

impl Document {
    fn new(ready: bool) -> Self {
        let now = Instant::now();
        Self {
            opened: false,
            sending: false,
            sent: String::new(),
            seen: String::new(),
            seen_at: now,
            ready,
            asking_about_starting: false,
            asked_about_starting_at: now,
        }
    }

    /// Whether it is time to ask again whether the server has finished starting. Never once
    /// the answer is yes, and never twice at once.
    fn wants_the_status_again(&self, now: Instant) -> bool {
        !self.ready
            && !self.asking_about_starting
            && now.duration_since(self.asked_about_starting_at) >= STARTING_IS_ASKED_ABOUT_EVERY
    }

    /// Take note of the text this frame is showing, so the debounce is measured from the
    /// last time it changed rather than from the first time it differed from what was sent.
    fn saw(&mut self, text: &str, now: Instant) {
        if self.seen != text {
            self.seen.clear();
            self.seen.push_str(text);
            self.seen_at = now;
        }
    }

    /// What to send, if anything. Pure, so the debounce is tested without a clock or a
    /// server: the open goes the moment the text is there, and a change waits for the
    /// typing to stop.
    fn owed(&self, text: &str, now: Instant) -> Owed {
        if self.sending {
            return Owed::Nothing;
        }
        if !self.opened {
            return Owed::Open;
        }
        if self.sent == text {
            return Owed::Nothing;
        }
        match now.duration_since(self.seen_at) >= TYPING_SETTLES_IN {
            true => Owed::Change,
            false => Owed::Nothing,
        }
    }
}

/// What a pane does about its language server on the frame it is drawing.
enum Next {
    Nothing,
    /// Nobody has asked yet whether anything serves this file.
    Ask,
    /// The server is still reading the project, and it is time to ask whether it has
    /// finished. Nothing about the text goes with it - this is only about the waiting.
    AskAboutStarting,
    /// The whole of the text, as an open or as a change.
    Send { text: String, opening: bool },
}

/// What the pane owes its server this frame, marking on the pane what it is about to do: a
/// question and a call both take frames to come back, and neither is asked twice.
fn next_of(editor: &mut FileEditor, ctx: &egui::Context) -> Next {
    let (text, served) = editor.text_and_server();
    match served {
        Served::Asking | Served::No => Next::Nothing,
        Served::Unknown => {
            *served = Served::Asking;
            Next::Ask
        }
        Served::Yes(document) => {
            let now = Instant::now();
            document.saw(text, now);
            match document.owed(text, now) {
                Owed::Nothing => {
                    // A change waiting on the typing to stop needs a frame to be sent on,
                    // and a window nobody is typing in draws no more of them.
                    if !document.sending && document.sent != text {
                        ctx.request_repaint_after(TYPING_SETTLES_IN);
                    }
                    // Asked only when there is nothing to send, so a change never waits a
                    // frame behind a question about the waiting.
                    if document.wants_the_status_again(now) {
                        document.asking_about_starting = true;
                        document.asked_about_starting_at = now;
                        return Next::AskAboutStarting;
                    }
                    Next::Nothing
                }
                owed => {
                    document.sending = true;
                    Next::Send {
                        text: text.to_string(),
                        opening: owed == Owed::Open,
                    }
                }
            }
        }
    }
}

impl App {
    /// Keep the language server's copy of a pane's file up with what is on screen.
    ///
    /// Called as the pane draws, because that is where the text is. Nothing goes out for a
    /// file no server serves: the first thing asked is whether there is a server at all,
    /// once, and the answer decides whether this pane ever speaks again.
    pub(crate) fn sync_document(
        &mut self,
        ctx: &egui::Context,
        pane_id: PaneId,
        session_id: &str,
    ) {
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        if !editor.has_a_document_to_keep_up() {
            return;
        }
        let file_path = editor.file_path.clone();
        // Worked out while the pane is borrowed and acted on once it is not: spawning a call
        // takes the whole window.
        match next_of(editor, ctx) {
            Next::Nothing => {}
            Next::Ask => self.ask_whether_a_server_serves(pane_id, session_id, file_path),
            Next::AskAboutStarting => {
                self.ask_whether_the_server_has_finished_starting(pane_id, session_id, file_path);
            }
            Next::Send { text, opening } => {
                self.tell_the_server(pane_id, session_id, file_path, text, opening);
            }
        }
    }

    /// Ask once whether anything serves this file. The document is opened on the frame after
    /// the answer says something does.
    ///
    /// The answer is remembered rather than asked for again each frame: on a `--remote`
    /// session it is a round trip, and a language server appearing on the far machine while
    /// a tab is open is not a thing that happens.
    fn ask_whether_a_server_serves(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: String,
    ) {
        let for_call = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(format!("lsp-status:{pane_id}")),
            move |backend| backend.lsp_status(&for_call, &file_path),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                *editor.server_heard_mut() = match result {
                    // Starting counts as served: what starts it is this document being
                    // opened. Whether it has finished starting is kept rather than folded in,
                    // because a question asked of a server that is still reading the project
                    // comes back empty and reads as an answer - see [`CanAnswer`].
                    Ok(status @ (LspStatus::Starting | LspStatus::Ready)) => {
                        Served::Yes(Document::new(status == LspStatus::Ready))
                    }
                    // A status that could not be had reads as a file with no server, which
                    // is silent and searchable rather than broken.
                    Ok(LspStatus::Unavailable) | Err(_) => Served::No,
                };
            },
        );
    }

    /// Ask again whether the server has finished reading the project.
    ///
    /// Nothing is said about the answer either way. A ⌘-click made while a server is starting
    /// says so, because a click is a direct request and deserves an answer; typing is not,
    /// and a toast for every word typed in the first ten seconds of a file would be worse
    /// than the wait it was explaining.
    fn ask_whether_the_server_has_finished_starting(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: String,
    ) {
        let for_call = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(format!("lsp-starting:{pane_id}")),
            move |backend| backend.lsp_status(&for_call, &file_path),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                let Served::Yes(document) = editor.server_heard_mut() else {
                    return;
                };
                document.asking_about_starting = false;
                // A status that could not be had is not an answer about the waiting: it is
                // asked again in a moment, and until then the pane asks the server nothing.
                if let Ok(status) = result {
                    document.ready = status == LspStatus::Ready;
                }
            },
        );
    }

    /// Send the server the whole of the text, as an open or as a change.
    fn tell_the_server(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: String,
        text: String,
        opening: bool,
    ) {
        let for_call = session_id.to_string();
        let sent = text.clone();
        self.tasks.spawn_keyed(
            Some(format!("lsp-document:{pane_id}")),
            move |backend| match opening {
                true => backend.lsp_did_open(&for_call, &file_path, &text),
                false => backend.lsp_did_change(&for_call, &file_path, &text),
            },
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                let heard = editor.server_heard_mut();
                let Served::Yes(document) = heard else {
                    return;
                };
                document.sending = false;
                match result {
                    Ok(()) => {
                        document.opened = true;
                        document.sent = sent;
                    }
                    // A server that could not be told is one this pane stops talking to.
                    // Trying again every frame would be a call a frame at the exact moment
                    // something is already wrong, and the ⌘-click has the repo to fall back
                    // on either way.
                    Err(_) => *heard = Served::No,
                }
            },
        );
    }

    /// Tell the server the file is gone, when the tab that closed was the last one on it.
    ///
    /// The same file can be open in two tabs, and a document closed under the tab still
    /// showing it would leave that tab asking about a file the server has never heard of.
    pub(crate) fn close_document(&mut self, closed: &FileEditor, session_id: &str) {
        let Served::Yes(document) = closed.server_heard() else {
            return;
        };
        // Never opened, so there is nothing to close.
        if !document.opened {
            return;
        }
        if self
            .model
            .file_editors
            .values()
            .any(|editor| editor.file_path == closed.file_path)
        {
            return;
        }
        let for_call = session_id.to_string();
        let file_path = closed.file_path.clone();
        self.tasks.spawn(
            move |backend| backend.lsp_did_close(&for_call, &file_path),
            // Nothing to do either way: the tab is gone, and a server that did not hear the
            // close has one stale document among the ones it is keeping anyway.
            move |_model, _result| {},
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document is opened the moment there is text to open it with, and nothing more is
    /// sent until it has gone through.
    #[test]
    fn the_document_is_opened_once_and_nothing_is_sent_while_a_call_is_out() {
        let now = Instant::now();
        let mut document = Document::new(true);
        assert_eq!(document.owed("fn one() {}", now), Owed::Open);

        document.sending = true;
        assert_eq!(document.owed("fn one() {}", now), Owed::Nothing);

        // As the open comes back: the server has heard this text, and has heard nothing
        // else until the typing stops again.
        document.sending = false;
        document.opened = true;
        document.sent = "fn one() {}".to_string();
        assert_eq!(document.owed("fn one() {}", now), Owed::Nothing);
    }

    /// The point of the debounce: a burst of typing sends the text once at the end of it
    /// rather than once a keystroke, which on a remote session is the whole file over the
    /// network each time.
    #[test]
    fn typing_sends_the_text_once_it_has_stopped_rather_than_once_a_keystroke() {
        let start = Instant::now();
        let mut document = Document::new(true);
        document.opened = true;
        document.sent = "fn one() {}".to_string();
        document.seen = document.sent.clone();
        document.seen_at = start;

        // A keystroke every fiftieth of a second, none of them worth a call.
        let mut typed = String::new();
        for (key, letter) in "// a comment".chars().enumerate() {
            typed.push(letter);
            let at = start + Duration::from_millis(20 * key as u64);
            document.saw(&typed, at);
            assert_eq!(document.owed(&typed, at), Owed::Nothing, "at key {key}");
        }

        // Still nothing a moment after the last of them, and the whole text once the pause
        // has run its length.
        let last = start + Duration::from_millis(20 * 11);
        assert_eq!(
            document.owed(&typed, last + TYPING_SETTLES_IN / 2),
            Owed::Nothing
        );
        assert_eq!(document.owed(&typed, last + TYPING_SETTLES_IN), Owed::Change);
    }
}
