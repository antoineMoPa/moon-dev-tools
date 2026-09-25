//! `moon open <file>`: the command line's way into a window that is already open.
//!
//! A window opens where a path is easiest to name, but the easiest place to name one is the
//! shell: tab completion is the shell's own, it completes hidden files the same as any
//! other, and `moon open .moontasks/notes.md` lands in the window it was typed in front of.
//! Which window that is, and how it is reached, is [`crate::instances`]' business.
//!
//! A file of a project no window is open on lands in a window all the same - the one that
//! was in front most recently - which opens a session on that project to put it in. So the
//! command is worth typing wherever a window is open at all, rather than only where one
//! happens to be open on the right repo.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{MoonCommand, PROGRAM};
use crate::instances;

/// `moon open [--wait] <path>[:<line>]`, and `moon edit` which is the same thing.
pub(super) fn parse_open(args: Vec<String>) -> Result<MoonCommand> {
    let mut args = args.into_iter().peekable();
    let wait = args.next_if(|arg| arg == "--wait").is_some();
    let named = args
        .next()
        .with_context(|| format!("`{PROGRAM} open` needs a file to open\n\n{}", help_text()))?;
    // A word starting with a dash is an option being tried, not a file: `--wait` is the only
    // one, and a tab opened on a file called `--foo` is nobody's idea of an answer. A file
    // really named that way is reached as `./--foo`.
    if named.starts_with('-') {
        bail!(
            "`{PROGRAM} open` takes `--wait` and a file, not {named}\n\n{}",
            help_text()
        );
    }
    if let Some(extra) = args.next() {
        bail!("`{PROGRAM} open` opens one file, so it has nothing to do with {extra}");
    }

    let (path, line) = split_line_number(&named);
    Ok(MoonCommand::Open { path, line, wait })
}

/// `moon open --help`: how a file is named, and where it lands.
pub(super) fn help_text() -> String {
    format!(
        "{PROGRAM} open [--wait] <path>[:<line>]

Opens a file in a window that is already open: the one on the file's project, or the one
last in front when no window is open on it. `{PROGRAM} edit` is the same command.

Usage:
  {PROGRAM} open <path>
  {PROGRAM} open <path>:<line>
  {PROGRAM} edit <path>
  {PROGRAM} edit --wait <path>

Options:
  --wait   return only once the file's tab is closed, so a program waiting on its editor
           - git, for a commit message - reads the file back when you are done with it:
             git config --global core.editor \"{PROGRAM} edit --wait\"

Examples:
  {PROGRAM} open src/main.rs
  {PROGRAM} open src/main.rs:42
  {PROGRAM} edit .moontasks/notes.md

The path is read against the directory this shell is in, the way the shell completed it.
A path nothing is at yet opens an empty tab, and the file is created when that tab is saved;
its folder has to exist already.
`{PROGRAM} list` says which windows are open, and what they are on."
    )
}

/// `src/lib.rs:42` names a line of a file, the way every tool that prints a place in a file
/// writes one, so that is how `moon open` reads one.
///
/// A path is only split when what follows the colon is a number: a file whose name really
/// does end in `:something` is named by itself, and nothing else is a line number.
fn split_line_number(named: &str) -> (String, Option<usize>) {
    let Some((path, tail)) = named.rsplit_once(':') else {
        return (named.to_string(), None);
    };
    match tail.parse::<usize>() {
        Ok(line) if !path.is_empty() => (path.to_string(), Some(line)),
        _ => (named.to_string(), None),
    }
}

/// Open a file in a window, and say which window took it.
///
/// The path is resolved here rather than in the window: it is typed against the directory
/// this shell is in, and the window is somewhere else entirely.
///
/// A path nothing is at yet is a file about to be written, the way `vim notes.md` is: the
/// window opens an empty tab on it, and the file is only created when that tab is saved - see
/// [`crate::native::panes::OpenPaneRequest::NewFile`]. Its folder has to exist: a folder that
/// does not is more likely a typo than a place to start writing.
pub(super) fn open_file(path: &str, line: Option<usize>, wait: bool) -> Result<()> {
    let (file, new) = file_to_open(path)?;

    let instance = instances::open_file(&file, line, wait)?;
    let at = match line {
        Some(line) => format!(":{line}"),
        None => String::new(),
    };
    let new = if new {
        " (new file, created on save)"
    } else {
        ""
    };
    println!(
        "{}{at}{new} → {} on {}",
        file.display(),
        instance.program,
        instance.project_path
    );
    if wait {
        instances::wait_until_closed(&instance, &file)?;
    }
    Ok(())
}

/// The file a path names, resolved the way the windows' records are, and whether nothing is
/// at it yet. Nothing is written here: a new file is the window's to create, when its tab is
/// saved.
fn file_to_open(path: &str) -> Result<(PathBuf, bool)> {
    let named = PathBuf::from(path);
    if named.exists() {
        let file = named
            .canonicalize()
            .with_context(|| format!("could not resolve {path}"))?;
        if !file.is_file() {
            bail!("{} is not a file", file.display());
        }
        return Ok((file, false));
    }

    let name = named
        .file_name()
        .with_context(|| format!("{path} does not name a file"))?;
    // A bare `notes.md` is in the folder the shell is in, which is an empty parent.
    let folder = match named.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let folder = folder
        .canonicalize()
        .with_context(|| format!("there is no folder to put {path} in"))?;
    Ok((folder.join(name), true))
}

/// The folder `moon shell <folder>` names, resolved the way the windows' records are. It has
/// to be there: a shell is started in it, and a folder that is not is more likely a typo
/// than a place to start one.
pub(super) fn folder_to_open(path: &str) -> Result<PathBuf> {
    let folder = Path::new(path)
        .canonicalize()
        .with_context(|| format!("there is no folder at {path}"))?;
    if !folder.is_dir() {
        bail!("{} is not a folder", folder.display());
    }
    Ok(folder)
}

pub(super) fn list_windows() -> Result<()> {
    let running = instances::running();
    if running.is_empty() {
        println!("no {PROGRAM} window is open on this machine");
        return Ok(());
    }
    let asked_from = instances::shell_window();
    for instance in running {
        // The window this shell belongs to is the one `moon open` prefers, so it is worth
        // being able to see which that is.
        let this_one = if Some(instance.pid) == asked_from {
            " (this shell's)"
        } else {
            ""
        };
        println!(
            "{:>7}  {:<12} {}{this_one}",
            instance.pid, instance.program, instance.project_path
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::command::parse_command;
    use super::*;

    fn parse(args: &[&str]) -> Result<MoonCommand> {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
    }

    #[test]
    fn a_path_and_a_line_are_told_apart_by_the_colon() {
        assert_eq!(
            parse(&["open", "src/main.rs:42"]).expect("expected it to parse"),
            MoonCommand::Open {
                path: "src/main.rs".to_string(),
                line: Some(42),
                wait: false,
            }
        );
    }

    /// `edit` and `open` are the same command.
    #[test]
    fn a_file_is_opened_under_either_word() {
        assert_eq!(
            parse(&["edit", "src/main.rs"]).expect("expected it to parse"),
            parse(&["open", "src/main.rs"]).expect("expected it to parse")
        );
    }

    /// What git runs when `core.editor` is `moon edit --wait`: the file comes last.
    #[test]
    fn waiting_is_asked_for_ahead_of_the_file() {
        assert_eq!(
            parse(&["edit", "--wait", ".git/COMMIT_EDITMSG"]).expect("expected it to parse"),
            MoonCommand::Open {
                path: ".git/COMMIT_EDITMSG".to_string(),
                line: None,
                wait: true,
            }
        );
    }

    #[test]
    fn a_file_with_a_colon_in_its_name_is_a_file() {
        assert_eq!(
            parse(&["open", "notes:tuesday.md"]).expect("expected it to parse"),
            MoonCommand::Open {
                path: "notes:tuesday.md".to_string(),
                line: None,
                wait: false,
            }
        );
    }

    #[test]
    fn opening_nothing_says_what_is_missing() {
        let error = parse(&["open"]).expect_err("expected a refusal");

        assert!(format!("{error}").contains("needs a file"), "got {error}");
    }

    /// `moon edit --help` used to open a tab on a file called `--help`.
    #[test]
    fn asking_for_help_is_answered_rather_than_opened() {
        assert_eq!(
            parse(&["edit", "--help"]).expect("expected it to parse"),
            MoonCommand::OpenHelp
        );
        assert_eq!(
            parse(&["open", "-h"]).expect("expected it to parse"),
            MoonCommand::OpenHelp
        );
        assert!(help_text().contains("open <path>:<line>"));
    }

    #[test]
    fn an_option_is_refused_rather_than_opened_as_a_file() {
        let error = parse(&["open", "--line=3"]).expect_err("expected a refusal");

        assert!(format!("{error}").contains("takes `--wait`"), "got {error}");
    }

    #[test]
    fn opening_two_files_at_once_is_refused() {
        let error = parse(&["open", "one.rs", "two.rs"]).expect_err("expected a refusal");

        assert!(format!("{error}").contains("one file"), "got {error}");
    }

    /// A folder to stand in for the directory a shell is in, named after the test so two
    /// cannot collide.
    fn temporary_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "moonreview-test-open-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("expected a folder");
        folder
            .canonicalize()
            .expect("expected the folder to resolve")
    }

    /// The window creates a new file when its tab is saved, so naming one writes nothing.
    #[test]
    fn a_file_that_does_not_exist_yet_is_named_without_being_created() {
        // Arrange
        let folder = temporary_folder("new");
        let path = folder.join("notes.md");

        // Act
        let (file, new) =
            file_to_open(&path.display().to_string()).expect("expected the path to resolve");

        // Assert
        assert!(new);
        assert_eq!(file, path);
        assert!(!path.exists(), "nothing should be written before a save");
    }

    #[test]
    fn a_file_that_exists_is_opened_as_it_is() {
        // Arrange
        let folder = temporary_folder("existing");
        let path = folder.join("notes.md");
        std::fs::write(&path, "already written").expect("expected to write the file");

        // Act
        let (file, new) =
            file_to_open(&path.display().to_string()).expect("expected the file to resolve");

        // Assert
        assert!(!new);
        assert_eq!(file, path);
        assert_eq!(
            std::fs::read_to_string(&path).expect("expected a file"),
            "already written"
        );
    }

    /// `moon shell <folder>` starts a shell in the folder, so the folder has to be there and
    /// be a folder: a file is not somewhere a shell starts.
    #[test]
    fn a_shell_s_folder_is_resolved_and_a_file_is_refused() {
        let folder = temporary_folder("shell-folder");
        let file = folder.join("notes.md");
        std::fs::write(&file, "").expect("expected to write the file");

        assert_eq!(
            folder_to_open(&folder.display().to_string()).expect("expected the folder"),
            folder
        );
        let error = folder_to_open(&file.display().to_string()).expect_err("expected a refusal");
        assert!(
            format!("{error}").contains("is not a folder"),
            "got {error}"
        );
        let error = folder_to_open(&folder.join("nowhere").display().to_string())
            .expect_err("expected a refusal");
        assert!(format!("{error}").contains("no folder at"), "got {error}");
    }

    #[test]
    fn a_file_in_a_folder_that_does_not_exist_is_refused() {
        let folder = temporary_folder("missing-folder");
        let path = folder.join("nowhere").join("notes.md");

        let error = file_to_open(&path.display().to_string()).expect_err("expected a refusal");

        assert!(
            format!("{error}").contains("no folder to put"),
            "got {error}"
        );
        assert!(!folder.join("nowhere").exists());
    }
}
