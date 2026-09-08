//! `moon open <file>`: the command line's way into a window that is already open.
//!
//! A window opens where a path is easiest to name, but the easiest place to name one is the
//! shell: tab completion is the shell's own, it completes hidden files the same as any
//! other, and `moon open .moontasks/notes.md` lands in the window it was typed in front of.
//! Which window that is, and how it is reached, is [`crate::instances`]' business.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::{MoonCommand, PROGRAM};
use crate::instances;

/// `moon open <path>[:<line>]`, and `moon edit` which is the same thing.
pub(super) fn parse_open(args: Vec<String>) -> Result<MoonCommand> {
    let mut args = args.into_iter();
    let named = args
        .next()
        .with_context(|| format!("`{PROGRAM} open` needs a file to open"))?;
    if let Some(extra) = args.next() {
        bail!("`{PROGRAM} open` opens one file, so it has nothing to do with {extra}");
    }

    let (path, line) = split_line_number(&named);
    Ok(MoonCommand::Open { path, line })
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
pub(super) fn open_file(path: &str, line: Option<usize>) -> Result<()> {
    let file = PathBuf::from(path)
        .canonicalize()
        .with_context(|| format!("there is no file at {path}"))?;
    if !file.is_file() {
        bail!("{} is not a file", file.display());
    }

    let instance = instances::open_file(&file, line)?;
    let at = match line {
        Some(line) => format!(":{line}"),
        None => String::new(),
    };
    println!(
        "{}{at} → {} on {}",
        file.display(),
        instance.program,
        instance.project_path
    );
    Ok(())
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
    use super::super::parse_command;
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
                line: Some(42)
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

    #[test]
    fn a_file_with_a_colon_in_its_name_is_a_file() {
        assert_eq!(
            parse(&["open", "notes:tuesday.md"]).expect("expected it to parse"),
            MoonCommand::Open {
                path: "notes:tuesday.md".to_string(),
                line: None
            }
        );
    }

    #[test]
    fn opening_nothing_says_what_is_missing() {
        let error = parse(&["open"]).expect_err("expected a refusal");

        assert!(format!("{error}").contains("needs a file"), "got {error}");
    }

    #[test]
    fn opening_two_files_at_once_is_refused() {
        let error = parse(&["open", "one.rs", "two.rs"]).expect_err("expected a refusal");

        assert!(format!("{error}").contains("one file"), "got {error}");
    }
}
