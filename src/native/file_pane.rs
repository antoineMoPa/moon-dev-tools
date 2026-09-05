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
//! editor's answer back on.

use egui::{Align, Layout, RichText, Ui};
use egui_frames::PaneId;
use egui_moon_editor::{Editor, EditorRequest, Language, Marks};

use crate::native::{
    app::App,
    completing::Completing,
    lsp_document::{CanAnswer, Served},
    theme::{Palette, SMALL_SIZE},
    widgets,
};

/// Between the pane's border and what it is showing.
const PANE_PADDING: i8 = 10;

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
            preview,
            close_confirmed: false,
            reveal: None,
            looking_up: None,
            asks_language_servers,
            served: Served::Unknown,
            completing: Completing::default(),
        }
    }

    /// Whether this pane's ⌘-click asks a language server before it searches the repo.
    pub(crate) fn asks_language_servers(&self) -> bool {
        self.asks_language_servers
    }

    /// Turn the language-server side of this one pane on in a test. A window built for a
    /// test has them off - see [`App::asks_language_servers`] - so a test that is about
    /// this wiring says so, on a file nothing serves, or it starts a real server.
    #[cfg(test)]
    pub(crate) fn asks_language_servers_for_test(&mut self) {
        self.asks_language_servers = true;
    }

    /// Whether this pane has anything to tell a server yet: a window with its language
    /// servers switched off tells nothing, and neither does a file whose text has not
    /// arrived - a document is opened with what is in it.
    pub(super) fn has_a_document_to_keep_up(&self) -> bool {
        self.asks_language_servers && self.saved.is_some()
    }

    /// The text on screen and what the server has heard of it, handed out together because
    /// what to send next is worked out from the two at once.
    pub(super) fn text_and_server(&mut self) -> (&str, &mut Served) {
        (self.code.text(), &mut self.served)
    }

    /// What the server has heard about this file, for the tab that is closing and for the
    /// call that has just come back.
    pub(super) fn server_heard(&self) -> &Served {
        &self.served
    }

    pub(super) fn server_heard_mut(&mut self) -> &mut Served {
        &mut self.served
    }

    /// Whether this pane offers to finish the word being typed at all: a window with its
    /// language servers switched off does not, and neither does a file no server is behind -
    /// which is most of a repo, and is why this is the first thing asked every frame.
    pub(super) fn offers_completions(&self) -> bool {
        self.asks_language_servers && matches!(self.served, Served::Yes(_))
    }

    /// The completion box's state, beside whether the server behind the file could answer a
    /// question about the text on screen at all: what to ask next is worked out from the two
    /// at once.
    pub(super) fn completing_and_server(&mut self) -> (&mut Completing, CanAnswer) {
        let can_answer = self.served.can_answer_about(self.code.text());
        (&mut self.completing, can_answer)
    }

    /// The completion box's state, for an answer that has just come back.
    pub(super) fn completing_mut(&mut self) -> &mut Completing {
        &mut self.completing
    }

    /// What the pane is showing, once it has arrived.
    #[cfg(test)]
    pub(crate) fn content_for_test(&self) -> Option<String> {
        self.saved.clone()
    }

    /// What is on screen, which after typing is not what was fetched.
    #[cfg(test)]
    pub(crate) fn text_for_test(&self) -> &str {
        self.code.text()
    }

    /// How many rows the pane is offering to finish the word being typed with.
    #[cfg(test)]
    pub(crate) fn rows_offered_for_test(&self) -> usize {
        self.completing.on_offer().len()
    }

    /// What those rows read as, for the end-to-end test that wants to see a real server's
    /// names come back.
    #[cfg(test)]
    pub(crate) fn labels_offered_for_test(&self) -> Vec<String> {
        self.completing
            .on_offer()
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    /// Whether the pane has heard back that nothing serves its file, which is the end state
    /// of the language-server side of a pane on most of a repo.
    #[cfg(test)]
    pub(crate) fn heard_no_server_for_test(&self) -> bool {
        matches!(self.served, Served::No)
    }

    /// Type into the file, as the editor widget does.
    #[cfg(test)]
    pub(crate) fn edit_for_test(&mut self, text: &str) {
        self.code.set_text(text.to_string());
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

impl App {
    /// Open a file in a tab of its own, or bring the tab already showing it forward.
    ///
    /// Deferred like every other pane change: this is called from inside the draw of the pane
    /// asking for it, and the tree holding that pane must not be rebuilt underneath it.
    pub(crate) fn open_file_pane(&mut self, session_id: &str, file_path: &str) {
        let already_open = self.model.layout.panes().any(|(_, pane)| {
            matches!(pane, crate::native::panes::Pane::File { file_path: open, .. }
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

        let pane_id = match self.model.layout.find_pane(
            |pane| matches!(pane, Pane::File { file_path: open, .. } if *open == file_path),
        ) {
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
        self.ensure_file_editor(pane_id, &session_id, &file_path);
        if let Some(editor) = self.model.file_editors.get_mut(&pane_id) {
            editor.preview = false;
        }
    }

    /// Put the match a content search found on screen, for a file opened at one of them. The
    /// text may not have arrived yet, so the match is left with the editor and the scroll
    /// happens on the frame that can measure where its line ended up.
    pub(crate) fn reveal_file_match(
        &mut self,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        at: crate::native::panes::OpenAt,
    ) {
        self.ensure_file_editor(pane_id, session_id, file_path);
        let Some(editor) = self.model.file_editors.get_mut(&pane_id) else {
            return;
        };
        editor.reveal = Some(at);
        // A match is in the text of the file, so the text is what the pane shows - a markdown
        // file opens rendered otherwise, where the line does not exist.
        editor.preview = false;
    }

    /// The file a pane is showing, fetched on first sight.
    fn ensure_file_editor(&mut self, pane_id: PaneId, session_id: &str, file_path: &str) {
        if self.model.file_editors.contains_key(&pane_id) {
            return;
        }
        self.model
            .file_editors
            .insert(
                pane_id,
                FileEditor::loading(file_path.to_string(), self.asks_language_servers),
            );
        self.load_file(pane_id, session_id, file_path);
    }

    fn load_file(&mut self, pane_id: PaneId, session_id: &str, file_path: &str) {
        let for_call = session_id.to_string();
        let path = file_path.to_string();
        let for_apply = pane_id;
        self.tasks.spawn_keyed(
            Some(format!("file:{pane_id}")),
            move |backend| backend.file_content(&for_call, &path),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&for_apply) else {
                    return;
                };
                match result {
                    Ok(payload) => {
                        editor.saved = Some(payload.content.clone());
                        editor.code.set_text(payload.content);
                        editor.error = None;
                    }
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
        if editor.saving || !editor.is_dirty() {
            return;
        }
        editor.saving = true;
        let content = editor.code.text().to_string();
        let file_path = editor.file_path.clone();

        let for_call = session_id.to_string();
        let for_write = file_path.clone();
        let written = content.clone();
        let for_apply = pane_id;
        self.tasks.spawn_keyed(
            Some(format!("save:{pane_id}")),
            move |backend| backend.write_file(&for_call, &for_write, &written),
            move |model, result| {
                let Some(editor) = model.file_editors.get_mut(&for_apply) else {
                    return;
                };
                editor.saving = false;
                match result {
                    Ok(()) => {
                        // What is on disk is what was sent, not whatever has been typed since.
                        editor.saved = Some(content);
                        editor.error = None;
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

    pub(crate) fn draw_file_pane(
        &mut self,
        ui: &mut Ui,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
    ) {
        let palette = self.palette_of();
        self.ensure_file_editor(pane_id, session_id, file_path);
        // What the language server behind this file has been told about it, brought up with
        // what the pane is showing.
        let ctx = ui.ctx().clone();
        self.sync_document(&ctx, pane_id, session_id);
        // A name ⌘-clicked in this pane on an earlier frame, once it has been looked up.
        crate::native::definition::follow(self, pane_id, session_id);
        let Some(editor) = self.model.file_editors.get(&pane_id) else {
            return;
        };
        let dirty = editor.is_dirty();
        let saving = editor.saving;
        let error = editor.error.clone();
        let loaded = editor.saved.is_some();
        let markdown = is_markdown(file_path);
        // The find bar selects matches in the laid-out text, so while it is on this pane the
        // text is what is shown, whatever the toggle says.
        let find_is_here = self
            .model
            .find
            .as_ref()
            .is_some_and(|find| find.pane_id == pane_id);
        let previewing = markdown && editor.preview && !find_is_here;

        // The pane's own margin: a frame body runs to the edge of the border, and a file name
        // or a line of code hard against it reads as a mistake.
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(PANE_PADDING, 6))
            .show(ui, |ui| {
                // The actions are laid out first and the path takes what is left, cut with
                // an ellipsis - a task's notes path is long, and a path that runs under the
                // buttons is worse than one that ends in a "…".
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if dirty && !saving && widgets::quiet_button(ui, "[save]").clicked() {
                            self.save_file_pane(pane_id, session_id);
                        }
                        if markdown
                            && loaded
                            && widgets::quiet_button(
                                ui,
                                if previewing { "[edit]" } else { "[preview]" },
                            )
                            .on_hover_text(if previewing {
                                "Edit the file as text"
                            } else {
                                "Render the markdown"
                            })
                            .clicked()
                            && let Some(editor) = self.model.file_editors.get_mut(&pane_id)
                        {
                            editor.preview = !previewing;
                        }
                        if dirty {
                            ui.label(
                                RichText::new(if saving { "saving…" } else { "unsaved" })
                                    .size(SMALL_SIZE - 1.0)
                                    .color(palette.warn),
                            );
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(file_path).strong())
                                    .truncate()
                                    .selectable(true),
                            )
                            // The whole of it, since the pane may only have room for the
                            // start.
                            .on_hover_text(file_path);
                        });
                    });
                });
                widgets::divider(ui, &palette);
                ui.add_space(4.0);

                if let Some(error) = error {
                    ui.label(RichText::new(error).color(palette.warn));
                    return;
                }
                if !loaded {
                    ui.spinner();
                    return;
                }

                if previewing {
                    draw_preview(self, ui, pane_id);
                } else {
                    draw_editor(self, ui, pane_id, session_id, &palette);
                }
            });
    }
}

/// About the measure GitHub lays a readme out at. Prose in a full-width pane puts a whole
/// paragraph on one line, which is more head-turning than reading.
const PREVIEW_MAX_WIDTH: f32 = 900.0;
/// What the rendered page keeps clear on either side even in a narrow pane - text against
/// the pane's edge reads like a mistake.
const PREVIEW_SIDE_PADDING: f32 = 100.0;

/// The markdown rendered as the page it describes, in place of the text of it.
///
/// It renders the edited text rather than the saved one, so flipping to the preview shows
/// what would be saved, not what was.
fn draw_preview(app: &mut App, ui: &mut Ui, pane_id: PaneId) {
    let Some(editor) = app.model.file_editors.get(&pane_id) else {
        return;
    };
    let text = editor.code.text().to_string();

    egui::ScrollArea::vertical()
        .id_salt(("file-pane-preview", pane_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = (ui.available_width() - 2.0 * PREVIEW_SIDE_PADDING)
                .min(PREVIEW_MAX_WIDTH)
                // A pane too narrow for the full padding still gets a readable column.
                .max(ui.available_width() * 0.5);
            let margin = ((ui.available_width() - width) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_max_width(width);
                    egui_commonmark::CommonMarkViewer::new().show(
                        ui,
                        &mut app.model.markdown_cache,
                        &text,
                    );
                });
            });
        });
}

fn draw_editor(app: &mut App, ui: &mut Ui, pane_id: PaneId, session_id: &str, palette: &Palette) {
    let style = palette.editor_style();
    // The find bar over this pane, if there is one. Read out before the editor is borrowed,
    // and handed back what the search turned up once the text has been laid out.
    let searching = app
        .model
        .find
        .as_ref()
        .filter(|find| find.pane_id == pane_id)
        .map(|find| Searching {
            query: find.query.clone(),
            at: find.at,
            pending: find.pending,
        });
    // The editor takes the keyboard it is owed, so a file or a task's notes brought forward
    // can be typed into without clicking into the text first. A file still being fetched, or
    // a markdown file showing its rendered page, has no editor to take it and leaves the
    // offer standing - see `App::follow_front_tab`.
    let takes_keyboard = app.pane_taking_keyboard == Some(pane_id);
    if takes_keyboard {
        app.pane_taking_keyboard = None;
    }

    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    // The match a content search asked for, once the text it is in has arrived.
    let reveal = editor.reveal.clone().filter(|_| editor.saved.is_some());
    // The find bar's matches are marks the editor lays into the text rather than a selection:
    // the bar holds the keyboard while it is open, and an unfocused editor paints no selection
    // at all, so a search would otherwise turn up matches nobody can see.
    let marks = match &searching {
        Some(searching) => egui_moon_editor::matches_in(editor.code.text(), &searching.query),
        None => Vec::new(),
    };

    let output = editor.code.ui(
        ui,
        &style,
        &EditorRequest {
            marks: Marks {
                ranges: &marks,
                current: searching.as_ref().map_or(0, |searching| searching.at),
                // Only when the bar asks: otherwise every frame would drag the caret back to
                // the match and the file could not be edited while the bar is open.
                select_current: searching
                    .as_ref()
                    .is_some_and(|searching| searching.pending),
            },
            line_of_interest: reveal.as_ref().map(|at| at.line),
            focus: takes_keyboard,
            // Command on macOS, ctrl elsewhere - the same modifier the whole of
            // `crate::native::bindings` is written in, and the one a browser makes a link
            // clickable under.
            navigate_modifier: Some(egui::Modifiers::COMMAND),
            // What the language server offered to finish the word being typed with, worked
            // out on an earlier frame - see [`crate::native::completing`]. The editor draws
            // the list and puts the chosen row in; this pane never touches the text.
            completions: editor.completing.on_offer(),
        },
    );
    // The name that was ⌘-clicked, if one was, and where in the text it sits - which is
    // already the position a language server is asked about. Where it is defined is
    // `crate::native::definition`'s business rather than this pane's.
    let navigated_to = output.navigated_to.clone();

    // Only ever the once: the line is where the file was opened, not where it is held, and
    // scrolling away from it has to stick. The find bar takes it from here, marking every
    // match of the query the way it does for one typed into it.
    let mark_match = match reveal.filter(|_| output.line_at.is_some()) {
        Some(at) => {
            editor.reveal = None;
            egui_moon_editor::match_index_on_line(editor.code.text(), &at.query, at.line)
                .map(|index| (at.query, index))
        }
        None => None,
    };

    if searching.is_some()
        && let Some(find) = &mut app.model.find
    {
        find.found(output.marks_laid_out);
    }
    if let Some((query, at)) = mark_match {
        crate::native::find::show_match(app, pane_id, query, at);
    }
    if let Some(word) = navigated_to {
        crate::native::definition::look_up(app, pane_id, session_id, word);
    }
    // What the caret is on now, and what became of the list that was up: whether that is
    // worth a question is `completing`'s to answer.
    crate::native::completing::follow_the_caret(app, pane_id, session_id, ui.ctx(), &output);
}

/// What the find bar is asking of a file pane this frame.
struct Searching {
    query: String,
    at: usize,
    pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(saved: &str, edited: &str) -> FileEditor {
        FileEditor {
            file_path: "src/lib.rs".to_string(),
            saved: Some(saved.to_string()),
            code: Editor::new(edited.to_string()),
            error: None,
            saving: false,
            preview: false,
            close_confirmed: false,
            reveal: None,
            looking_up: None,
            asks_language_servers: false,
            served: Served::Unknown,
            completing: Completing::default(),
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
