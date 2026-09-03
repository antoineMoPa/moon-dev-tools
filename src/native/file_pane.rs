//! One file of the repo, open in a tab of its own for reading and editing.
//!
//! Scrolling is what makes this feel like an editor rather than a text dump: the line numbers
//! stay put while the code slides sideways under them, which is what every editor does and
//! what a plain two-axis scroll area does not. Vertically the fringe and the code move
//! together, so a line number is always beside its line.

use egui::{Align, Layout, RichText, Ui, vec2};
use egui_frames::PaneId;

use crate::native::{
    app::App,
    theme::{CODE_SIZE, Palette, SMALL_SIZE},
    widgets,
};

/// Wide enough for five digits, which covers any file worth opening in a review.
const FRINGE_WIDTH: f32 = 46.0;
/// Between the pane's border and what it is showing.
const PANE_PADDING: i8 = 10;
/// Between the edge of the editor and the text in it, which is what a `TextEdit` keeps clear
/// by default and is kept here because the frame around the text is ours.
const TEXT_MARGIN: egui::Margin = egui::Margin::symmetric(4, 2);

/// A file being read or edited, and what has happened to it since it was opened.
pub(crate) struct FileEditor {
    pub(crate) file_path: String,
    /// The text as it is on disk, as far as this window knows.
    saved: Option<String>,
    /// The text in the editor, which is what gets written.
    edited: String,
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
}

impl FileEditor {
    fn loading(file_path: String) -> Self {
        let preview = is_markdown(&file_path);
        Self {
            file_path,
            saved: None,
            edited: String::new(),
            error: None,
            saving: false,
            preview,
            close_confirmed: false,
            reveal: None,
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

        let pane_id = match self
            .model
            .layout
            .find_pane(|pane| matches!(pane, Pane::File { file_path: open, .. } if *open == file_path))
        {
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
            .insert(pane_id, FileEditor::loading(file_path.to_string()));
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
                        editor.edited = payload.content;
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
        let content = editor.edited.clone();
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
                    draw_editor(self, ui, pane_id, &palette);
                }
            });
    }
}

/// The fringe and the code, scrolling together down the page and apart across it.
/// Every place the query appears in the text, as character ranges - which is what egui's
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

    // Stepped past each match rather than to the next start, so a query that overlaps
    // itself (like ".." over "...") doesn't turn up two matches sharing a character - which
    // would hand the layout job in `marked_text` a pair of ranges out of order.
    let mut matches = Vec::new();
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        if haystack[start..start + needle.len()] == needle[..] {
            matches.push(start..start + needle.len());
            start += needle.len();
        } else {
            start += 1;
        }
    }
    matches
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
    let text = editor.edited.clone();

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

fn draw_editor(app: &mut App, ui: &mut Ui, pane_id: PaneId, palette: &Palette) {
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
    let mut bring_into_view: Option<egui::Rect> = None;
    // Where the line a content search opened the file at was laid out, once the text is on
    // screen to measure.
    let mut reveal_at: Option<egui::Rect> = None;
    // The query to hand the find bar, and which of its matches this file was opened at.
    let mut mark_match: Option<(String, usize)> = None;
    // The editor takes the keyboard it is owed, so a file or a task's notes brought forward
    // can be typed into without clicking into the text first. A file still being fetched, or
    // a markdown file showing its rendered page, has no editor to take it and leaves the
    // offer standing - see `App::follow_front_tab`.
    let takes_keyboard = app.pane_taking_keyboard == Some(pane_id);
    if takes_keyboard {
        app.pane_taking_keyboard = None;
    }

    egui::ScrollArea::vertical()
        .id_salt(("file-pane", pane_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
                    return;
                };
                let line_count = editor.edited.lines().count().max(1);
                // The match a content search asked for, once the text it is in has arrived.
                let reveal = editor
                    .reveal
                    .clone()
                    .filter(|_| editor.saved.is_some());
                // A short file still gets an editor down to the bottom of the pane, so the
                // text sits on a page rather than in a box the size of what it holds.
                let rows_on_screen = (ui.available_height() / row_height).floor() as usize;

                // The fringe is outside the horizontal scroll area, so scrolling the code
                // sideways slides it under numbers that stay where they are. Its height is
                // only an estimate for layout - the numbers are painted where the laid-out
                // text really put each line.
                let fringe_height = row_height * line_count as f32;
                let (fringe, _) = ui.allocate_exact_size(
                    vec2(FRINGE_WIDTH, fringe_height),
                    egui::Sense::hover(),
                );
                // The fringe's painter, kept from out here: the one inside the horizontal
                // scroll area clips to the code, and the numbers sit left of it.
                let painter = ui.painter().clone();
                let muted = palette.muted;

                egui::ScrollArea::horizontal()
                    .id_salt(("file-pane-code", pane_id))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        // The find bar's matches are drawn into the text itself rather
                        // than left to the editor's selection: the bar holds the keyboard
                        // while it is open, and an unfocused editor paints no selection at
                        // all, so a search would otherwise turn up matches nobody can see.
                        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                            let job = match &searching {
                                Some(searching) => {
                                    marked_text(ui, text.as_str(), searching, *palette, wrap)
                                }
                                None => plain_text(ui, text.as_str(), wrap),
                            };
                            ui.fonts_mut(|fonts| fonts.layout_job(job))
                        };
                        // A frame of its own, in place of the boxed-in one a `TextEdit`
                        // draws: no rounded corners, no border, and no accent-coloured ring
                        // when it holds the keyboard - the pane's frame already shows that,
                        // and the text of a file should read as the page of an editor
                        // rather than as a form field on it.
                        // Nothing painted behind the text either: the pane's own background
                        // carries through, so the code and the fringe of numbers beside it
                        // sit on one surface instead of the text being a panel on top.
                        let frame = egui::Frame::new().inner_margin(TEXT_MARGIN);
                        let output = egui::TextEdit::multiline(&mut editor.edited)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .frame(frame)
                            .margin(TEXT_MARGIN)
                            .desired_width(f32::INFINITY)
                            .desired_rows(line_count.max(rows_on_screen))
                            .layouter(&mut layouter)
                            .show(ui);
                        if takes_keyboard {
                            output.response.request_focus();
                        }

                        // Each number at the height the galley actually gave its line -
                        // counting multiples of the font's row height drifts away from the
                        // text within a screen, because the editor lays its rows out with
                        // spacing of its own. A row only starts a line when the row before
                        // it ended in a newline, which is what keeps the numbers right if
                        // the text ever wraps.
                        let visible = painter.clip_rect().expand(row_height).y_range();
                        let mut line = 0;
                        let mut starts_line = true;
                        for placed in &output.galley.rows {
                            let starts = starts_line;
                            starts_line = placed.ends_with_newline;
                            if !starts {
                                continue;
                            }
                            line += 1;
                            let y = output.galley_pos.y + placed.pos.y;
                            if Some(line) == reveal.as_ref().map(|at| at.line) {
                                reveal_at = Some(egui::Rect::from_min_size(
                                    egui::pos2(output.galley_pos.x, y),
                                    vec2(1.0, row_height),
                                ));
                            }
                            if !visible.contains(y) {
                                continue;
                            }
                            painter.text(
                                egui::pos2(fringe.max.x - 6.0, y),
                                egui::Align2::RIGHT_TOP,
                                line.to_string(),
                                egui::FontId::monospace(CODE_SIZE - 1.0),
                                muted,
                            );
                        }

                        if let Some(searching) = &searching {
                            let shown = show_match(ui, &editor.edited, searching, output);
                            found = Some(shown.total);
                            bring_into_view = shown.current;
                        }
                        // Only ever the once: the line is where the file was opened, not
                        // where it is held, and scrolling away from it has to stick. The
                        // find bar takes it from here, marking every match of the query the
                        // way it does for one typed into it.
                        if let Some(at) = reveal.filter(|_| reveal_at.is_some()) {
                            bring_into_view = reveal_at;
                            editor.reveal = None;
                            mark_match = match_index_on_line(&editor.edited, &at.query, at.line)
                                .map(|index| (at.query, index));
                        }
                    });

                // Asked for out here, where the pane's vertical scroll can hear it: the
                // horizontal area around the code takes both axes' scroll targets so they
                // cannot leak, and drops the one it has no bar for - so a match below the
                // fold, asked for from inside it, would never be scrolled to.
                if let Some(rect) = bring_into_view {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
            });
        });

    if let Some(total) = found
        && let Some(find) = &mut app.model.find
    {
        find.found(total);
    }
    if let Some((query, at)) = mark_match {
        crate::native::find::show_match(app, pane_id, query, at);
    }
}

/// Which match of the file the one on `line` is, counting from zero, which is what the find
/// bar calls the current match. A line holding more than one is stepped to at its first.
fn match_index_on_line(text: &str, query: &str, line: usize) -> Option<usize> {
    let mut before = 0;
    for (index, text_of_line) in text.split_inclusive('\n').enumerate() {
        let on_this_line = matches_in(text_of_line, query).len();
        if index + 1 == line {
            return (on_this_line > 0).then_some(before);
        }
        before += on_this_line;
    }
    None
}

/// What the find bar is asking of a file pane this frame.
struct Searching {
    query: String,
    at: usize,
    pending: bool,
}

/// The text laid out the way the editor would lay it out on its own.
fn plain_text(ui: &Ui, text: &str, wrap_width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.append(text, 0.0, code_format(ui, None));
    job
}

/// The text laid out with every match of the query behind it, and the one the bar has
/// stepped to marked more strongly than the rest, so stepping is visible without the others
/// disappearing.
fn marked_text(
    ui: &Ui,
    text: &str,
    searching: &Searching,
    palette: Palette,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let matches = byte_matches_in(text, &searching.query);
    if matches.is_empty() {
        return plain_text(ui, text, wrap_width);
    }

    // The same tint a match gets in a review, which is strong enough to pick one out of the
    // code without hiding it. The current match is underlined rather than tinted harder: a
    // background solid enough to stand out from the others would take the text with it.
    let tint = palette.accent.linear_multiply(0.35);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut cut = 0;
    for (index, range) in matches.iter().enumerate() {
        job.append(&text[cut..range.start], 0.0, code_format(ui, None));
        let mut format = code_format(ui, Some(tint));
        if index == searching.at {
            format.underline = egui::Stroke::new(1.0, palette.accent);
        }
        job.append(&text[range.clone()], 0.0, format);
        cut = range.end;
    }
    job.append(&text[cut..], 0.0, code_format(ui, None));
    job
}

/// One run of the editor's text: the font and colour a plain `TextEdit` would have given it,
/// over whatever the find bar is marking it with.
fn code_format(ui: &Ui, background: Option<egui::Color32>) -> egui::TextFormat {
    let visuals = ui.visuals();
    egui::TextFormat {
        font_id: egui::TextStyle::Monospace.resolve(ui.style()),
        color: visuals
            .override_text_color
            .unwrap_or_else(|| visuals.widgets.inactive.text_color()),
        background: background.unwrap_or(egui::Color32::TRANSPARENT),
        ..Default::default()
    }
}

/// The same matches `matches_in` finds, as byte ranges of the text - which is what a layout
/// job's runs are cut at, where the editor's cursor counts characters.
fn byte_matches_in(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let starts: Vec<usize> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .collect();
    matches_in(text, query)
        .into_iter()
        .map(|range| starts[range.start]..starts[range.end])
        .collect()
}

/// What laying the search over the editor turned up.
struct Shown {
    /// How many matches there were, for the bar's tally.
    total: usize,
    /// Where the current match ended up on screen, when there is one to be brought into
    /// view. Scrolled to by the caller rather than here - see `draw_editor`.
    current: Option<egui::Rect>,
}

/// Select the current match in the laid-out editor and say where it landed.
fn show_match(
    ui: &mut Ui,
    text: &str,
    searching: &Searching,
    mut output: egui::text_edit::TextEditOutput,
) -> Shown {
    let matches = matches_in(text, &searching.query);
    // Only when the bar asks: otherwise every frame would drag the caret back to the match
    // and the file could not be edited while the bar is open.
    if !searching.pending {
        return Shown { total: matches.len(), current: None };
    }
    let Some(range) = matches.get(searching.at) else {
        return Shown { total: matches.len(), current: None };
    };

    let cursors = egui::text::CCursorRange::two(
        egui::text::CCursor::new(range.start),
        egui::text::CCursor::new(range.end),
    );
    let at = output
        .galley
        .pos_from_cursor(egui::text::CCursor::new(range.start))
        .translate(output.galley_pos.to_vec2());
    // Sideways from in here, where the code's own scroll can hear it.
    ui.scroll_to_rect(at, Some(Align::Center));

    output.state.cursor.set_char_range(Some(cursors));
    output.state.store(ui.ctx(), output.response.id);
    Shown { total: matches.len(), current: Some(at) }
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
            preview: false,
            close_confirmed: false,
            reveal: None,
        }
    }

    #[test]
    fn the_match_on_a_line_is_counted_from_the_ones_above_it() {
        let text = "one needle\nnothing\nneedle needle\nneedle\n";

        assert_eq!(match_index_on_line(text, "needle", 1), Some(0));
        // The line above holds two of them, so the one below is the fourth.
        assert_eq!(match_index_on_line(text, "needle", 3), Some(1));
        assert_eq!(match_index_on_line(text, "needle", 4), Some(3));
    }

    #[test]
    fn a_line_without_the_query_on_it_has_no_match_to_step_to() {
        let text = "one needle\nnothing\n";

        assert_eq!(match_index_on_line(text, "needle", 2), None);
        assert_eq!(match_index_on_line(text, "needle", 9), None);
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

    /// A query that overlaps itself, over a run of characters it can overlap with, used to
    /// turn up matches that shared a character - "target_env'..." searched for ".." found the
    /// first two dots and then the last two, one byte apart. `marked_text` cuts the text at
    /// each match in turn, so a pair like that put its second cut behind the first and
    /// panicked slicing the text - which is what closed the window the bar was open over.
    #[test]
    fn a_self_overlapping_query_does_not_turn_up_overlapping_matches() {
        let found = matches_in("target_env'...\" >&2", "..");

        assert_eq!(found, vec![11..13]);
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

    /// The marks are cut into the text by byte, while the caret counts characters. A line
    /// with anything but ASCII on it would land the marks somewhere else entirely if the two
    /// were mixed up.
    #[test]
    fn a_mark_is_the_bytes_of_the_text_the_match_covers() {
        let text = "let caf\u{e9} = \"caf\u{e9}\";\n";
        let found = byte_matches_in(text, "caf\u{e9}");

        assert_eq!(found.len(), 2);
        for range in found {
            assert_eq!(&text[range], "caf\u{e9}");
        }
    }

    #[test]
    fn a_file_is_dirty_only_once_it_differs_from_what_was_saved() {
        assert!(!editor_with("fn one() {}", "fn one() {}").is_dirty());
        assert!(editor_with("fn one() {}", "fn two() {}").is_dirty());
        // Nothing has arrived yet, so there is nothing to have changed.
        assert!(!FileEditor::loading("src/lib.rs".to_string()).is_dirty());
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

        assert!(FileEditor::loading("Moontasks.md".to_string()).preview);
        assert!(!FileEditor::loading("src/lib.rs".to_string()).preview);
    }

}
