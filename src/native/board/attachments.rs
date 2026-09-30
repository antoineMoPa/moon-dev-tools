//! The documents a task is about, listed in the task's `file_attachments.txt`, and the
//! opening of one.
//!
//! The list is a plain file so that a person or an agent adds a document by writing a line.
//! A click hands the document to the machine's own opener - `open` on a Mac - so a PDF
//! opens in the reader it is set to rather than in a pane made for text. A markdown file is
//! the exception: it is text the window reads and writes itself, so it opens in a tab the way
//! `moon edit` puts it in one.

use std::path::{Path, PathBuf};

use egui::{RichText, Ui};

use crate::{
    moontasks::TaskView,
    native::{
        app::App,
        board::{gesture::Controls, marks, resources},
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
/// `heading` is for the pane, where the list stands on its own under a name.
pub(crate) fn draw_list(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
    palette: &Palette,
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
            let name = widgets::quiet_button(ui, listed).on_hover_text(format!("Open {}", path.display()));
            name_pressed = card.pressed(&name);
        });
        if row_pressed || name_pressed {
            open(app, path);
        }
    }
}

/// Open one listed document: a markdown file in a tab, anything else with [`OPENER`].
///
/// The tab is asked for the way a shell's `moon edit` asks, so a document outside the board's
/// repo - an absolute path to another project - opens in a session on its own project. A
/// browser has no such asks, so there every document goes to [`OPENER`].
fn open(app: &mut App, path: PathBuf) {
    #[cfg(not(target_arch = "wasm32"))]
    if crate::native::file_pane::is_markdown(&path.display().to_string()) {
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
