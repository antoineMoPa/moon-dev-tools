//! Desktop entry folders - finds the entry files of this machine, and lists the applications
//! that the ones speaking for their id offer.

use std::{
    collections::HashSet,
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use super::{Session, entry};
use crate::api::applications::InstalledApplication;

/// Where a person's own data is under their home directory when `XDG_DATA_HOME` does not
/// say, and where the system's is when `XDG_DATA_DIRS` does not: the defaults of the XDG
/// Base Directory specification.
const DATA_HOME_UNDER_HOME: &str = ".local/share";
const DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The folder of a data folder that entries are in, and what an entry's file name ends in.
const ENTRIES_FOLDER: &str = "applications";
const ENTRY_EXTENSION: &str = "desktop";

/// The folders entries are read from, the person's own first: `applications` under
/// `XDG_DATA_HOME`, then under each of `XDG_DATA_DIRS` in the order it has them. A variable
/// that is unset or empty is its default.
///
/// Nothing is added for snap or flatpak: each puts the folder it exports its entries to in
/// `XDG_DATA_DIRS` when a person logs in, so a server started from a login session has them
/// and one started outside of any - a systemd unit - is to be given the variable.
pub(super) fn entries_folders(
    home: Option<OsString>,
    data_home: Option<OsString>,
    data_dirs: Option<OsString>,
) -> Vec<PathBuf> {
    let said = |variable: Option<OsString>| variable.filter(|value| !value.is_empty());
    // A user with no home directory has no folder of their own, only the system's.
    let data_home = said(data_home).map(PathBuf::from).or_else(|| {
        said(home).map(|home| PathBuf::from(home).join(DATA_HOME_UNDER_HOME))
    });
    let data_dirs = said(data_dirs).unwrap_or_else(|| DATA_DIRS.into());
    data_home
        .into_iter()
        .chain(env::split_paths(&data_dirs))
        // A path that is not absolute is one the specification has ignored.
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(ENTRIES_FOLDER))
        .collect()
}

/// The applications the entries of these folders offer a menu, sorted by name.
///
/// An entry is known by its id, and the first folder to have an id is the one that speaks
/// for it - even when what it says is that the entry is hidden, which is how a person takes
/// an application the system installed off their own menu.
pub(super) fn offered_in(folders: &[PathBuf], session: &Session) -> Vec<InstalledApplication> {
    let mut spoken_for = HashSet::new();
    let mut offered = Vec::new();
    for folder in folders {
        let mut files = Vec::new();
        collect_entry_files(folder, "", &mut files);
        for (id, file) in files {
            if !spoken_for.insert(id) {
                continue;
            }
            match entry::read(&file, session) {
                Ok(Some(application)) => offered.push(application),
                Ok(None) => {}
                // One package's broken file is no reason to offer nothing of the others'.
                Err(error) => eprintln!("[moonreview] ignoring {}: {error:#}", file.display()),
            }
        }
    }
    // By command after name, so two entries of one name are in the same order every time.
    offered.sort_by_cached_key(|application| {
        (application.name.to_lowercase(), application.command.clone())
    });
    offered
}

/// Add every entry file under `folder` to `files`, each with its id: its path under the
/// entries folder with a `-` for each `/`, so `kde/okular.desktop` is `kde-okular.desktop`.
/// `id_of_folder` is what the ids of this folder's files start with.
fn collect_entry_files(folder: &Path, id_of_folder: &str, files: &mut Vec<(String, PathBuf)>) {
    let children = match fs::read_dir(folder) {
        Ok(children) => children,
        // Most machines have no folder at most of the places `XDG_DATA_DIRS` names.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            eprintln!("[moonreview] not reading {}: {error}", folder.display());
            return;
        }
    };
    for child in children.flatten() {
        let path = child.path();
        let name = child.file_name().to_string_lossy().into_owned();
        // `file_type` does not follow a link, so a folder linked into itself is not walked
        // forever. An entry file that is a link - how flatpak exports its own - is read
        // through it like any other.
        if child.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_entry_files(&path, &format!("{id_of_folder}{name}-"), files);
        } else if path.extension().is_some_and(|extension| extension == ENTRY_EXTENSION) {
            files.push((format!("{id_of_folder}{name}"), path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder of entries, removed when the test is over.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let folder = env::temp_dir().join(format!(
                "moonreview-desktop-entries-{}-{test}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&folder);
            fs::create_dir_all(&folder).expect("a scratch folder");
            Self(folder)
        }

        /// Write an entry file, and the folders above it.
        fn entry(&self, path_under_scratch: &str, text: &str) {
            let file = self.0.join(path_under_scratch);
            fs::create_dir_all(file.parent().expect("a file is in a folder"))
                .expect("the entry's folder");
            fs::write(file, text).expect("the entry's file");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn session() -> Session {
        Session {
            desktops: Vec::new(),
            messages_locale: None,
            path: OsString::new(),
        }
    }

    fn names(offered: &[InstalledApplication]) -> Vec<&str> {
        offered
            .iter()
            .map(|application| application.name.as_str())
            .collect()
    }

    #[test]
    fn an_unset_or_empty_variable_is_its_default() {
        let unset = entries_folders(Some("/home/moon".into()), None, None);
        let empty = entries_folders(Some("/home/moon".into()), Some("".into()), Some("".into()));

        assert_eq!(
            unset,
            [
                PathBuf::from("/home/moon/.local/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
        assert_eq!(empty, unset);
    }

    /// Where snap and flatpak entries come from: the session's own variable.
    #[test]
    fn the_folders_are_the_ones_the_variables_name_in_their_order() {
        let folders = entries_folders(
            Some("/home/moon".into()),
            Some("/data/mine".into()),
            Some("/var/lib/flatpak/exports/share:relative/share::/usr/share".into()),
        );

        assert_eq!(
            folders,
            [
                PathBuf::from("/data/mine/applications"),
                PathBuf::from("/var/lib/flatpak/exports/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn the_first_folder_to_have_an_id_speaks_for_it() {
        let scratch = Scratch::new("shadowing");
        scratch.entry(
            "mine/browser.desktop",
            "[Desktop Entry]\nType=Application\nName=My Browser\nExec=mine\n",
        );
        // Hidden in the person's own folder: off the menu, whatever the system's says.
        scratch.entry(
            "mine/editor.desktop",
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor\nHidden=true\n",
        );
        for id in ["browser", "editor", "clock"] {
            scratch.entry(
                &format!("system/{id}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName=System {id}\nExec={id}\n"),
            );
        }

        let offered = offered_in(
            &[scratch.0.join("mine"), scratch.0.join("system")],
            &session(),
        );

        assert_eq!(names(&offered), ["My Browser", "System clock"]);
        assert_eq!(offered[0].command, "mine");
    }

    #[test]
    fn the_applications_are_sorted_by_name_whatever_its_case() {
        let scratch = Scratch::new("sorting");
        for name in ["xterm", "Chromium", "Vim"] {
            scratch.entry(
                &format!("{name}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n"),
            );
        }

        let offered = offered_in(&[scratch.0.clone()], &session());

        assert_eq!(names(&offered), ["Chromium", "Vim", "xterm"]);
    }

    /// A file in a folder of the entries folder has that folder in its id, so it is the
    /// entry another folder's `kde-okular.desktop` speaks for.
    #[test]
    fn an_entry_in_a_folder_of_its_own_has_the_folder_in_its_id() {
        let scratch = Scratch::new("ids");
        scratch.entry(
            "mine/kde-okular.desktop",
            "[Desktop Entry]\nType=Application\nName=Mine\nExec=mine\n",
        );
        scratch.entry(
            "system/kde/okular.desktop",
            "[Desktop Entry]\nType=Application\nName=System\nExec=system\n",
        );
        scratch.entry("system/notes.txt", "not an entry");

        let offered = offered_in(
            &[scratch.0.join("mine"), scratch.0.join("system")],
            &session(),
        );

        assert_eq!(names(&offered), ["Mine"]);
    }

    /// One package's broken file does not take the others' off the menu.
    #[test]
    fn an_entry_that_cannot_be_read_is_left_out_and_the_rest_are_offered() {
        let scratch = Scratch::new("broken");
        scratch.entry("broken.desktop", "Name=No group\n");
        scratch.entry(
            "clock.desktop",
            "[Desktop Entry]\nType=Application\nName=Clock\nExec=xclock\n",
        );

        let offered = offered_in(
            &[scratch.0.clone(), scratch.0.join("not-there")],
            &session(),
        );

        assert_eq!(names(&offered), ["Clock"]);
    }
}
