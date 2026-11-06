//! The documents a task is about, listed in the task's `file_attachments.txt`, and the
//! opening of one. The task's pane lists the files left in the task's folder beside them, and
//! has a button that opens the window's own file picker in the folder - not the OS's browser.
//!
//! The list is a plain file so that a person or an agent adds a document by writing a line.
//! A click on a text file - a `.txt`, a `.json`, markdown, source, a file with no extension -
//! opens it in a tab the way `moon edit` puts it in one: it is text the window reads and
//! writes itself. Any other document is handed to the machine's own opener - `open` on a Mac -
//! so a PDF opens in the reader it is set to rather than in a pane made for text. A web page is
//! text too, but it is attached to be looked at, so it goes to the opener and the browser.
//!
//! The mark at the end of a row takes the document's line out of the file, the way the mark on
//! a file's row takes that file off the card: the document itself stays where it is.

use std::path::{Path, PathBuf};

use egui::{Align, Layout as UiLayout, RichText, Ui};

use crate::native::file_picker::PickPurpose;

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

/// The width of the dot [`marks::attachment_dot`] draws, which a folder's file leaves room for.
const DOT_WIDTH: f32 = 7.0;

/// The extensions of text documents that go to [`OPENER`] all the same: a page is attached to
/// be seen in a browser, not read as markup.
#[cfg(not(target_arch = "wasm32"))]
const EXTENSIONS_FOR_OPENER: &[&str] = &["html", "htm"];

/// Where a listed document is: a line naming an absolute path is that path, and any other is
/// from the task's folder.
pub(crate) fn path_of(task: &TaskView, listed: &str) -> PathBuf {
    Path::new(&task.dir_path).join(listed)
}

/// The Files section of a task's pane: a heading with the button that opens the task's folder,
/// then the documents the task lists, then the files left in the folder. The pane is the only
/// place the folder's own files are listed - a card lists the documents, and no more.
pub(crate) fn draw_section(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Files").size(SMALL_SIZE).color(palette.muted));
        open_folder_button(app, ui, task);
    });
    ui.add_space(3.0);
    draw_list(app, ui, task, &mut Controls::elsewhere(), palette, actions);
    draw_folder_files(app, ui, task, palette);
}

/// The button at the right of the Files heading, which opens the window's own file picker in
/// the task's folder - the same folder browser as File > Open, so a file is picked from the
/// list and opened the way the list opens one. A browser has no folder of the machine to
/// browse, so there is no button.
#[cfg(not(target_arch = "wasm32"))]
fn open_folder_button(app: &mut App, ui: &mut Ui, task: &TaskView) {
    ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
        let open = widgets::clickable(ui.small_button("open folder"))
            .on_hover_text(format!("Browse {}", task.dir_path));
        if open.clicked() {
            app.ask_for_a_pick(PickPurpose::TaskFile, &task.dir_path);
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn open_folder_button(_app: &mut App, _ui: &mut Ui, _task: &TaskView) {}

/// The list, one link a line. Nothing is drawn for a task with no documents. `card` is what a
/// press on a link is claimed from, so on a card a click opens the document rather than
/// picking the card up; the task's pane has no card and passes [`Controls::elsewhere`]. The
/// mark at the end of a row asks, through `actions`, for that document to be taken off the
/// task.
pub(crate) fn draw_list(
    app: &mut App,
    ui: &mut Ui,
    task: &TaskView,
    card: &mut Controls,
    palette: &Palette,
    actions: &mut Vec<BoardAction>,
) {
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

/// The files in the task's folder that the list above does not already show, a row each. A row
/// opens its file the way an attachment's does. None of them is taken off the task with a mark,
/// since none is on it: a file stays listed for as long as it is in the folder.
fn draw_folder_files(app: &mut App, ui: &mut Ui, task: &TaskView, palette: &Palette) {
    for name in &task.folder_files {
        let path = path_of(task, name);
        let row = resources::draw_row(ui, palette, true, &format!("Open {}", path.display()));
        let mut name_pressed = false;
        resources::draw_in_row(ui, row.rect, |ui| {
            // The place the dot of an attachment's row takes, so the names line up.
            ui.add_space(DOT_WIDTH + ui.spacing().item_spacing.x);
            let button = widgets::quiet_button_cut_to_row(ui, name)
                .on_hover_text(format!("Open {}", path.display()));
            name_pressed = button.clicked();
        });
        if row.clicked() || name_pressed {
            open(app, path);
        }
    }
}

/// Hand `path` to [`OPENER`], which opens it in whatever the machine has for it.
fn hand_to_opener(app: &mut App, path: PathBuf) {
    if let Err(error) = std::process::Command::new(OPENER).arg(&path).spawn() {
        app.model
            .error(format!("could not open {}: {error}", path.display()));
    }
}

/// Open one listed document: a text file in a tab, anything else - and a web page - with
/// [`OPENER`].
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
pub(crate) fn open(app: &mut App, path: PathBuf) {
    // An error is a path that cannot be read as a file, which is no text for a tab either.
    #[cfg(not(target_arch = "wasm32"))]
    if !goes_to_opener_by_name(&path)
        && matches!(crate::git::is_likely_binary_file(&path), Ok(false))
    {
        app.asked_files
            .push_back(crate::instances::window::OpenFileAsked {
                path,
                line: None,
                wait: false,
            });
        return;
    }
    hand_to_opener(app, path);
}

/// Whether `path` is named as one of [`EXTENSIONS_FOR_OPENER`].
#[cfg(not(target_arch = "wasm32"))]
fn goes_to_opener_by_name(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| EXTENSIONS_FOR_OPENER.contains(&extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_page_goes_to_the_opener_and_markdown_does_not() {
        assert!(goes_to_opener_by_name(Path::new("report.html")));
        assert!(goes_to_opener_by_name(Path::new("/tmp/page.htm")));
        assert!(!goes_to_opener_by_name(Path::new("notes.md")));
        assert!(!goes_to_opener_by_name(Path::new("Makefile")));
    }
}
