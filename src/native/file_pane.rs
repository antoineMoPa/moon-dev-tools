//! One file of the repo, open in a tab of its own for reading and editing.
//!
//! Scrolling is what makes this feel like an editor rather than a text dump: the line numbers
//! stay put while the code slides sideways under them, which is what every editor does and
//! what a plain two-axis scroll area does not. Vertically the fringe and the code move
//! together, so a line number is always beside its line.

use egui::{Align, Layout, RichText, Ui, vec2};

use crate::native::{
    app::App,
    theme::{CODE_SIZE, Palette, SMALL_SIZE},
    widgets,
};

/// Wide enough for five digits, which covers any file worth opening in a review.
const FRINGE_WIDTH: f32 = 46.0;
/// Between the pane's border and what it is showing.
const PANE_PADDING: i8 = 10;

/// A file being read or edited, and what has happened to it since it was opened.
pub(crate) struct FileEditor {
    pub(crate) file_path: String,
    /// The text as it is on disk, as far as this window knows.
    saved: Option<String>,
    /// The text in the editor, which is what gets written.
    edited: String,
    error: Option<String>,
    saving: bool,
    /// Set when a close was asked for while there were unsaved edits: the second press goes
    /// through, the way discarding a hunk does.
    pub(crate) close_confirmed: bool,
}

impl FileEditor {
    fn loading(file_path: String) -> Self {
        Self {
            file_path,
            saved: None,
            edited: String::new(),
            error: None,
            saving: false,
            close_confirmed: false,
        }
    }

    /// What the pane is showing, once it has arrived.
    #[cfg(test)]
    pub(crate) fn content_for_test(&self) -> Option<String> {
        self.saved.clone()
    }

    /// Type into the file, as the editor widget does.
    #[cfg(test)]
    pub(crate) fn edit_for_test(&mut self, text: &str) {
        self.edited = text.to_string();
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.saved
            .as_ref()
            .is_some_and(|saved| *saved != self.edited)
    }
}

impl App {
    /// Open a file in a tab of its own, or bring the tab already showing it forward.
    ///
    /// Deferred like every other pane change: this is called from inside the draw of the pane
    /// asking for it, and the tree holding that pane must not be rebuilt underneath it.
    pub(crate) fn open_file_pane(&mut self, session_id: &str, file_path: &str) {
        let already_open = self.model.layout.panes.values().any(|pane| {
            matches!(pane, crate::native::layout::Pane::File { file_path: open, .. }
                if open == file_path)
        });
        if already_open || self.pending_action.is_some() {
            return;
        }
        self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
            crate::native::layout::OpenPaneRequest::File {
                session_id: session_id.to_string(),
                file_path: file_path.to_string(),
            },
        ));
    }

    /// The file a pane is showing, fetched on first sight.
    fn ensure_file_editor(&mut self, pane_id: &str, session_id: &str, file_path: &str) {
        if self.model.file_editors.contains_key(pane_id) {
            return;
        }
        self.model
            .file_editors
            .insert(pane_id.to_string(), FileEditor::loading(file_path.to_string()));
        self.load_file(pane_id, session_id, file_path);
    }

    fn load_file(&mut self, pane_id: &str, session_id: &str, file_path: &str) {
        let for_call = session_id.to_string();
        let path = file_path.to_string();
        let for_apply = pane_id.to_string();
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
                        editor.edited = payload.content;
                        editor.error = None;
                    }
                    Err(error) => editor.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Write the file a pane is editing back to the working tree.
    pub(crate) fn save_file_pane(&mut self, pane_id: &str, session_id: &str) {
        let Some(editor) = self.model.file_editors.get_mut(pane_id) else {
            return;
        };
        if editor.saving || !editor.is_dirty() {
            return;
        }
        editor.saving = true;
        let content = editor.edited.clone();
        let file_path = editor.file_path.clone();

        let for_call = session_id.to_string();
        let for_write = file_path.clone();
        let written = content.clone();
        let for_apply = pane_id.to_string();
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
    pub(crate) fn file_pane_is_dirty(&self, pane_id: &str) -> bool {
        self.model
            .file_editors
            .get(pane_id)
            .is_some_and(FileEditor::is_dirty)
    }

    pub(crate) fn draw_file_pane(
        &mut self,
        ui: &mut Ui,
        pane_id: &str,
        session_id: &str,
        file_path: &str,
    ) {
        let palette = self.palette_of();
        self.ensure_file_editor(pane_id, session_id, file_path);
        let Some(editor) = self.model.file_editors.get(pane_id) else {
            return;
        };
        let dirty = editor.is_dirty();
        let saving = editor.saving;
        let error = editor.error.clone();
        let loaded = editor.saved.is_some();

        // The pane's own margin: a frame body runs to the edge of the border, and a file name
        // or a line of code hard against it reads as a mistake.
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(PANE_PADDING, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(file_path).strong()).selectable(true),
                    );
                    if dirty {
                        ui.label(
                            RichText::new(if saving { "saving…" } else { "unsaved" })
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.warn),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if dirty && !saving && widgets::quiet_button(ui, "[save]").clicked() {
                            self.save_file_pane(pane_id, session_id);
                        }
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

                draw_editor(self, ui, pane_id, &palette);
            });
    }
}

/// The fringe and the code, scrolling together down the page and apart across it.
/// Every place the query appears in the text, as character ranges — which is what egui's
/// text cursor counts in, so a match can be handed straight to the editor as a selection.
///
/// Matched without regard for case, the way the find bar does everywhere else.
pub(crate) fn matches_in(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let haystack: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    // A character that changes length when lowercased would put the character indexes out of
    // step with the text the editor holds, so that text is matched exactly instead.
    let (haystack, needle) = if haystack.len() == text.chars().count() {
        (haystack, needle)
    } else {
        (text.chars().collect(), query.chars().collect())
    };
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }

    (0..=haystack.len() - needle.len())
        .filter(|start| haystack[*start..start + needle.len()] == needle[..])
        .map(|start| start..start + needle.len())
        .collect()
}

fn draw_editor(app: &mut App, ui: &mut Ui, pane_id: &str, palette: &Palette) {
    let font = egui::FontId::monospace(CODE_SIZE);
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
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
    let mut found: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt(("file-pane", pane_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let Some(editor) = app.model.file_editors.get_mut(pane_id) else {
                    return;
                };
                let line_count = editor.edited.lines().count().max(1);

                // The fringe is outside the horizontal scroll area, so scrolling the code
                // sideways slides it under numbers that stay where they are.
                let fringe_height = row_height * line_count as f32;
                let (fringe, _) = ui.allocate_exact_size(
                    vec2(FRINGE_WIDTH, fringe_height),
                    egui::Sense::hover(),
                );
                if ui.is_rect_visible(fringe) {
                    for line in 0..line_count {
                        let at = egui::pos2(
                            fringe.max.x - 6.0,
                            fringe.min.y + row_height * line as f32,
                        );
                        ui.painter().text(
                            at,
                            egui::Align2::RIGHT_TOP,
                            format!("{}", line + 1),
                            egui::FontId::monospace(CODE_SIZE - 1.0),
                            palette.muted,
                        );
                    }
                }

                egui::ScrollArea::horizontal()
                    .id_salt(("file-pane-code", pane_id))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let output = egui::TextEdit::multiline(&mut editor.edited)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(line_count)
                            .show(ui);
                        if let Some(searching) = &searching {
                            found = Some(show_match(ui, &editor.edited, searching, output));
                        }
                    });
            });
        });

    if let Some(total) = found
        && let Some(find) = &mut app.model.find
    {
        find.found(total);
    }
}

/// What the find bar is asking of a file pane this frame.
struct Searching {
    query: String,
    at: usize,
    pending: bool,
}

/// Select the current match in the laid-out editor and bring it into view. Returns how many
/// matches there were, for the bar's tally.
fn show_match(
    ui: &mut Ui,
    text: &str,
    searching: &Searching,
    mut output: egui::text_edit::TextEditOutput,
) -> usize {
    let matches = matches_in(text, &searching.query);
    // Only when the bar asks: otherwise every frame would drag the caret back to the match
    // and the file could not be edited while the bar is open.
    if !searching.pending {
        return matches.len();
    }
    let Some(range) = matches.get(searching.at) else {
        return matches.len();
    };

    let cursors = egui::text::CCursorRange::two(
        egui::text::CCursor::new(range.start),
        egui::text::CCursor::new(range.end),
    );
    let at = output
        .galley
        .pos_from_cursor(egui::text::CCursor::new(range.start))
        .translate(output.galley_pos.to_vec2());
    ui.scroll_to_rect(at, Some(egui::Align::Center));

    output.state.cursor.set_char_range(Some(cursors));
    output.state.store(ui.ctx(), output.response.id);
    matches.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(saved: &str, edited: &str) -> FileEditor {
        FileEditor {
            file_path: "src/lib.rs".to_string(),
            saved: Some(saved.to_string()),
            edited: edited.to_string(),
            error: None,
            saving: false,
            close_confirmed: false,
        }
    }

    #[test]
    fn a_match_is_a_character_range_of_the_text() {
        let text = "fn greet() {}\nfn greet_again() {}\n";
        let found = matches_in(text, "greet");

        assert_eq!(found.len(), 2);
        let first = found[0].clone();
        assert_eq!(
            text.chars().skip(first.start).take(first.len()).collect::<String>(),
            "greet"
        );
    }

    #[test]
    fn case_is_not_what_a_search_is_about() {
        assert_eq!(matches_in("Cargo.toml", "cargo").len(), 1);
        assert_eq!(matches_in("Cargo.toml", "CARGO").len(), 1);
    }

    /// A match past the first line has to count the newline, or the editor would put the
    /// caret somewhere else entirely.
    #[test]
    fn a_match_on_a_later_line_counts_the_line_breaks_before_it() {
        let text = "one\ntwo\nthree";
        let found = matches_in(text, "three");

        assert_eq!(found, vec![8..13]);
    }

    #[test]
    fn nothing_matches_an_empty_query_or_a_query_that_is_not_there() {
        assert!(matches_in("hello", "").is_empty());
        assert!(matches_in("hello", "absent").is_empty());
        assert!(matches_in("hi", "far too long").is_empty());
    }

    #[test]
    fn a_file_is_dirty_only_once_it_differs_from_what_was_saved() {
        assert!(!editor_with("fn one() {}", "fn one() {}").is_dirty());
        assert!(editor_with("fn one() {}", "fn two() {}").is_dirty());
        // Nothing has arrived yet, so there is nothing to have changed.
        assert!(!FileEditor::loading("src/lib.rs".to_string()).is_dirty());
    }

}
