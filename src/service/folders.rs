//! Folders - lists a folder of this machine's disk for a window's own file picker, and gives
//! a file anywhere on that disk a session to be read and written through.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::api::{
    AppState, OpenSessionRequest,
    folders::{EntryKind, FilePlaced, FolderEntry, FolderListing},
};

/// What `~` at the start of a typed path stands for.
const HOME_MARK: &str = "~";

/// What is in a folder, for the picker a window draws itself - see
/// `crate::native::file_picker`.
///
/// The folder is named the way a person types one: from `/`, or from `~` for the home of
/// whoever this server runs as - which only this side can say where is. Anything else is
/// refused rather than read against the folder this process happens to be running in.
pub(crate) fn list_folder(path: &str) -> Result<FolderListing> {
    let folder = folder_named(path)?
        .canonicalize()
        .map_err(|error| anyhow!("{path} cannot be listed: {error}"))?;
    let read = fs::read_dir(&folder)
        .map_err(|error| anyhow!("{} cannot be listed: {error}", folder.display()))?;

    let mut entries = Vec::new();
    for entry in read {
        let entry =
            entry.map_err(|error| anyhow!("{} cannot be listed: {error}", folder.display()))?;
        // A name that is not text cannot be sent as text, nor handed back to be opened.
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Through a symlink, so a link to a folder is a folder to go into. One that leads
        // nowhere is left as a file: picking it says what is wrong with it.
        let leads_to_a_folder = fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir());
        entries.push(FolderEntry {
            name,
            kind: match leads_to_a_folder {
                true => EntryKind::Folder,
                false => EntryKind::File,
            },
        });
    }

    let folder = folder.into_os_string().into_string().map_err(|folder| {
        anyhow!(
            "{} is not a path that can be written as text",
            folder.display()
        )
    })?;
    Ok(FolderListing { folder, entries })
}

/// The folder a typed path names, with `~` read as this account's home.
fn folder_named(path: &str) -> Result<PathBuf> {
    let named = Path::new(path);
    if named.is_absolute() {
        return Ok(named.to_path_buf());
    }
    let Ok(under_home) = named.strip_prefix(HOME_MARK) else {
        bail!("{path} is not a folder named from / or from ~");
    };
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .context("this server's account has no home folder for ~ to stand for")?;
    Ok(PathBuf::from(home).join(under_home))
}

/// Open a session on the project holding a file, and say what the file is called inside it -
/// see [`FilePlaced`]. The project is the repo around the file, or the folder it is in when no
/// repo is around it: the same rule a window is opened on a folder by.
///
/// The path is absolute. A symlink is followed to the file it leads to, since that is the
/// file a save writes; a path nothing is at yet is placed by its folder, which has to exist.
pub(crate) fn place_file(state: &AppState, path: &str) -> Result<FilePlaced> {
    let named = Path::new(path);
    if !named.is_absolute() {
        bail!("{path} is not an absolute path");
    }
    let (file, exists) = match named.canonicalize() {
        Ok(file) => (file, true),
        Err(_) => {
            let (Some(folder), Some(name)) = (named.parent(), named.file_name()) else {
                bail!("{path} does not name a file");
            };
            let folder = folder
                .canonicalize()
                .map_err(|error| anyhow!("there is no folder to put {path} in: {error}"))?;
            (folder.join(name), false)
        }
    };
    if file.is_dir() {
        bail!("{} is a folder, not a file", file.display());
    }
    let folder = file
        .parent()
        .with_context(|| format!("{path} does not name a file"))?;

    let opened = super::open_session(
        state,
        OpenSessionRequest {
            repo_path: folder.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )?;
    let project = crate::api::with_session(state, &opened.session_id, |session| {
        Ok(session.repo_path.clone())
    })?;
    let file_path = file
        .strip_prefix(&project)
        .expect("a project holds the folder it was found from");

    Ok(FilePlaced {
        session_id: opened.session_id,
        project: project.display().to_string(),
        file_path: file_path.display().to_string(),
        exists,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder to list, named after the test so two cannot collide, and resolved the way a
    /// listing's own folder is.
    fn scratch_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("moon-folders-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("expected a folder");
        folder
            .canonicalize()
            .expect("expected the folder to resolve")
    }

    fn kind_of(listing: &FolderListing, name: &str) -> Option<EntryKind> {
        listing
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    }

    /// A listing says which entries are folders - a link to one included - and names the
    /// folder it listed by its resolved path, whichever way it was reached.
    #[test]
    fn a_folder_is_listed_with_what_each_entry_is() {
        let folder = scratch_folder("listed");
        fs::create_dir(folder.join("src")).expect("expected a folder");
        fs::write(folder.join(".hidden"), "").expect("expected a file");
        fs::write(folder.join("notes.md"), "").expect("expected a file");
        std::os::unix::fs::symlink(folder.join("src"), folder.join("link"))
            .expect("expected a link");

        let listing = list_folder(&format!("{}/src/..", folder.display()))
            .expect("expected the folder to be listed");

        assert_eq!(Path::new(&listing.folder), folder);
        assert_eq!(listing.entries.len(), 4);
        assert_eq!(kind_of(&listing, "src"), Some(EntryKind::Folder));
        assert_eq!(kind_of(&listing, "link"), Some(EntryKind::Folder));
        assert_eq!(kind_of(&listing, "notes.md"), Some(EntryKind::File));
        assert_eq!(kind_of(&listing, ".hidden"), Some(EntryKind::File));
    }

    #[test]
    fn a_folder_named_from_neither_the_root_nor_home_is_refused() {
        let error = list_folder("src/native").expect_err("expected a refusal");

        assert!(
            format!("{error}").contains("from / or from ~"),
            "got {error}"
        );
    }

    #[test]
    fn a_folder_that_is_not_there_says_so() {
        let folder = scratch_folder("missing");

        let error =
            list_folder(&format!("{}/nowhere", folder.display())).expect_err("expected a refusal");

        assert!(
            format!("{error}").contains("cannot be listed"),
            "got {error}"
        );
    }
}
