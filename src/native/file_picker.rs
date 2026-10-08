//! File picker - the window's own dialog for picking a file or a folder: a line holding a path,
//! over a list of what is in the folder the path names. Drawn like the palette, and driven
//! from the keyboard the same way.
//!
//! What it lists is read through the [`Backend`](crate::backend::Backend), so the disk it
//! browses is the one the repos are on - the server's - which is the disk a pick has to be a
//! path of. Every window picks with it, and none with the OS's dialog: that one browses the
//! disk of the machine the window is on, which a `--remote` window's repos are not on, is not
//! there at all in a browser or in `moon desktop`, and is another dialog on each system.
//!
//! It is drawn frame after frame rather than blocking until it has a path, so a pick is asked
//! for with what it is for - a [`PickPurpose`] - and when it is made the path goes to what
//! that purpose names: see [`picked`].
//!
//! The keys: typing narrows the list to the names holding what was typed, and a `/` typed
//! goes into the folder named before it - so a whole path can be typed or pasted, from `/` or
//! from `~/`. The arrows move along the list. Enter goes into the folder on the row, or picks
//! what is on it. Backspace after a `/` goes up a folder, as the `..` row does. Esc, or a
//! press anywhere else, puts the picker away with nothing picked.

mod drawing;
mod rows;

pub(crate) use drawing::draw;

use crate::{api::folders::FolderListing, native::app::App};

use rows::Picks;

/// What `~` on the line stands for is the server's to say: the home of whoever it runs as.
/// Where a picker opens when nothing says where better.
pub(crate) const HOME: &str = "~";

/// What a pick is for, which is where the path goes once it is made - see [`picked`] - and
/// what the picker lists and offers meanwhile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickPurpose {
    /// The folder a window opens on, asked for by its launch screen.
    RepoFolder,
    // The two below are the file tabs' own, which a browser's window does not open by picking
    // yet: its File menu has neither item.
    /// A file to read and edit in a tab.
    #[cfg(not(target_arch = "wasm32"))]
    FileToEdit,
    /// Where the file of an untitled tab goes and what it is called, asked for by the tab's
    /// first save.
    #[cfg(not(target_arch = "wasm32"))]
    NameForUntitledFile {
        pane_id: egui_frames::PaneId,
        session_id: String,
        /// The name the tab has so far, offered on the line to be typed over.
        suggested: String,
    },
}

impl PickPurpose {
    fn picks(&self) -> Picks {
        match self {
            Self::RepoFolder => Picks::Folder,
            #[cfg(not(target_arch = "wasm32"))]
            Self::FileToEdit => Picks::File,
            #[cfg(not(target_arch = "wasm32"))]
            Self::NameForUntitledFile { .. } => Picks::NewName,
        }
    }

    /// The name a new file is offered under, for the one purpose that names one.
    fn suggested_name(&self) -> Option<&str> {
        match self {
            Self::RepoFolder => None,
            #[cfg(not(target_arch = "wasm32"))]
            Self::FileToEdit => None,
            #[cfg(not(target_arch = "wasm32"))]
            Self::NameForUntitledFile { suggested, .. } => Some(suggested),
        }
    }

    /// What the picker says it is asking, over its line.
    fn asks(&self) -> &'static str {
        match self {
            Self::RepoFolder => "Choose a folder to open",
            #[cfg(not(target_arch = "wasm32"))]
            Self::FileToEdit => "Open a file to edit",
            #[cfg(not(target_arch = "wasm32"))]
            Self::NameForUntitledFile { .. } => "Save the new file",
        }
    }
}

/// What the picker has of the folder its line names.
enum Listing {
    /// The line names no folder: it has no `/` in it yet.
    NoFolder,
    /// Asked for, and not answered yet.
    Waiting,
    Read(FolderListing),
    /// The folder could not be listed - it is not there, or not this account's to read - and
    /// why.
    Failed(String),
}

/// The picker while it is up.
pub(crate) struct FilePicker {
    purpose: PickPurpose,
    /// The path typed so far - see [`rows`] for how it is read.
    line: String,
    /// The folder `listing` is of, as the line named it. `None` before the first frame has
    /// asked, and when the line names none.
    asked: Option<String>,
    listing: Listing,
    highlighted: usize,
    /// The line the highlight was put under. Typing, or a listing arriving, changes which
    /// rows there are, and a highlight from before that means nothing.
    highlighted_under: Option<String>,
    /// Set when the line was written by the picker rather than typed: the caret belongs at
    /// its end, with the name offered for a new file selected to be typed over.
    places_caret: bool,
    /// Where it drew last frame, which a press outside of puts it away - see
    /// [`crate::native::model::PaletteState::rect`].
    rect: Option<egui::Rect>,
}

impl FilePicker {
    fn opened(purpose: PickPurpose, folder: &str) -> Self {
        let mut picker = Self {
            purpose,
            line: String::new(),
            asked: None,
            listing: Listing::NoFolder,
            highlighted: 0,
            highlighted_under: None,
            places_caret: false,
            rect: None,
        };
        picker.go_to(rows::folder_line(folder));
        picker
    }

    /// Put a folder on the line, to be listed by the next frame.
    fn go_to(&mut self, folder_line: String) {
        self.line = format!(
            "{folder_line}{}",
            self.purpose.suggested_name().unwrap_or_default()
        );
        self.places_caret = true;
    }

    /// The rows it is listing, as they read.
    #[cfg(test)]
    pub(crate) fn rows_for_test(&self) -> Vec<String> {
        drawing::rows_of(self).iter().map(rows::title_of).collect()
    }
}

impl App {
    /// Put the window's own picker up for a pick of this purpose, opened on `folder` - a
    /// folder of the server's disk named from `/`, or [`HOME`]. Nothing comes back from here:
    /// the pick, when one is made, goes to [`picked`].
    pub(crate) fn ask_for_a_pick(&mut self, purpose: PickPurpose, folder: &str) {
        // One box over the window at a time, and this is the one just asked for.
        self.model.palette.dismiss();
        self.model.file_picker = Some(FilePicker::opened(purpose, folder));
    }

    /// Ask for the folder the line names, when it is not the one already listed or asked for.
    fn list_the_folder_on_the_line(&mut self) {
        let Some(picker) = &mut self.model.file_picker else {
            return;
        };
        let folder = rows::split(&picker.line).map(|(folder, _)| folder.to_string());
        if picker.asked == folder {
            return;
        }
        picker.asked = folder.clone();
        picker.highlighted_under = None;
        let Some(folder) = folder else {
            picker.listing = Listing::NoFolder;
            return;
        };
        picker.listing = Listing::Waiting;

        let asked = folder.clone();
        self.tasks.spawn(
            move |backend| backend.list_folder(&folder),
            move |model, result| {
                let Some(picker) = &mut model.file_picker else {
                    return;
                };
                // The line has gone on to another folder since, whose own listing is coming.
                if picker.asked.as_deref() != Some(asked.as_str()) {
                    return;
                }
                picker.listing = match result {
                    Ok(listing) => Listing::Read(listing),
                    Err(error) => Listing::Failed(format!("{error}")),
                };
                picker.highlighted_under = None;
            },
        );
    }
}

/// What was picked.
struct Pick {
    /// Absolute and resolved on the server's disk: the folder the server said it listed, and
    /// for a file a name in it.
    path: String,
    /// Whether something is at the path. Only a name typed for a new file can be of nothing.
    is_there: bool,
}

/// Hand a pick to what it was asked for.
///
/// Each purpose's own follow-up is where the rest of that purpose's logic is: the launch
/// screen's and File › Open's in [`crate::native::app`], the untitled tab's beside its save.
fn picked(app: &mut App, purpose: PickPurpose, pick: Pick) {
    match purpose {
        PickPurpose::RepoFolder => app.open_picked_folder(pick.path),
        #[cfg(not(target_arch = "wasm32"))]
        PickPurpose::FileToEdit => app.open_picked_file(std::path::PathBuf::from(pick.path)),
        #[cfg(not(target_arch = "wasm32"))]
        PickPurpose::NameForUntitledFile {
            pane_id,
            session_id,
            ..
        } => app.save_untitled_file_as(
            pane_id,
            &session_id,
            std::path::Path::new(&pick.path),
            pick.is_there,
        ),
    }
}
