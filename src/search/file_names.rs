//! Finding a file of the repo by name.
//!
//! `ag -g` lists the paths that match a pattern, which is a name search over the same set of
//! files a content search reads.

use std::path::Path;

use anyhow::Result;

use crate::{
    api::{SearchProgress, SearchScope},
    search::{self, Ended, Flow, SearchListener},
};

/// How many paths the search hands back. A query loose enough to match more than this is one
/// the user is still typing; the report says it was cut short so the palette can too.
const MATCH_LIMIT: usize = 200;

/// Every path in `scope` whose name matches `query`, nearest the root first, reported to the
/// listener as they come in and once more when the walk is over.
///
/// The walk goes to the end: which paths are nearest the root is not known before every
/// path has been seen.
pub(crate) fn stream_matching_paths(
    repo_path: &Path,
    query: &str,
    scope: SearchScope,
    listener: &mut dyn SearchListener<String>,
) -> Result<()> {
    let pattern = pattern_for(query);
    let mut nearest = Nearest::holding(MATCH_LIMIT);
    let ended = search::stream(repo_path, scope, &["-g", &pattern], &mut |lines| {
        if !listener.wanted() {
            return Flow::Stop;
        }
        let mut changed = false;
        for line in lines {
            let path = line.trim();
            if !path.is_empty() {
                changed |= nearest.offer(path.to_string());
            }
        }
        if changed {
            listener.found(nearest.report(false));
        }
        Flow::Continue
    })?;
    if ended == Ended::Finished {
        listener.found(nearest.report(true));
    }
    Ok(())
}

/// The paths worth showing out of everything the search prints: the `limit` nearest the
/// root, alphabetical within a depth, kept in that order as they come. The file being looked
/// for is more often the one near the top of the tree than one buried under a vendored
/// directory - and half a million printed paths are never sorted whole.
struct Nearest {
    paths: Vec<String>,
    limit: usize,
    /// Whether a path was left out for being further from the root than the ones kept.
    left_out: bool,
}

impl Nearest {
    fn holding(limit: usize) -> Self {
        Self {
            paths: Vec::new(),
            limit,
            left_out: false,
        }
    }

    /// Put a path where it belongs among the kept ones, or leave it out. Whether the kept
    /// ones changed.
    fn offer(&mut self, path: String) -> bool {
        let rank = rank_of(&path);
        let at = self
            .paths
            .binary_search_by(|kept| rank_of(kept).cmp(&rank))
            .unwrap_or_else(|at| at);
        if at >= self.limit {
            self.left_out = true;
            return false;
        }
        self.paths.insert(at, path);
        if self.paths.len() > self.limit {
            self.paths.pop();
            self.left_out = true;
        }
        true
    }

    fn report(&self, done: bool) -> SearchProgress<String> {
        SearchProgress {
            matches: self.paths.clone(),
            truncated: self.left_out,
            done,
        }
    }
}

/// Shallow before deep, and alphabetical within a depth.
fn rank_of(path: &str) -> (usize, &str) {
    (path.matches('/').count(), path)
}

/// The regex `ag` is given for a typed query.
///
/// Everything typed is matched literally. Spaces are the one exception: they stand for "and
/// then, further along the path", which is what makes `nat pal` find `src/native/palette.rs`.
fn pattern_for(query: &str) -> String {
    query
        .split_whitespace()
        .map(search::escape_regex)
        .collect::<Vec<_>>()
        .join(".*")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::LastReport;

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

    #[test]
    fn the_nearest_the_root_are_kept_in_order_and_the_rest_left_out() {
        let mut nearest = Nearest::holding(2);

        assert!(nearest.offer("src/deep/first.rs".to_string()));
        assert!(nearest.offer("b.rs".to_string()));
        assert!(!nearest.left_out);
        // Nearer the root than the deep one, which is pushed off the end.
        assert!(nearest.offer("a.rs".to_string()));
        assert_eq!(nearest.paths, vec!["a.rs", "b.rs"]);
        assert!(nearest.left_out);
        // Deeper than everything kept: nothing changes.
        assert!(!nearest.offer("src/other.rs".to_string()));
        assert_eq!(nearest.paths, vec!["a.rs", "b.rs"]);
    }

    /// The finder lists the files of the repo, and the ones its `.gitignore` leaves out only
    /// in the scope that asks for them. `node_modules` is in neither.
    #[test]
    fn the_ignored_files_are_listed_only_when_asked_for() {
        let repo = search::repo_with_an_ignored_file("names");

        let mut of_the_repo = LastReport::wanting();
        stream_matching_paths(&repo, "rs", SearchScope::RepoFiles, &mut of_the_repo).unwrap();
        assert_eq!(of_the_repo.done().matches, vec!["src/kept.rs".to_string()]);

        let mut with_ignored = LastReport::wanting();
        stream_matching_paths(
            &repo,
            "rs",
            SearchScope::IncludingIgnored,
            &mut with_ignored,
        )
        .unwrap();
        assert_eq!(
            with_ignored.done().matches,
            vec!["build/left.rs".to_string(), "src/kept.rs".to_string()]
        );
    }

    /// A search nobody wants is stopped, and reports nothing - not even that it is done.
    #[test]
    fn a_search_nobody_wants_reports_nothing() {
        let repo = search::repo_with_an_ignored_file("unwanted");

        let mut nobody = LastReport::<String>::wanting();
        nobody.wanted = false;
        stream_matching_paths(&repo, "rs", SearchScope::RepoFiles, &mut nobody).unwrap();

        assert_eq!(nobody.reports, 0);
    }
}
