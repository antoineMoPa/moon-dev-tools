//! Putting a pane's file to the language server behind it, on the window's own threads.
//!
//! Deciding what a server is owed about an open buffer - opened when the text lands, changed
//! once the typing has stopped, closed when the last tab on it goes - is not this window's
//! policy, and it lives in [`egui_moon_code_ide::Served`]. What is left here is the part
//! that is: the state hangs off a pane, the calls go through [`crate::backend::Backend`] so a
//! `--remote` session reaches servers on the far machine, and they run on
//! [`crate::native::tasks`] rather than on a thread the crate would otherwise own - this
//! window already has a way to do blocking work and hand the answer back on a later frame,
//! and two of those would be one too many.
//!
//! What the answers are then used for is [`crate::native::definition`]'s business.

use egui_frames::PaneId;
use egui_moon_code_ide::{DocumentAsk, LanguageSource, LspStatus};

use crate::native::{app::App, file_pane::FileEditor, language_source::SessionLanguages};

use std::time::Instant;

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
        let owed = {
            let (text, served) = editor.text_and_server();
            served.owed(text, Instant::now())
        };
        // A change waiting on the typing to stop needs a frame to be sent on, and a window
        // nobody is typing in draws no more of them.
        if let Some(after) = owed.draw_again_in {
            ctx.request_repaint_after(after);
        }
        match owed.ask {
            None => {}
            Some(DocumentAsk::WhetherServed) => {
                self.ask_whether_a_server_serves(pane_id, session_id, file_path);
            }
            Some(DocumentAsk::WhetherStillStarting) => {
                self.ask_whether_the_server_has_finished_starting(pane_id, session_id, file_path);
            }
            Some(DocumentAsk::Send { text, opening }) => {
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
            move |backend| Ok(SessionLanguages::new(backend, &for_call).status(&file_path)),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                // A status that could not be had already reads as a file with no server -
                // see [`SessionLanguages::status`] - so there is one answer here, not two.
                editor
                    .server_heard_mut()
                    .served_answered(result.unwrap_or(LspStatus::Unavailable));
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
            move |backend| Ok(SessionLanguages::new(backend, &for_call).status(&file_path)),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                match result {
                    Ok(status) => editor.server_heard_mut().starting_answered(status),
                    // Not an answer about the waiting: it is asked again in a moment, and
                    // until then the pane asks the server nothing.
                    Err(_) => editor.server_heard_mut().starting_could_not_be_asked(),
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
            move |backend| {
                let languages = SessionLanguages::new(backend, &for_call);
                match opening {
                    true => languages.did_open(&file_path, &text),
                    false => languages.did_change(&file_path, &text),
                }
            },
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                match result {
                    Ok(()) => editor.server_heard_mut().heard(sent),
                    // A server that could not be told is one this pane stops talking to.
                    // Trying again every frame would be a call a frame at the exact moment
                    // something is already wrong, and the ⌘-click has the repo to fall back
                    // on either way.
                    Err(_) => editor.server_heard_mut().could_not_be_told(),
                }
            },
        );
    }

    /// Tell the server the file is gone, when the tab that closed was the last one on it.
    ///
    /// The same file can be open in two tabs, and a document closed under the tab still
    /// showing it would leave that tab asking about a file the server has never heard of.
    pub(crate) fn close_document(&mut self, closed: &FileEditor, session_id: &str) {
        // Never opened, so there is nothing to close.
        if !closed.server_heard().was_opened() {
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
            move |backend| SessionLanguages::new(backend, &for_call).did_close(&file_path),
            // Nothing to do either way: the tab is gone, and a server that did not hear the
            // close has one stale document among the ones it is keeping anyway.
            move |_model, _result| {},
        );
    }
}
