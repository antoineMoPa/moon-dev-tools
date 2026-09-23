//! Drawing a file tab: its header, the rendered page a markdown file opens on, and the editor
//! with its context menu.

use egui::{Align, Layout, RichText, Ui};
use egui_frames::PaneId;
use egui_moon_code_ide::LspPosition;
use egui_moon_editor::{EditorRequest, Marks};

use crate::native::{
    app::App,
    theme::{Palette, SMALL_SIZE},
    widgets,
};

use super::is_markdown;

/// Between the pane's border and what it is showing.
const PANE_PADDING: i8 = 10;

/// What the header reads on a file that is not in the repo, in place of the save it does not
/// offer.
const OUTSIDE_THE_REPO_NOTE: &str = "outside the repo · read-only";

/// What the header reads on a file another program wrote while the tab had edits of its own.
const WRITTEN_ELSEWHERE_NOTE: &str = "changed on disk";

/// What the header reads on a tab opened on a path nothing is at yet, until a save creates it.
const NEW_FILE_NOTE: &str = "new file · not saved yet";

/// `[reload]` once it has been pressed, asking for the second press that drops the edits.
const RELOAD_CONFIRM_LABEL: &str = "[reload · discard edits]";

impl App {
    pub(crate) fn draw_file_pane(
        &mut self,
        ui: &mut Ui,
        pane_id: PaneId,
        session_id: &str,
        file_path: &str,
        revision: Option<&str>,
    ) {
        let palette = self.palette_of();
        self.ensure_file_editor(pane_id, session_id, file_path, revision);
        // The dated entry a work log tab was opened for, once its text is here to put it in.
        self.add_waiting_work_log_entry(pane_id);
        // What the language server behind this file has been told about it, brought up with
        // what the pane is showing.
        let ctx = ui.ctx().clone();
        self.sync_document(&ctx, pane_id, session_id);
        // A format asked for on an earlier frame, once the server has heard the text.
        crate::native::formatting::follow(self, &ctx, pane_id, session_id);
        // What its server has found wrong with it, asked for now and then.
        crate::native::diagnostics::follow(self, &ctx, pane_id, session_id);
        // A name ⌘-clicked in this pane on an earlier frame, once it has been looked up.
        crate::native::definition::follow(self, pane_id, session_id);
        // Who last touched each stretch of the text, kept up with the text while it is shown.
        crate::native::blame::follow(self, &ctx, pane_id, session_id);
        self.check_the_disk(pane_id, session_id);
        let Some(editor) = self.model.file_editors.get(&pane_id) else {
            return;
        };
        let dirty = editor.is_dirty();
        let new_file = !editor.on_disk;
        let saving = editor.saving;
        let written_elsewhere = editor.written_elsewhere.is_some();
        let reload_confirmed = editor.reload_confirmed;
        let outside_the_repo = editor.outside_the_repo;
        let read_only = editor.is_read_only();
        let problems = crate::native::diagnostics::said_in_header(editor.diagnosed.found());
        let error = editor.error.clone();
        let loaded = editor.saved.is_some();
        let blaming = editor.blaming.is_on();
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
                        if (dirty || new_file)
                            && !saving
                            && !read_only
                            && widgets::quiet_button(ui, "[save]").clicked()
                        {
                            self.save_file_pane(pane_id, session_id);
                        }
                        // Left of `[save]`, the other way out of a file written under the
                        // edits in the pane.
                        if written_elsewhere
                            && !saving
                            && widgets::quiet_button(
                                ui,
                                if reload_confirmed {
                                    RELOAD_CONFIRM_LABEL
                                } else {
                                    "[reload]"
                                },
                            )
                            .on_hover_text(if reload_confirmed {
                                "Press again to discard your edits and show the file as it is on disk"
                            } else {
                                "Show the file as it is on disk, discarding your edits"
                            })
                            .clicked()
                        {
                            self.reload_file_pane(pane_id);
                        }
                        // Who last touched each stretch of the text, beside the lines. Only
                        // a file of the repo has a history here to ask about, and only the
                        // text has lines to put it beside.
                        if loaded
                            && !outside_the_repo
                            && !previewing
                            && widgets::quiet_button(
                                ui,
                                if blaming { "[hide blame]" } else { "[blame]" },
                            )
                            .on_hover_text(if blaming {
                                "Take the column of who last touched each stretch down"
                            } else {
                                "Show who last touched each stretch of the file, and in which commit, beside the lines"
                            })
                            .clicked()
                        {
                            crate::native::blame::toggle(self, pane_id);
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
                        // What the pane is, said where the save would otherwise be: a jump
                        // into a dependency opens a file this window has no business writing,
                        // and a pane that looked like every other file tab but silently
                        // refused to save would be worse than one that says what it is.
                        if outside_the_repo {
                            ui.label(
                                RichText::new(OUTSIDE_THE_REPO_NOTE)
                                    .size(SMALL_SIZE - 1.0)
                                    .color(palette.muted),
                            )
                            .on_hover_text(
                                "This file is not in the repository. It opened because a language server named it as where the definition is, and it can only be read.",
                            );
                        }
                        // The same for a tab on an old version of a file: which one, and
                        // that it is there to be read.
                        if let Some(revision) = revision {
                            ui.label(
                                RichText::new(format!(
                                    "as of {} · read-only",
                                    crate::native::panes::short_revision(revision)
                                ))
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                            )
                            .on_hover_text(format!(
                                "The file as it was in commit {revision}, opened from a blame. It can only be read; the blame beside it is of this version."
                            ));
                        }
                        // What the server found wrong, where the eye goes after the file's name.
                        if let Some((said, worst)) = &problems {
                            ui.label(RichText::new(said).size(SMALL_SIZE - 1.0).color(
                                crate::native::diagnostics::colour_of(*worst, &palette),
                            ))
                            .on_hover_text("What the language server found wrong with this file. Point at an underline to read it.");
                        }
                        if written_elsewhere && !saving {
                            ui.label(
                                RichText::new(WRITTEN_ELSEWHERE_NOTE)
                                    .size(SMALL_SIZE - 1.0)
                                    .color(palette.warn),
                            )
                            .on_hover_text(
                                "Another program wrote this file after it was opened here, while it had unsaved edits. [reload] shows its version and discards yours; [save] writes yours over it.",
                            );
                        } else if new_file {
                            ui.label(
                                RichText::new(if saving { "creating…" } else { NEW_FILE_NOTE })
                                    .size(SMALL_SIZE - 1.0)
                                    .color(palette.warn),
                            )
                            .on_hover_text(
                                "Nothing is at this path yet. Saving creates the file; closing the tab without saving leaves nothing behind.",
                            );
                        } else if dirty && !read_only {
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
    // Read out before the editor is borrowed, like the find bar above it.
    let indent = app.model.project.indent();

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

    let underlines = crate::native::diagnostics::underlines(
        editor.code.text(),
        editor.diagnosed.found(),
        palette,
    );
    // Who last touched each stretch of the text, when the tab has asked - see
    // `crate::native::blame`. The editor draws the column and says which stretch the pointer
    // is on; what the stretch is about stays here.
    let notes = editor.blaming.notes(palette).to_vec();
    let in_the_repo = !editor.outside_the_repo;
    let blame_shown = editor.blaming.is_on();
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
            // What a Tab press puts in, as the repo's `.moonreview.json` has it - four spaces
            // until it says otherwise. A fact about the repo, so it is read from the repo.
            indent,
            // What the server behind the file found wrong with it - see
            // `crate::native::diagnostics`.
            underlines: &underlines,
            notes: &notes,
        },
    );
    // The stretch the pointer rests on, told in full, and the one clicked - what a click
    // asks for depends on where in the stretch it landed, which is `blame`'s to say.
    if let Some(pointed) = &output.pointed_note
        && let Some(chunk) = editor.blaming.chunk(pointed.note)
    {
        crate::native::blame::tell_on_hover(&pointed.response, palette, chunk);
    }
    let stretch_clicked = output
        .note_clicked
        .and_then(|click| Some((editor.blaming.chunk(click.note)?.clone(), click)));
    // The name that was ⌘-clicked, if one was, and where in the text it sits - which is
    // already the position a language server is asked about. Where it is defined is
    // `crate::native::definition`'s business rather than this pane's.
    let navigated_to = output.navigated_to.clone();
    editor.caret = output.caret.as_ref().map(|caret| LspPosition {
        line: caret.line,
        column: caret.column,
    });
    let edits = editor.offers_edits();
    editor.caret_word = output.word_at_caret.clone();
    let finds_places = editor.offers_places();

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
    if let Some((chunk, click)) = stretch_clicked {
        crate::native::blame::follow_click(app, session_id, &chunk, click);
    }
    // What the caret is on now, and what became of the list that was up: whether that is
    // worth a question is `completing`'s to answer.
    crate::native::completing::follow_the_caret(app, pane_id, session_id, ui.ctx(), &output);
    // The word the pointer rests on, and what its server says about it and about what is
    // wrong under the pointer.
    crate::native::hover::follow_the_pointer(app, ui.ctx(), pane_id, session_id, &output);
    if let Some(told) = crate::native::hover::told(app, pane_id, &output, palette) {
        crate::native::hover::draw_tooltip(app, &output.response, told);
    }
    // The signature of the call being typed, above the caret.
    crate::native::signature::follow_the_caret(app, ui.ctx(), pane_id, session_id, &output);
    crate::native::signature::draw(app, ui.ctx(), pane_id, &output, palette);

    // A right-click has already put the caret on the name it was made on - the editor does
    // that - so what the menu offers is about that name.
    use crate::native::bindings::Action;
    let mut places_asked = None;
    let mut rename_asked = false;
    let mut format_asked = false;
    let mut actions_asked = false;
    let mut blame_asked = false;
    egui::Popup::context_menu(&output.response).show(|ui| {
        for kind in crate::native::places::KINDS {
            if menu_item(
                ui,
                finds_places,
                kind.command,
                Action::FindPlaces(kind.which),
            )
            .clicked()
            {
                places_asked = Some(kind.which);
            }
        }
        ui.separator();
        actions_asked = menu_item(ui, edits, "code actions", Action::CodeActions).clicked();
        rename_asked = menu_item(ui, edits, "rename symbol", Action::RenameSymbol).clicked();
        format_asked = menu_item(ui, edits, "format file", Action::FormatFile).clicked();
        // A file outside the repo has no history here, so the item is not there rather
        // than greyed out with a note about language servers it is nothing to do with.
        if in_the_repo {
            ui.separator();
            blame_asked = menu_item(
                ui,
                true,
                if blame_shown {
                    "hide blame"
                } else {
                    "show blame"
                },
                Action::ToggleBlame,
            )
            .clicked();
        }
    });
    if blame_asked {
        crate::native::blame::toggle(app, pane_id);
    }
    if format_asked {
        crate::native::formatting::start(app, pane_id);
    }
    if actions_asked {
        crate::native::code_actions::start(app, pane_id, session_id);
    }
    if let Some(which) = places_asked {
        crate::native::places::ask(app, pane_id, session_id, which);
    }
    if rename_asked {
        crate::native::renaming::start(app, pane_id, session_id);
    }
}

/// One item of a file tab's context menu, with the key that does the same thing beside it.
fn menu_item(
    ui: &mut Ui,
    enabled: bool,
    title: &str,
    action: crate::native::bindings::Action,
) -> egui::Response {
    let shortcut = crate::native::bindings::chord_of(action)
        .map(crate::native::bindings::describe)
        .unwrap_or_default();
    ui.add_enabled(enabled, egui::Button::new(title).shortcut_text(shortcut))
        .on_disabled_hover_text("No language server serves this file")
}

/// What the find bar is asking of a file pane this frame.
struct Searching {
    query: String,
    at: usize,
    pending: bool,
}
