//! Finding text in the files of the repo.
//!
//! One row per matching line, the way `ag` prints them: the path, the line number, and the
//! line itself, which is what the palette shows and what lets it open the file at that line.

use std::path::Path;

use anyhow::{Result, bail};

use crate::{
    api::{ContentMatch, SearchProgress, SearchScope},
    search::{self, Ended, Flow, SearchListener},
};

/// How many matching lines the search hands back. A query loose enough to match more than
/// this is one the user is still typing; the report says it was cut short so the palette
/// can too - and the search stops there, as the rest would never be shown.
const MATCH_LIMIT: usize = 200;

/// How much of a matching line is kept. A minified file is one line thousands of characters
/// long, and neither the palette nor the wire has any use for the whole of it.
const LINE_LIMIT: usize = 300;

/// Every line of the files in `scope` that holds `query`, in the order `ag` walks the tree,
/// reported to the listener as they come in and once more when the search is over.
///
/// The text is matched literally and without regard for case, the way the find bar over a
/// pane matches: what is typed is what is looked for, brackets and dots included.
pub(crate) fn stream_matching_lines(
    repo_path: &Path,
    query: &str,
    scope: SearchScope,
    listener: &mut dyn SearchListener<ContentMatch>,
) -> Result<()> {
    // An empty query would match every line of every file. Nothing has been asked for yet.
    if query.is_empty() {
        listener.found(SearchProgress::nothing());
        return Ok(());
    }
    let mut matches: Vec<ContentMatch> = Vec::new();
    let mut truncated = false;
    let mut unreadable = None;
    let ended = search::stream(
        repo_path,
        scope,
        &[
            "--literal",
            "--ignore-case",
            "--numbers",
            "--nogroup",
            "--",
            query,
        ],
        &mut |lines| {
            if !listener.wanted() {
                return Flow::Stop;
            }
            let mut changed = false;
            for line in lines {
                if line.is_empty() {
                    continue;
                }
                if matches.len() == MATCH_LIMIT {
                    truncated = true;
                    break;
                }
                match match_of(&line) {
                    Ok(found) => {
                        matches.push(found);
                        changed = true;
                    }
                    Err(error) => {
                        unreadable = Some(error);
                        return Flow::Stop;
                    }
                }
            }
            if truncated {
                // Enough: the rest would never be shown, so the search ends here, done.
                listener.found(SearchProgress {
                    matches: matches.clone(),
                    truncated,
                    done: true,
                });
                return Flow::Stop;
            }
            if changed {
                listener.found(SearchProgress {
                    matches: matches.clone(),
                    truncated,
                    done: false,
                });
            }
            Flow::Continue
        },
    )?;
    if let Some(error) = unreadable {
        return Err(error);
    }
    if ended == Ended::Finished {
        listener.found(SearchProgress {
            matches,
            truncated,
            done: true,
        });
    }
    Ok(())
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
    use crate::search::LastReport;

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

    fn files_of(progress: &SearchProgress<ContentMatch>) -> Vec<&str> {
        let mut files: Vec<&str> = progress
            .matches
            .iter()
            .map(|found| found.file_path.as_str())
            .collect();
        files.sort_unstable();
        files
    }

    /// The lines of the files the repo ignores are found only in the scope that asks for them,
    /// and the lines under `node_modules` in neither.
    #[test]
    fn the_ignored_files_are_read_only_when_asked_for() {
        let repo = search::repo_with_an_ignored_file("contents");

        let mut of_the_repo = LastReport::wanting();
        stream_matching_lines(&repo, "needle", SearchScope::RepoFiles, &mut of_the_repo).unwrap();
        assert_eq!(files_of(of_the_repo.done()), vec!["src/kept.rs"]);

        let mut with_ignored = LastReport::wanting();
        stream_matching_lines(
            &repo,
            "needle",
            SearchScope::IncludingIgnored,
            &mut with_ignored,
        )
        .unwrap();
        assert_eq!(
            files_of(with_ignored.done()),
            vec!["build/left.rs", "src/kept.rs"]
        );
    }

    /// Past the limit the search stops: what it reports is cut short, and done.
    #[test]
    fn the_search_stops_at_the_limit() {
        let repo = search::repo_with_an_ignored_file("limit");
        std::fs::write(
            repo.join("src/many.rs"),
            "needle\n".repeat(MATCH_LIMIT + 50),
        )
        .expect("failed to write");

        let mut report = LastReport::wanting();
        stream_matching_lines(&repo, "needle", SearchScope::RepoFiles, &mut report).unwrap();

        let done = report.done();
        assert_eq!(done.matches.len(), MATCH_LIMIT);
        assert!(done.truncated);
    }
}
