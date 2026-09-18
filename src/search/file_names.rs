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
        // A rank carries the whole path, so an exact hit is the same path over again - a
        // path is listed once however many times the searcher prints it.
        let at = match self.paths.binary_search_by(|kept| rank_of(kept).cmp(&rank)) {
            Ok(_) => return false,
            Err(at) => at,
        };
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

/// What a typed query asks for, which is read off the query itself.
enum Query<'a> {
    /// `/…/` - what stands between the slashes is a regex, handed to the searcher as it was
    /// typed, for the search no other form reaches.
    Regex(&'a str),
    /// A query with a `*` or a `?` in it: a wildcard pattern that starts where a directory
    /// or a file name starts and runs to the end of the path. `git*.rs` is every Rust file
    /// under a `git` directory or named `git…`, and `*.rs` is every Rust file.
    Wildcard(&'a str),
    /// Anything else: the terms matched literally, in order, anywhere along the path.
    Terms(&'a str),
}

fn query_of(query: &str) -> Query<'_> {
    let typed = query.trim();
    if let Some(regex) = typed
        .strip_prefix('/')
        .and_then(|rest| rest.strip_suffix('/'))
        && !regex.is_empty()
    {
        return Query::Regex(regex);
    }
    if typed.contains(['*', '?']) {
        return Query::Wildcard(typed);
    }
    Query::Terms(query)
}

/// Where a name starts in a path as `ag` weighs it against a `-g` pattern: after a `/`, or at
/// the very front - which is `./`, though what it prints is not: the search is run in the
/// repo with no path of its own, so every path it weighs is `./` and then the path as the
/// rows show it.
const NAME_START: &str = "(^\\./|/)";

/// The regex `ag` is given for a typed query - see [`Query`] for the three it can be.
fn pattern_for(query: &str) -> String {
    match query_of(query) {
        Query::Regex(regex) => regex.to_string(),
        // Anchored at the end, so `*.rs` does not also match a path that merely has `.rs`
        // somewhere in the middle of it; and at the start of a name, so `git*.rs` does not
        // also match `digit.rs`.
        Query::Wildcard(wildcard) => {
            format!("{NAME_START}{}$", wildcard_regex(wildcard))
        }
        Query::Terms(terms) => terms
            .split_whitespace()
            .map(search::escape_regex)
            .collect::<Vec<_>>()
            .join(".*"),
    }
}

/// A wildcard pattern as a regex: `*` stands for any run of characters, `/` included, and
/// `?` for one of them. Everything else is matched as it was typed, the `.` of an extension
/// included.
fn wildcard_regex(wildcard: &str) -> String {
    let mut pattern = String::with_capacity(wildcard.len() * 2);
    for character in wildcard.chars() {
        match character {
            '*' => pattern.push_str(".*"),
            '?' => pattern.push('.'),
            _ => search::escape_char_into(character, &mut pattern),
        }
    }
    pattern
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

    /// A `*` or a `?` turns the query into a pattern from the start of a name to the end of
    /// the path, with everything else still matched as it was typed.
    #[test]
    fn a_wildcard_query_runs_from_a_name_to_the_end_of_the_path() {
        assert_eq!(pattern_for("*.rs"), "(^\\./|/).*\\.rs$");
        assert_eq!(pattern_for("src/*/mod.rs"), "(^\\./|/)src/.*/mod\\.rs$");
        assert_eq!(pattern_for("mod.?s"), "(^\\./|/)mod\\..s$");
    }

    #[test]
    fn a_query_between_slashes_is_a_regex_as_typed() {
        assert_eq!(pattern_for("/^src/.*[.]rs$/"), "^src/.*[.]rs$");
        // Nothing between them is not a regex, and neither is one slash.
        assert_eq!(pattern_for("//"), "//");
        assert_eq!(pattern_for("/"), "/");
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

    /// A path printed twice is listed once: the rows are what the repo holds, and the same
    /// file twice over is a row nobody can tell from the one above it.
    #[test]
    fn a_path_offered_twice_is_kept_once() {
        let mut nearest = Nearest::holding(4);

        assert!(nearest.offer("src/a.rs".to_string()));
        assert!(!nearest.offer("src/a.rs".to_string()));
        assert_eq!(nearest.paths, vec!["src/a.rs"]);
        assert!(
            !nearest.left_out,
            "the same path again is not a path left out"
        );
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

    /// The wildcard reaches the searcher, not only the pattern builder: `src/*.rs` is the
    /// Rust files of `src`, and `kept*` is the file named that, however deep it is.
    #[test]
    fn a_wildcard_query_finds_the_paths_of_that_shape() {
        let repo = search::repo_with_an_ignored_file("wildcards");

        let mut under_src = LastReport::wanting();
        stream_matching_paths(&repo, "src/*.rs", SearchScope::RepoFiles, &mut under_src).unwrap();
        assert_eq!(under_src.done().matches, vec!["src/kept.rs".to_string()]);

        let mut by_name = LastReport::wanting();
        stream_matching_paths(&repo, "kept*", SearchScope::RepoFiles, &mut by_name).unwrap();
        assert_eq!(by_name.done().matches, vec!["src/kept.rs".to_string()]);

        // From the start of a name, not from anywhere inside one.
        let mut inside_a_name = LastReport::wanting();
        stream_matching_paths(&repo, "ept*", SearchScope::RepoFiles, &mut inside_a_name).unwrap();
        assert!(inside_a_name.done().matches.is_empty());
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
