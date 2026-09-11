//! Whether the file a tab is showing was written by something other than the tab - an agent, a
//! formatter, a checkout - and what the tab does about it.
//!
//! The tab reads its file again every [`DISK_CHECK_INTERVAL`] while it is on screen. A tab
//! with nothing typed into it shows the file as it now is, since nothing is lost by it. A tab
//! with edits of its own keeps them and holds the other version back, the header saying the
//! file changed on disk, until `[reload]` takes that version or `[save]` writes the edits over
//! it.

use std::time::{Duration, Instant};

use egui_frames::PaneId;

use super::FileEditor;
use crate::{api::FileContentPayload, native::app::App};

/// How often a file tab on screen reads its file again. The commit pane reads what git would
/// commit about as often.
const DISK_CHECK_INTERVAL: Duration = Duration::from_millis(1000);

impl FileEditor {
    /// Show the file as a read of it has it, the way opening it does.
    pub(super) fn take_what_is_on_disk(&mut self, payload: FileContentPayload) {
        self.saved = Some(payload.content.clone());
        self.code.set_text(payload.content);
        // What the fringe marks the new lines of the text against, so what has been written
        // since the last commit - saved or not - stands out.
        self.code.set_base(payload.committed);
        self.error = None;
        self.outside_the_repo = payload.outside_the_repo;
        self.written_elsewhere = None;
        self.reload_confirmed = false;
    }

    /// What a check of the disk found. A file nobody else wrote is left alone. One written
    /// elsewhere is put on screen when there is nothing typed here to lose, and otherwise held
    /// until `[reload]` or `[save]` settles which of the two stays.
    fn disk_answered(&mut self, payload: FileContentPayload) {
        if self.saved.as_ref() == Some(&payload.content) {
            // Also how a change made elsewhere and then undone there stops being announced.
            self.written_elsewhere = None;
            self.reload_confirmed = false;
            return;
        }
        if self.is_dirty() {
            self.written_elsewhere = Some(payload);
            return;
        }
        self.take_what_is_on_disk(payload);
    }

    /// Whether the header is saying another program wrote the file under unsaved edits.
    #[cfg(test)]
    pub(crate) fn is_written_elsewhere_for_test(&self) -> bool {
        self.written_elsewhere.is_some()
    }
}

impl App {
    /// Read the file a pane is showing again, once every [`DISK_CHECK_INTERVAL`], to see
    /// whether something other than this pane wrote it. Only a pane on screen is drawn, so
    /// only a pane on screen reads - a tab behind another catches up once it is in front.
    pub(super) fn check_the_disk(&mut self, pane_id: PaneId, session_id: &str) {
        let key = format!("file-check:{pane_id}");
        if self.tasks.is_busy(&key) {
            return;
        }
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        let due = editor
            .last_disk_check
            .is_none_or(|last| last.elapsed() >= DISK_CHECK_INTERVAL);
        // Nothing to compare against before the text has arrived, a file being written is
        // about to be what this pane sent, and a new file has nothing on disk to read yet.
        if editor.saved.is_none() || editor.saving || !due || !editor.on_disk {
            return;
        }
        editor.last_disk_check = Some(Instant::now());
        let writes_sent = editor.writes_sent;

        let for_call = session_id.to_string();
        let path = editor.file_path.clone();
        self.tasks.spawn_keyed(
            Some(key),
            move |backend| backend.file_content(&for_call, &path),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                if editor.writes_sent != writes_sent {
                    return;
                }
                // A read that failed says nothing about who wrote the file, so the pane stays
                // as it is: the next check reads again, and a save says for itself when the
                // file is not there to write.
                let Ok(payload) = result else {
                    return;
                };
                editor.disk_answered(payload);
            },
        );
    }

    /// Put the file as another program wrote it in place of the edits in the pane. It takes
    /// two presses, the first turning the button into the question, since the edits go.
    pub(super) fn reload_file_pane(&mut self, pane_id: PaneId) {
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        if !editor.reload_confirmed {
            editor.reload_confirmed = true;
            return;
        }
        let payload = editor
            .written_elsewhere
            .take()
            .expect("[reload] is only offered on a file written elsewhere");
        editor.take_what_is_on_disk(payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::file_pane::tests::editor_with;

    fn on_disk(content: &str) -> FileContentPayload {
        FileContentPayload {
            file_path: "src/lib.rs".to_string(),
            content: content.to_string(),
            outside_the_repo: false,
            committed: Some(String::new()),
        }
    }

    /// A clean pane takes what another program wrote; one with edits keeps them and holds
    /// the other version back; and the other version going away again stops the announcing.
    #[test]
    fn a_file_written_elsewhere_is_taken_only_when_no_edit_is_lost() {
        let mut clean = editor_with("one\n", "one\n");
        clean.disk_answered(on_disk("two\n"));
        assert_eq!(clean.code.text(), "two\n");
        assert!(!clean.is_dirty());
        assert!(clean.written_elsewhere.is_none());

        let mut edited = editor_with("one\n", "mine\n");
        edited.disk_answered(on_disk("theirs\n"));
        assert_eq!(edited.code.text(), "mine\n");
        assert!(edited.written_elsewhere.is_some());

        edited.disk_answered(on_disk("one\n"));
        assert_eq!(edited.code.text(), "mine\n");
        assert!(edited.written_elsewhere.is_none());
    }
}
