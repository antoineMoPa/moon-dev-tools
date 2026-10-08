//! Drawing the picker: what it asks, its line, the rows of the folder the line names - and
//! reading the keys that move along them.

use egui::{Align2, Key, Modifiers, RichText, vec2};

use crate::native::{
    app::App,
    palette::{
        Kept, cut_to_width, floating_frame, floating_width, paint_highlight, pressed_outside,
    },
    theme::{Palette, SMALL_SIZE, UI_SIZE},
    widgets,
};

use super::{
    FilePicker, Listing,
    rows::{self, Row, RowKind},
};

/// The height of one row of the list. A row is a name, so it is a line tall where the
/// palette's are two.
const ROW_HEIGHT: f32 = 24.0;

/// The gap between a row's edge and what is written in it.
const ROW_PADDING: f32 = 9.0;

/// How much of the window's height the rows may take before they scroll - see the palette's.
const ROWS_HEIGHT_OF_SCREEN: f32 = 0.5;

/// How many rows are drawn at most. A folder of thousands - a `node_modules`, `/usr/lib` - is
/// narrowed by typing rather than read down, and the rows past this are counted under the
/// list instead of laid out every frame.
const ROWS_DRAWN: usize = 300;

/// What the picker says under its rows, since the line says none of it itself.
const KEYS_NOTE: &str = "enter opens or picks · backspace goes up · esc cancels";

/// The line's own id, so its caret can be read before it is drawn and placed after.
const LINE_ID: &str = "moonreview-file-picker-line";

/// The rows under the line as it stands. None until its folder has been listed.
pub(super) fn rows_of(picker: &FilePicker) -> Vec<Row> {
    let Listing::Read(listing) = &picker.listing else {
        return Vec::new();
    };
    let (_, typed) = rows::split(&picker.line).expect("a listed line names a folder");
    rows::rows_for(&picker.purpose, listing, typed)
}

pub(crate) fn draw(app: &mut App, ctx: &egui::Context) {
    let Some(picker) = &app.model.file_picker else {
        return;
    };
    // The palette opened over it is the box that was asked for last, and a press anywhere
    // else belongs to whatever was pressed - see the palette's own.
    if app.model.palette.open || pressed_outside(ctx, picker.rect) {
        app.model.file_picker = None;
        return;
    }
    app.list_the_folder_on_the_line();
    let palette = app.palette_of();
    let picker = app
        .model
        .file_picker
        .as_mut()
        .expect("the picker is up: checked above");

    let line_id = egui::Id::new(LINE_ID);
    // Backspace with the caret after a `/` takes the whole folder off the line. With anything
    // else before the caret, or some of the line selected, it is the line's own and deletes
    // what Backspace deletes. Read before the keys are: the caret is the context's to say,
    // and the context is not to be asked from inside a read of its input.
    let backspace_goes_up =
        caret_is_after_a_folder(ctx, line_id, &picker.line) && way_up(picker).is_some();
    // The keys that are the list's are taken before the line sees them: an arrow would move
    // its caret, and Enter would take the keyboard off it.
    let (cancel, move_down, move_up, accept, goes_up) = ctx.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::Escape),
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::Enter),
            backspace_goes_up && input.consume_key(Modifiers::NONE, Key::Backspace),
        )
    });
    if cancel {
        app.model.file_picker = None;
        return;
    }
    if goes_up {
        let up = way_up(picker).expect("checked as the key was taken");
        picker.go_to(up);
        ctx.request_repaint();
    }

    let rows = rows_of(picker);
    let relisted = picker.highlighted_under.as_deref() != Some(picker.line.as_str());
    if relisted {
        picker.highlighted = rows::first_choice(&rows);
        picker.highlighted_under = Some(picker.line.clone());
    }
    if !rows.is_empty() {
        let last = rows.len() - 1;
        if move_down {
            picker.highlighted = (picker.highlighted + 1).min(last);
        }
        if move_up {
            picker.highlighted = picker.highlighted.saturating_sub(1);
        }
        picker.highlighted = picker.highlighted.min(last);
    }
    let mut chosen = (accept && !rows.is_empty()).then_some(picker.highlighted);

    let screen = ctx.viewport_rect();
    let area = egui::Area::new("moonreview-file-picker".into())
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, vec2(0.0, screen.height() * 0.12))
        .show(ctx, |ui| {
            floating_frame(&palette).show(ui, |ui| {
                ui.set_width(floating_width(screen));
                ui.label(
                    RichText::new(picker.purpose.asks())
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                let entry = ui.add(
                    egui::TextEdit::singleline(&mut picker.line)
                        .id(line_id)
                        .hint_text("/a/folder/, or ~/")
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(7, 5)),
                );
                entry.request_focus();
                if std::mem::take(&mut picker.places_caret) {
                    place_caret(ui.ctx(), line_id, &picker.line);
                }

                ui.add_space(6.0);
                if rows.is_empty() {
                    let (said, ink) = said_of_no_rows(&picker.listing, &palette);
                    ui.label(RichText::new(said).color(ink));
                } else {
                    // Sized and scrolled the way the palette's rows are - see there.
                    let drawn = rows.len().min(ROWS_DRAWN);
                    let keep_highlight_in_view = move_down || move_up || relisted;
                    let rows_height = (drawn as f32 * ROW_HEIGHT
                        + drawn.saturating_sub(1) as f32 * ui.spacing().item_spacing.y)
                        .min(screen.height() * ROWS_HEIGHT_OF_SCREEN);
                    let folder = listed_folder(picker).to_string();
                    egui::ScrollArea::vertical()
                        .max_height(rows_height)
                        .min_scrolled_height(rows_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (index, row) in rows.iter().take(ROWS_DRAWN).enumerate() {
                                let highlighted = index == picker.highlighted;
                                let drawn = draw_row(ui, row, &folder, highlighted, &palette);
                                if highlighted && keep_highlight_in_view {
                                    drawn.scroll_to_me(None);
                                }
                                if drawn.clicked() {
                                    chosen = Some(index);
                                }
                                if drawn.hovered() {
                                    picker.highlighted = index;
                                }
                            }
                        });
                }
                ui.label(
                    RichText::new(footnote(rows.len()))
                        .size(SMALL_SIZE - 1.0)
                        .color(palette.muted),
                );
            });
        });
    picker.rect = Some(area.response.rect);

    if let Some(row) = chosen.and_then(|index| rows.into_iter().nth(index)) {
        act_on(app, row);
        ctx.request_repaint();
    }
}

/// Do what a row is for: go where it leads, or put the picker away and hand on its pick.
fn act_on(app: &mut App, row: Row) {
    let picker = app
        .model
        .file_picker
        .as_mut()
        .expect("a row is only acted on while the picker is up");
    let folder = listed_folder(picker).to_string();
    let pick = match row.kind {
        RowKind::Up => {
            let up =
                rows::parent_line(&folder).expect("the way up is only listed where there is one");
            picker.go_to(up);
            return;
        }
        RowKind::Folder => {
            picker.go_to(rows::folder_line(&rows::join(&folder, &row.name)));
            return;
        }
        RowKind::ThisFolder => super::Pick {
            path: folder,
            is_there: true,
        },
        RowKind::File => super::Pick {
            path: rows::join(&folder, &row.name),
            is_there: true,
        },
        RowKind::NewName { replaces } => super::Pick {
            path: rows::join(&folder, &row.name),
            is_there: replaces,
        },
    };
    let picker = app.model.file_picker.take().expect("just borrowed");
    super::picked(app, picker.purpose, pick);
}

/// The folder the rows are of, by its resolved path.
fn listed_folder(picker: &FilePicker) -> &str {
    let Listing::Read(listing) = &picker.listing else {
        unreachable!("there are rows only once their folder has been listed");
    };
    &listing.folder
}

/// The line to go up to from where the picker is, when its folder is listed and is not the
/// root. An unlisted folder has no resolved path to say what is above it.
fn way_up(picker: &FilePicker) -> Option<String> {
    let Listing::Read(listing) = &picker.listing else {
        return None;
    };
    rows::parent_line(&listing.folder)
}

/// Whether the line ends in a folder, and its caret is sitting after it with nothing selected.
fn caret_is_after_a_folder(ctx: &egui::Context, line_id: egui::Id, line: &str) -> bool {
    if !rows::split(line).is_some_and(|(_, typed)| typed.is_empty()) {
        return false;
    }
    let Some(caret) =
        egui::TextEdit::load_state(ctx, line_id).and_then(|state| state.cursor.char_range())
    else {
        return false;
    };
    caret.is_empty() && caret.primary.index == egui::text::CharIndex(line.chars().count())
}

/// Put the caret at the end of the line, with what follows its last `/` selected: nothing,
/// after going into a folder, and the name offered for a new file, to be typed over.
fn place_caret(ctx: &egui::Context, line_id: egui::Id, line: &str) {
    let Some(mut state) = egui::TextEdit::load_state(ctx, line_id) else {
        return;
    };
    let (folder, _) = rows::split(line).expect("the picker only writes lines that name a folder");
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(folder.chars().count()),
            egui::text::CCursor::new(line.chars().count()),
        )));
    state.store(ctx, line_id);
}

/// What is said where the rows would be, and in which ink: a folder that could not be listed
/// is the one that reads as something gone wrong.
fn said_of_no_rows(listing: &Listing, palette: &Palette) -> (String, egui::Color32) {
    match listing {
        Listing::NoFolder => ("A path starts at / or at ~/".to_string(), palette.muted),
        Listing::Waiting => ("Listing…".to_string(), palette.muted),
        Listing::Read(_) => ("Nothing here by that name".to_string(), palette.muted),
        Listing::Failed(why) => (why.clone(), palette.warn),
    }
}

/// The keys, and how many rows were left undrawn when there were too many to draw.
fn footnote(rows: usize) -> String {
    match rows.checked_sub(ROWS_DRAWN) {
        Some(undrawn) if undrawn > 0 => {
            format!("{undrawn} more - type to narrow · {KEYS_NOTE}")
        }
        _ => KEYS_NOTE.to_string(),
    }
}

/// What a row is written in: a dotfile dimmed, and a name that would replace a file in the
/// ink a warning is.
fn ink_of(row: &Row, palette: &Palette) -> egui::Color32 {
    match row.kind {
        RowKind::NewName { replaces: true } => palette.warn,
        _ if row.hidden => palette.muted,
        _ => palette.ink,
    }
}

/// One row: what it reads as, and against its right edge - on a row that is about the folder
/// rather than something in it - the folder it means, read from its end as a path is.
fn draw_row(
    ui: &mut egui::Ui,
    row: &Row,
    folder: &str,
    highlighted: bool,
    palette: &Palette,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), egui::Sense::click());
    let response = widgets::clickable(response);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    if highlighted {
        paint_highlight(ui, rect, palette);
    }

    let mut text_width = rect.width() - 2.0 * ROW_PADDING;
    let means = match row.kind {
        RowKind::Up => rows::parent_line(folder),
        RowKind::ThisFolder | RowKind::NewName { .. } => Some(rows::folder_line(folder)),
        RowKind::Folder | RowKind::File => None,
    };
    if let Some(means) = means {
        let note = cut_to_width(
            ui.painter(),
            &means,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            palette.muted,
            text_width * 0.6,
            Kept::End,
        );
        text_width -= note.size().x + ROW_PADDING;
        let at = egui::pos2(
            rect.max.x - ROW_PADDING - note.size().x,
            rect.center().y - note.size().y / 2.0,
        );
        ui.painter().galley(at, note, palette.muted);
    }

    let ink = ink_of(row, palette);
    let title = cut_to_width(
        ui.painter(),
        &rows::title_of(row),
        egui::FontId::proportional(UI_SIZE),
        ink,
        text_width,
        Kept::Start,
    );
    let at = egui::pos2(
        rect.min.x + ROW_PADDING,
        rect.center().y - title.size().y / 2.0,
    );
    ui.painter().galley(at, title, ink);
    response
}
