//! Finding a file of the repo by name, with `ag`.
//!
//! `ag -g` lists the paths that match a pattern and leaves out whatever the repo ignores,
//! which is the whole reason it is here rather than a directory walk of our own: honouring
//! `.gitignore` is `ag`'s job and it already does it. It runs where the repo is — beside the
//! server on a `--remote` connection — so this sits with the rest of the service.

use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::api::FileMatchesPayload;

/// How many paths the search hands back. A query loose enough to match more than this is one
/// the user is still typing; the payload says it was cut short so the palette can too.
const MATCH_LIMIT: usize = 200;

/// The searcher. Required to be installed — it is what makes the search ignore-aware.
const SEARCHER: &str = "ag";

/// Every path in the repo whose name matches `query`, nearest the root first.
///
/// Dotfiles are included and `.git` is not: `.github/workflows/ci.yml` is a file of the repo
/// and the object store is not.
pub(crate) fn matching_paths(repo_path: &Path, query: &str) -> Result<FileMatchesPayload> {
    let pattern = pattern_for(query);
    let output = Command::new(SEARCHER)
        .current_dir(repo_path)
        // A window opened from a launcher has a bare PATH, without the `/opt/homebrew/bin` the
        // user installed `ag` into — see [`crate::shell_path`].
        .env("PATH", crate::shell_path::installed_tools_path())
        .args(["--hidden", "--ignore", ".git", "--nocolor", "-g", &pattern])
        .output()
        .with_context(|| {
            format!("{SEARCHER} has to be installed to find files by name: could not run it")
        })?;

    // 1 is "nothing matched", which is an empty list rather than a failure.
    match output.status.code() {
        Some(0) => {}
        Some(1) => return Ok(FileMatchesPayload::default()),
        _ => bail!("{}", String::from_utf8_lossy(&output.stderr).trim()),
    }

    let mut paths: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    // Shallow before deep, and alphabetical within a depth: the file being looked for is more
    // often the one near the top of the tree than one buried under a vendored directory.
    paths.sort_by_key(|path| (path.matches('/').count(), path.clone()));

    let truncated = paths.len() > MATCH_LIMIT;
    paths.truncate(MATCH_LIMIT);
    Ok(FileMatchesPayload {
        files: paths,
        truncated,
    })
}

/// The regex `ag` is given for a typed query.
///
/// Everything typed is matched literally, so a query with a `.` or a `+` in it finds the file
/// with that character in its name. Spaces are the one exception: they stand for "and then,
/// further along the path", which is what makes `nat pal` find `src/native/palette.rs`.
fn pattern_for(query: &str) -> String {
    query
        .split_whitespace()
        .map(escape_regex)
        .collect::<Vec<_>>()
        .join(".*")
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_matched_in_order_along_the_path() {
        assert_eq!(pattern_for("nat pal"), "nat.*pal");
    }

    #[test]
    fn what_was_typed_is_matched_literally() {
        assert_eq!(pattern_for("palette.rs"), "palette\\.rs");
        assert_eq!(pattern_for("a+b (c)"), "a\\+b.*\\(c\\)");
    }

    #[test]
    fn an_empty_query_matches_every_path() {
        assert_eq!(pattern_for("   "), "");
    }
}
