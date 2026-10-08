//! The picker's line read as a path, and the rows listed under it.
//!
//! The line is the whole of where the picker is: everything up to its last `/` is the folder
//! being listed, and what follows narrows the list - or, saving a new file, names it. Going
//! into a folder writes its name onto the line, and going up takes one off, so a path typed or
//! pasted whole and one walked row by row end in the same place.

use crate::api::folders::{EntryKind, FolderEntry, FolderListing};

use super::PickPurpose;

/// Between the folders of a path of the server's disk - see [`crate::api::folders`].
const SEPARATOR: char = '/';

/// The row that goes up a folder reads as the shell's name for it.
const UP_NAME: &str = "..";

/// The line split at its last `/`: the folder to list, the `/` kept on it, and what was typed
/// after. `None` when the line has no `/` in it yet, and so names no folder.
pub(super) fn split(path: &str) -> Option<(&str, &str)> {
    let folder_ends = path.rfind(SEPARATOR)? + SEPARATOR.len_utf8();
    Some(path.split_at(folder_ends))
}

/// A folder and a name in it, as one path.
pub(super) fn join(folder: &str, name: &str) -> String {
    // Only the root ends in the separator, being nothing else.
    match folder.ends_with(SEPARATOR) {
        true => format!("{folder}{name}"),
        false => format!("{folder}{SEPARATOR}{name}"),
    }
}

/// The folder holding a resolved one, written as the line writes a folder - ending in `/`.
/// `None` for the root, which nothing holds.
pub(super) fn parent_line(folder: &str) -> Option<String> {
    let (above, name) = folder.rsplit_once(SEPARATOR)?;
    (!name.is_empty()).then(|| format!("{above}{SEPARATOR}"))
}

/// A resolved folder as the line writes one to list it.
pub(super) fn folder_line(folder: &str) -> String {
    join(folder, "")
}

/// What picking a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowKind {
    /// Go up to the folder holding the one listed.
    Up,
    /// Go into this folder of the one listed.
    Folder,
    /// Pick this file of the folder listed.
    File,
    /// Pick the folder listed itself, which is what a pick of a folder ends on.
    ThisFolder,
    /// Pick the name typed on the line as a file of the folder listed. `replaces` when a file
    /// of that name is already there.
    NewName { replaces: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) kind: RowKind,
    /// The entry's name, or the name typed for a new file. Empty on the two rows that are
    /// about the folder listed rather than something in it.
    pub(super) name: String,
    /// A dotfile, drawn dimmed.
    pub(super) hidden: bool,
}

impl Row {
    fn about_the_folder(kind: RowKind) -> Self {
        Self {
            kind,
            name: String::new(),
            hidden: false,
        }
    }
}

/// How well a name answers what was typed, the best first. `None` for a name that does not
/// hold it at all, which is left off the list.
fn rank(name: &str, typed: &str) -> Option<u8> {
    let name = name.to_lowercase();
    let typed = typed.to_lowercase();
    if name == typed {
        Some(0)
    } else if name.starts_with(&typed) {
        Some(1)
    } else if name.contains(&typed) {
        Some(2)
    } else {
        None
    }
}

/// The rows under the line: what is about the folder itself first, then its entries - the
/// ones a pick of this purpose can use, narrowed to what was typed after the last `/`.
///
/// Folders come before files, and dotfiles after everything else: they are most of a home
/// folder and seldom what is being looked for, and typing the dot brings them to the top.
pub(super) fn rows_for(purpose: &PickPurpose, listing: &FolderListing, typed: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let names_a_folder = listing
        .entries
        .iter()
        .any(|entry| entry.kind == EntryKind::Folder && entry.name == typed);
    // Narrowing by the name a new file is offered under would hide every folder it could go
    // in, so that name narrows nothing until it has been typed over.
    let narrows = !typed.is_empty() && purpose.suggested_name() != Some(typed);

    match purpose.picks() {
        Picks::Folder if typed.is_empty() => {
            rows.push(Row::about_the_folder(RowKind::ThisFolder));
        }
        Picks::NewName if !typed.is_empty() && !names_a_folder => {
            let replaces = listing
                .entries
                .iter()
                .any(|entry| entry.kind == EntryKind::File && entry.name == typed);
            rows.push(Row {
                kind: RowKind::NewName { replaces },
                name: typed.to_string(),
                hidden: false,
            });
        }
        Picks::File | Picks::Folder | Picks::NewName => {}
    }
    if !narrows && parent_line(&listing.folder).is_some() {
        rows.push(Row::about_the_folder(RowKind::Up));
    }

    let mut entries: Vec<(u8, &FolderEntry)> = listing
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Folder || purpose.picks() == Picks::File)
        .filter_map(|entry| match narrows {
            true => Some((rank(&entry.name, typed)?, entry)),
            false => Some((0, entry)),
        })
        .collect();
    entries.sort_by_cached_key(|(rank, entry)| {
        (
            *rank,
            is_hidden(&entry.name),
            entry.kind != EntryKind::Folder,
            entry.name.to_lowercase(),
        )
    });
    rows.extend(entries.into_iter().map(|(_, entry)| Row {
        kind: match entry.kind {
            EntryKind::Folder => RowKind::Folder,
            EntryKind::File => RowKind::File,
        },
        name: entry.name.clone(),
        hidden: is_hidden(&entry.name),
    }));
    rows
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The row Enter acts on before an arrow has moved: the first that is not the way up, so
/// Enter on a picker just opened never walks out of the folder it opened on.
pub(super) fn first_choice(rows: &[Row]) -> usize {
    rows.iter()
        .position(|row| row.kind != RowKind::Up)
        .unwrap_or(0)
}

/// What a row reads as on the list.
pub(super) fn title_of(row: &Row) -> String {
    match &row.kind {
        RowKind::Up => UP_NAME.to_string(),
        RowKind::Folder => format!("{}{SEPARATOR}", row.name),
        RowKind::File => row.name.clone(),
        RowKind::ThisFolder => "open this folder".to_string(),
        RowKind::NewName { replaces: false } => format!("save as {}", row.name),
        RowKind::NewName { replaces: true } => format!("replace {}", row.name),
    }
}

/// What a purpose is after, which decides the entries listed and what Enter does on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Picks {
    /// A file that is there.
    File,
    /// A folder that is there.
    Folder,
    /// A name for a file, in a folder that is there.
    NewName,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(folder: &str, entries: &[(&str, EntryKind)]) -> FolderListing {
        FolderListing {
            folder: folder.to_string(),
            entries: entries
                .iter()
                .map(|(name, kind)| FolderEntry {
                    name: name.to_string(),
                    kind: *kind,
                })
                .collect(),
        }
    }

    fn titles(rows: &[Row]) -> Vec<String> {
        rows.iter().map(title_of).collect()
    }

    #[test]
    fn the_line_is_a_folder_and_what_was_typed_after_it() {
        assert_eq!(split("/etc/host"), Some(("/etc/", "host")));
        assert_eq!(split("/"), Some(("/", "")));
        assert_eq!(split("~/prog/"), Some(("~/prog/", "")));
        assert_eq!(split("~"), None);
        assert_eq!(split(""), None);
    }

    #[test]
    fn a_path_is_joined_and_taken_apart_at_the_root_too() {
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/etc", "hostname"), "/etc/hostname");
        assert_eq!(folder_line("/etc"), "/etc/");
        assert_eq!(folder_line("/"), "/");
        assert_eq!(parent_line("/etc/apt").as_deref(), Some("/etc/"));
        assert_eq!(parent_line("/etc").as_deref(), Some("/"));
        assert_eq!(parent_line("/"), None);
    }

    /// Folders before files, dotfiles after both, and the way up ahead of them all - but not
    /// what Enter lands on.
    #[test]
    fn a_folder_is_listed_folders_first_and_dotfiles_last() {
        let listing = listing(
            "/home/moon",
            &[
                (".bashrc", EntryKind::File),
                ("notes.md", EntryKind::File),
                (".config", EntryKind::Folder),
                ("prog", EntryKind::Folder),
            ],
        );

        let rows = rows_for(&PickPurpose::RepoFolder, &listing, "");
        assert_eq!(
            titles(&rows),
            ["open this folder", "..", "prog/", ".config/"]
        );
        assert_eq!(first_choice(&rows), 0);

        let rows = rows_for(&file_purpose(), &listing, "");
        assert_eq!(
            titles(&rows),
            ["..", "prog/", "notes.md", ".config/", ".bashrc"]
        );
        assert_eq!(first_choice(&rows), 1);
    }

    /// What is typed narrows the list whatever its case, and the name it spells out exactly
    /// is the first row, so a path pasted whole is one Enter from picked.
    #[test]
    fn what_is_typed_narrows_the_list_and_an_exact_name_comes_first() {
        let listing = listing(
            "/etc",
            &[
                ("hosts.allow", EntryKind::File),
                ("ghostty", EntryKind::Folder),
                ("hosts", EntryKind::File),
                ("fstab", EntryKind::File),
            ],
        );

        let rows = rows_for(&file_purpose(), &listing, "HOSTS");

        assert_eq!(titles(&rows), ["hosts", "hosts.allow"]);
        assert_eq!(
            titles(&rows_for(&file_purpose(), &listing, "host")),
            ["hosts", "hosts.allow", "ghostty/"]
        );
    }

    /// Saving a new file: the name offered hides no folder until it is typed over, a name
    /// typed is the first row, and one a file already has says it will replace it.
    #[test]
    fn a_new_name_is_offered_first_and_says_when_it_replaces_a_file() {
        let listing = listing(
            "/repo",
            &[("src", EntryKind::Folder), ("notes.md", EntryKind::File)],
        );
        let purpose = name_purpose("untitled");

        assert_eq!(
            titles(&rows_for(&purpose, &listing, "untitled")),
            ["save as untitled", "..", "src/"]
        );
        assert_eq!(
            titles(&rows_for(&purpose, &listing, "notes.md")),
            ["replace notes.md"]
        );
        // A folder's own name goes into it rather than being saved over it.
        assert_eq!(titles(&rows_for(&purpose, &listing, "src")), ["src/"]);
    }

    fn file_purpose() -> PickPurpose {
        PickPurpose::FileToEdit
    }

    fn name_purpose(suggested: &str) -> PickPurpose {
        // A pane's id is only ever handed out by a layout.
        let mut layout = egui_frames::Layout::new();
        let pane_id = layout.add_pane(layout.primary_frame(), (), None);
        PickPurpose::NameForUntitledFile {
            pane_id,
            session_id: String::new(),
            suggested: suggested.to_string(),
        }
    }
}
