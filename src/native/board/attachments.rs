//! The documents a task is about, listed in the task's `file_attachments.txt`, and the
//! opening of one.
//!
//! The list is a plain file so that a person or an agent adds a document by writing a line.
//! A click on a text file - a `.txt`, a `.json`, markdown, source, a file with no extension -
//! opens it in a tab the way `moon edit` puts it in one: it is text the window reads and
//! writes itself. Any other document is handed to the machine's own opener - `open` on a Mac -
//! so a PDF opens in the reader it is set to rather than in a pane made for text.
//!
//! The mark at the end of a row takes the document's line out of the file, the way the mark on
//! a file's row takes that file off the card: the document itself stays where it is.

use std::path::{Path, PathBuf};

use egui::{Align, Layout as UiLayout, RichText, Ui};

use crate::{
    moontasks::TaskView,
    native::{
        app::App,
        board::{BoardAction, close_button, gesture::Controls, marks, resources},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// The program that opens a document in whatever the machine has for it.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(not(target_os = "macos"))]
const OPENER: &str = "xdg-open";

/// Where a listed document is: a line naming an absolute path is that path, and any other is
/// from the task's folder.
pub(crate) fn path_of(task: &TaskView, listed: &str) -> PathBuf {
    Path::new(&task.dir_path).join(listed)
}

/// The list, one link a line. Nothing is drawn for a task with no documents. `card` is what a
/// press on a link is claimed from, so on a card a click opens the document rather than
/// picking the card up; the task's pane has no card and passes [`Controls::elsewhere`].
/// `heading` is for the pane, where the list stands on its own under a name. The mark at the
/// end of a row asks, through `actions`, for that document to be taken off the task.
pub(crate) fn draw_list(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
    heading: bool,
) {
    if task.attachments.is_empty() {
        return;
    }
    if heading {
        ui.label(RichText::new("Files").size(SMALL_SIZE).color(palette.muted));
        ui.add_space(3.0);
    }
    for listed in &task.attachments {
        let path = path_of(task, listed);
        // A row like a shell's or a run's: a dot, then the name, the whole row opening it.
        let row = resources::draw_row(ui, palette, true, &format!("Open {}", path.display()));
        let row_pressed = card.pressed(&row);
        let mut name_pressed = false;
        resources::draw_in_row(ui, row.rect, |ui| {
            marks::attachment_dot(ui, palette);
            // The mark takes its place at the end of the row before the name is drawn, because
            // the name is cut to whatever of the row is left: drawn first, a long one would
            // leave the mark no room.
            ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                // Without asking, like the mark on a file's row: nothing is lost by it. The
                // document stays where it is, and listing it again is writing its line again.
                let take_off = close_button(ui, palette)
                    .on_hover_text("Take this file off the task, and leave the file where it is");
                if card.pressed(&take_off) {
                    actions.push(BoardAction::RemoveAttachment {
                        task_id: task.id.clone(),
                        listed: listed.clone(),
                    });
                }
                ui.with_layout(UiLayout::left_to_right(Align::Center), |ui| {
                    // Cut to the row: a card is as wide as its column, whatever the document
                    // is called.
                    let name = widgets::quiet_button_cut_to_row(ui, listed)
                        .on_hover_text(format!("Open {}", path.display()));
                    name_pressed = card.pressed(&name);
                });
            });
        });
        if row_pressed || name_pressed {
            open(app, path);
        }
    }
}

/// Open one listed document: a text file in a tab, anything else with [`OPENER`].
///
/// Text is told from what is in the file rather than from its name, the way a review tells
/// which untracked files it can show, so no list of extensions has to keep up with what gets
/// attached. A line with nothing at its path, or with a folder there, goes to [`OPENER`] with
/// the documents that are not text: a tab on a path nothing is at is a new file, which a
/// mistyped line is not asking for.
///
/// The tab is asked for the way a shell's `moon edit` asks, so a document outside the board's
/// repo - an absolute path to another project - opens in a session on its own project. A
/// browser has no such asks, so there every document goes to [`OPENER`].
fn open(app: &mut App, path: PathBuf) {
    // An error is a path that cannot be read as a file, which is no text for a tab either.
    #[cfg(not(target_arch = "wasm32"))]
    if matches!(crate::git::is_likely_binary_file(&path), Ok(false)) {
        app.asked_files
            .push_back(crate::instances::window::OpenFileAsked {
                path,
                line: None,
                wait: false,
            });
        return;
    }
    if let Err(error) = std::process::Command::new(OPENER).arg(&path).spawn() {
        app.model
            .error(format!("could not open {}: {error}", path.display()));
    }
}
