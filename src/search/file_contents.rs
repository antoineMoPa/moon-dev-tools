//! Finding text in the files of the repo.
//!
//! One row per matching line, the way `ag` prints them: the path, the line number, and the
//! line itself, which is what the palette shows and what lets it open the file at that line.

use std::path::Path;

use anyhow::{Result, bail};

use crate::{
    api::{ContentMatch, ContentMatchesPayload},
    search,
};

/// How many matching lines the search hands back. A query loose enough to match more than
/// this is one the user is still typing; the payload says it was cut short so the palette
/// can too.
const MATCH_LIMIT: usize = 200;

/// How much of a matching line is kept. A minified file is one line thousands of characters
/// long, and neither the palette nor the wire has any use for the whole of it.
const LINE_LIMIT: usize = 300;

/// Every line of the repo that holds `query`, in the order `ag` walks the tree.
///
/// The text is matched literally and without regard for case, the way the find bar over a
/// pane matches: what is typed is what is looked for, brackets and dots included.
pub(crate) fn matching_lines(repo_path: &Path, query: &str) -> Result<ContentMatchesPayload> {
    // An empty query would match every line of every file. Nothing has been asked for yet.
    if query.is_empty() {
        return Ok(ContentMatchesPayload::default());
    }
    let Some(output) = search::search(
        repo_path,
        &[
            "--literal",
            "--ignore-case",
            "--numbers",
            "--nogroup",
            "--",
            query,
        ],
    )?
    else {
        return Ok(ContentMatchesPayload::default());
    };

    let lines: Vec<&str> = output.lines().filter(|line| !line.is_empty()).collect();
    let truncated = lines.len() > MATCH_LIMIT;
    let matches = lines
        .iter()
        .take(MATCH_LIMIT)
        .map(|line| match_of(line))
        .collect::<Result<Vec<_>>>()?;

    Ok(ContentMatchesPayload { matches, truncated })
}

/// One printed line, which `ag` prints as `path:line number:the line`.
fn match_of(printed: &str) -> Result<ContentMatch> {
    let mut parts = printed.splitn(3, ':');
    let (Some(file_path), Some(line_number), Some(line)) =
        (parts.next(), parts.next(), parts.next())
    else {
        bail!("could not read what the search printed: {printed}");
    };
    let Ok(line_number) = line_number.parse::<usize>() else {
        bail!("could not read what the search printed: {printed}");
    };

    let line = line.trim();
    Ok(ContentMatch {
        file_path: file_path.to_string(),
        line_number,
        line: line.chars().take(LINE_LIMIT).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_printed_line_is_read_as_a_match() {
        let found = match_of("src/native/palette.rs:42:    let query = trimmed;").unwrap();

        assert_eq!(found.file_path, "src/native/palette.rs");
        assert_eq!(found.line_number, 42);
        assert_eq!(found.line, "let query = trimmed;");
    }

    #[test]
    fn the_colons_of_the_line_itself_are_left_alone() {
        let found = match_of("src/main.rs:7:use crate::api::AppState;").unwrap();

        assert_eq!(found.line, "use crate::api::AppState;");
    }

    #[test]
    fn a_line_the_search_could_not_have_printed_is_an_error() {
        assert!(match_of("src/main.rs:not a line number:text").is_err());
        assert!(match_of("nothing to read").is_err());
    }
}
