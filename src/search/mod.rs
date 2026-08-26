//! Searching the repo with `ag`, by file name and by what the files hold.
//!
//! `ag` leaves out whatever the repo ignores, which is the whole reason it is here rather
//! than a directory walk of our own: honouring `.gitignore` is its job and it already does
//! it. The searches run where the repo is - beside the server on a `--remote` connection -
//! so this sits with the rest of the service.

pub(crate) mod file_contents;
pub(crate) mod file_names;

use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

/// The searcher. Required to be installed - it is what makes the searches ignore-aware.
const SEARCHER: &str = "ag";

/// The arguments both searches are run with: dotfiles are files of the repo and
/// `.github/workflows/ci.yml` is worth finding, while the object store under `.git` is not.
const REPO_ARGS: &[&str] = &["--hidden", "--ignore", ".git", "--nocolor"];

/// Run `ag` in the repo and hand back the lines it printed. `None` is "nothing matched",
/// which every caller shows as an empty list rather than as a failure.
fn search(repo_path: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new(SEARCHER)
        .current_dir(repo_path)
        // A window opened from a launcher has a bare PATH, without the `/opt/homebrew/bin`
        // the user installed `ag` into - see [`crate::shell_path`].
        .env("PATH", crate::shell_path::installed_tools_path())
        .args(REPO_ARGS)
        .args(args)
        .output()
        .with_context(|| format!("{SEARCHER} has to be installed to search: could not run it"))?;

    match output.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned())),
        Some(1) => Ok(None),
        _ => bail!("{}", String::from_utf8_lossy(&output.stderr).trim()),
    }
}

/// What was typed, as a regex that matches it literally: a query with a `.` or a `+` in it
/// finds the text that has that character in it rather than whatever the regex would mean.
fn escape_regex(term: &str) -> String {
    const SPECIAL: &[char] = &[
        '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$',
    ];
    let mut escaped = String::with_capacity(term.len());
    for character in term.chars() {
        if SPECIAL.contains(&character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
