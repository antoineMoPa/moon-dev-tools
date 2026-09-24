//! Opening a file in a tab - by name, at a match, at a line of an old version, or on a task's
//! notes - and writing it back.

use egui_frames::PaneId;
use egui_moon_code_ide::LanguageSource;

use crate::native::{app::App, language_source::SessionLanguages};

use super::FileEditor;

impl App {
    /// Open a file in a tab of its own, or bring the tab already showing it forward.
    ///
    /// Deferred like every other pane change: this is called from inside the draw of the pane
    /// asking for it, and the tree holding that pane must not be rebuilt underneath it.
    pub(crate) fn open_file_pane(&mut self, session_id: &str, file_path: &str) {
        let already_open = self.model.layout.panes().any(|(_, pane)| {
            matches!(pane, crate::native::panes::Pane::File { file_path: open, revision: None, .. }
                if open == file_path)
        });
        if already_open || self.pending_action.is_some() {
            return;
        }
        self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
            crate::native::panes::OpenPaneRequest::File {
                session_id: session_id.to_string(),
                file_path: file_path.to_string(),
                at: None,
            },
        ));
    }

    /// Open a file at a place in it - the line `moon open <file>:<line>` typed in a shell
    /// named, or the change a mention of the file in the review stands beside. A file already
    /// in a tab is brought forward and scrolled there rather than left where it is: the ask
    /// was made to look at that place, and the tab may well be behind another one.
    pub(crate) fn open_file_pane_at(
        &mut self,
        session_id: &str,
        file_path: &str,
        at: Option<crate::native::panes::OpenAt>,
    ) {
        self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
            crate::native::panes::OpenPaneRequest::File {
                session_id: session_id.to_string(),
                file_path: file_path.to_string(),
                at,
            },
        ));
    }

    /// Open a file of a task's beside the board: in the frame the other file tabs are in, else
    /// the column the rest of that task's tabs are in, else a new column down the right - the
    /// way a shell opens. It lands in the text editor rather than the rendered page, because a
    /// file opened off a card is opened to be written.
    pub(crate) fn open_notes_pane(
        &mut self,
        session_id: String,
        file_path: String,
        task_id: String,
    ) {
        use crate::native::panes::{Pane, PaneKind};

        let pane_id = match self.model.layout.find_pane(|pane| {
            matches!(pane, Pane::File { file_path: open, revision: None, .. } if *open == file_path)
        }) {
            Some((pane, _)) => {
                // The tab was already open, on the file of the repo rather than on the task's
                // copy of it: opening it from a card is what puts it on that task, and what
                // marks the card while it is in front.
                if let Some(Pane::File { task_id: on, .. }) = self.model.layout.pane_mut(pane) {
                    *on = Some(task_id.clone());
                }
                self.model.layout.focus_pane(pane);
                pane
            }
            None => {
                let pane = Pane::File {
                    session_id: session_id.clone(),
                    file_path: file_path.clone(),
                    task_id: Some(task_id.clone()),
                    revision: None,
                };
                let active = self.model.layout.active_frame();
                match self
                    .model
                    .layout
                    .frame_holding(active, |pane| pane.kind() == PaneKind::File)
                    .or_else(|| self.task_column())
                {
                    Some(frame) => self.model.layout.add_pane(frame, pane, None),
                    None => self.model.layout.add_pane_against_edge(
                        egui_frames::DropSide::Right,
                        egui_frames::DEFAULT_EDGE_SHARE,
                        pane,
                    ),
                }
            }
        };
        self.ensure_file_editor(pane_id, &session_id, &file_path, None);
        if let Some(editor) = self.model.file_editors.get_mut(&pane_id) {
            editor.preview = false;
        }
    }

    /// Put a line of a file on screen - the match a content search found, or the change a
    /// review pointed at - for a file opened at it. The text may not have arrived yet, so
    /// the place is left with the editor and the scroll happens on the frame that can
    /// measure where its line ended up.
    pub(crate) fn reveal_file_match(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        at: crate::native::panes::OpenAt,
    ) {
        self.ensure_file_editor(pane_id, session_id, file_path, None);
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        editor.reveal = Some(at);
        // A match is in the text of the file, so the text is what the pane shows - a markdown
        // file opens rendered otherwise, where the line does not exist.
        editor.preview = false;
    }

    /// Put a line of an old version of a file on screen, for a tab opened off a blame onto
    /// the version before a change - see [`crate::native::blame`]. The text may not have
    /// arrived yet, so the line is left with the editor the way a search's match is, with
    /// nothing to mark on it.
    pub(crate) fn reveal_file_line(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        revision: &str,
        line: usize,
    ) {
        self.ensure_file_editor(pane_id, session_id, file_path, Some(revision));
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        editor.reveal = Some(crate::native::panes::OpenAt {
            line,
            query: String::new(),
        });
    }

    /// Start a tab on a path nothing is at yet - see [`FileEditor::new_file`]. A tab already
    /// open on the path keeps what it has: the new file is already being written there.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn begin_new_file(&mut self, pane_id: PaneId, file_path: &str) {
        let asks_language_servers = self.asks_language_servers;
        self.model
            .file_editors
            .entry(pane_id)
            .or_insert_with(|| FileEditor::new_file(file_path.to_string(), asks_language_servers));
    }

    /// The file a pane is showing, fetched on first sight - as it is, or as the commit
    /// `revision` had it.
    pub(super) fn ensure_file_editor(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        revision: Option<&str>,
    ) {
        if self.model.file_editors.contains_key(&pane_id) {
            return;
        }
        let editor = match revision {
            Some(revision) => FileEditor::at_revision(file_path.to_string(), revision.to_string()),
            None => FileEditor::loading(file_path.to_string(), self.asks_language_servers),
        };
        self.model.file_editors.insert(pane_id, editor);
        self.load_file(pane_id, session_id, file_path, revision);
    }

    fn load_file(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        revision: Option<&str>,
    ) {
        let for_call = session_id.to_string();
        let path = file_path.to_string();
        let revision = revision.map(str::to_string);
        let for_apply = pane_id;
        self.tasks.spawn_keyed(
            Some(format!("file:{pane_id}")),
            move |backend| match &revision {
                Some(revision) => backend.file_content_at(&for_call, &path, revision),
                None => backend.file_content(&for_call, &path),
            },
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&for_apply) else {
                    return;
                };
                match result {
                    Ok(payload) => editor.take_what_is_on_disk(payload),
                    Err(error) => editor.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Write the file a pane is editing back to the working tree.
    pub(crate) fn save_file_pane(&mut self, pane_id: PaneId, session_id: &str) {
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        // A file outside the repo, or an old version of one, is read-only, so there is nothing
        // to write even if a chord asked for it: the write would be refused repo-side, and the
        // refusal would read as a failure rather than as the answer it is.
        // A new file is saved even with nothing typed in it: saving it is what makes it.
        if editor.saving || editor.is_read_only() || (editor.on_disk && !editor.is_dirty()) {
            return;
        }
        editor.saving = true;
        editor.writes_sent += 1;
        let content = editor.code.text().to_string();
        let file_path = editor.file_path.clone();

        let for_call = session_id.to_string();
        let for_write = file_path.clone();
        let written = content.clone();
        let for_apply = pane_id;
        let tells_the_server = editor.server_heard().was_opened();
        // The first save of a tab opened on a path nothing was at is what creates the file.
        let creates = !editor.on_disk;
        self.tasks.spawn_keyed(
            Some(format!("save:{pane_id}")),
            move |backend| {
                if creates {
                    backend.create_file(&for_call, &for_write, &written)?;
                } else {
                    backend.write_file(&for_call, &for_write, &written)?;
                }
                // The file is written whatever the server makes of it: one that did not hear
                // only misses the check a save sets off, and hears the next save.
                if tells_the_server {
                    let _ = SessionLanguages::new(backend, &for_call).did_save(&for_write);
                }
                Ok(())
            },
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&for_apply) else {
                    return;
                };
                editor.saving = false;
                match result {
                    Ok(()) => {
                        // What is on disk is what was sent, not whatever has been typed since.
                        editor.saved = Some(content);
                        editor.on_disk = true;
                        editor.error = None;
                        // Written over whatever another program had put there.
                        editor.written_elsewhere = None;
                        editor.reload_confirmed = false;
                    }
                    Err(error) => {
                        let message = format!("{error}");
                        editor.error = Some(message.clone());
                        model.error(format!("could not save {file_path}: {message}"));
                    }
                }
            },
        );
    }

    /// Everything a tab strip needs to know about a file pane: its title, and whether it has
    /// unsaved edits to mark.
    pub(crate) fn file_pane_is_dirty(&self, pane_id: PaneId) -> bool {
        self.model
            .file_editors
            .get(&pane_id)
            .is_some_and(FileEditor::is_dirty)
    }
}
