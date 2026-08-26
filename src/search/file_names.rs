//! Finding a file of the repo by name.
//!
//! `ag -g` lists the paths that match a pattern, which is a name search over the same set of
//! files a content search reads.

use std::path::Path;

use anyhow::Result;

use crate::{api::FileMatchesPayload, search};

/// How many paths the search hands back. A query loose enough to match more than this is one
/// the user is still typing; the payload says it was cut short so the palette can too.
const MATCH_LIMIT: usize = 200;

/// Every path in the repo whose name matches `query`, nearest the root first.
pub(crate) fn matching_paths(repo_path: &Path, query: &str) -> Result<FileMatchesPayload> {
    let pattern = pattern_for(query);
    let Some(output) = search::search(repo_path, &["-g", &pattern])? else {
        return Ok(FileMatchesPayload::default());
    };

    let mut paths: Vec<String> = output
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
