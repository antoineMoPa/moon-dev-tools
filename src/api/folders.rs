//! Folders - the wire types of a folder of the server's disk as the window's own file picker
//! lists it, and of a file anywhere on that disk once it has a session to be read through.
//!
//! Every path here is one of the server's disk, written the way that machine writes it - from
//! `/`, with `/` between its folders. The window never reads any of them against its own disk.

use serde::{Deserialize, Serialize};

/// What is in one folder of the server's disk - see `crate::native::file_picker`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderListing {
    /// The folder that was listed: absolute, with `~`, `..` and every symlink already
    /// followed. A pick is this and the name of an entry, so what a picker hands on is a path
    /// the server resolved rather than one somebody typed.
    pub(crate) folder: String,
    /// In the order the disk gave them, which is none: the picker sorts.
    pub(crate) entries: Vec<FolderEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderEntry {
    pub(crate) name: String,
    pub(crate) kind: EntryKind,
}

/// What an entry of a folder is, as far as picking goes: something to go into, or something
/// to pick. A symlink is whichever of the two it leads to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryKind {
    Folder,
    File,
}

/// The file to place - see [`FilePlaced`]. An absolute path of the server's disk.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
pub(crate) struct PlaceFileRequest {
    pub(crate) path: String,
}

/// Where a file anywhere on the server's disk is read and written from.
///
/// A tab names its file by a session and a path inside the folder that session is on, and
/// everything a tab does - reading, saving, asking a language server - goes by that pair. So a
/// file of no project the window has open is given a session on the project it does sit in:
/// the repo around it, or its own folder when it is in no repo, as `/etc/hostname` is.
///
/// Not in the browser's build: only [`crate::backend::Backend::place_file`] is answered
/// with one.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct FilePlaced {
    pub(crate) session_id: String,
    /// The folder the session is on, which `file_path` starts from.
    pub(crate) project: String,
    pub(crate) file_path: String,
    /// Whether anything is at the path yet. A tab on a path nothing is at starts empty, and
    /// its first save creates the file.
    pub(crate) exists: bool,
}
