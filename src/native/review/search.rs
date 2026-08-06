//! Finding text across every hunk of a review.
//!
//! The diff pane only lays out the hunks that are on screen, so a search cannot be a matter
//! of looking at what was drawn. It goes through the parsed lines of every hunk instead —
//! the same parse the pane draws from, taken out of the app's cache — which is what makes a
//! match in a file scrolled far out of sight findable at all.

use crate::native::{app::App, review::diff::DiffLine};

/// Where the query turned up: the hunk, the line of it, and the columns of that line's body.
///
/// Columns count characters of the body, with the `+`/`-` marker already taken off, because
/// that is the text the pane draws and therefore the text a highlight has to line up with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Match {
    pub(crate) hunk_id: String,
    pub(crate) line_index: usize,
    pub(crate) column: usize,
    pub(crate) width: usize,
}

/// Every occurrence of the query, in the order the review shows them, matched without regard
/// for case.
pub(crate) fn find_all(app: &mut App, session_id: &str, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    // The patch each hunk is drawn from: its expansion if it has one, its preview otherwise,
    // so a search sees exactly what the pane would show.
    let Some(review) = app.model.review_ref(session_id) else {
        return Vec::new();
    };
    let patches: Vec<(String, String)> = review
        .hunks()
        .iter()
        .map(|hunk| {
            let patch = review
                .expanded_patches
                .get(&hunk.id)
                .cloned()
                .unwrap_or_else(|| hunk.patch_preview.clone());
            (hunk.id.clone(), patch)
        })
        .collect();

    let needle = query.to_lowercase();
    let mut found = Vec::new();
    for (hunk_id, patch) in patches {
        let lines = app.diff_lines(&hunk_id, &patch);
        for (line_index, line) in lines.iter().enumerate() {
            if line.is_chrome() {
                continue;
            }
            find_in_line(line, &needle, &hunk_id, line_index, &mut found);
        }
    }
    found
}

fn find_in_line(
    line: &DiffLine,
    needle: &str,
    hunk_id: &str,
    line_index: usize,
    out: &mut Vec<Match>,
) {
    let body: Vec<char> = line.body().to_lowercase().chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || body.len() < needle.len() {
        return;
    }
    for column in 0..=body.len() - needle.len() {
        if body[column..column + needle.len()] == needle[..] {
            out.push(Match {
                hunk_id: hunk_id.to_string(),
                line_index,
                column,
                width: needle.len(),
            });
        }
    }
}

/// The columns of a line the query covers, for the pane to draw behind the text. Worked out
/// per line while drawing rather than looked up, because only the lines on screen need it.
pub(crate) fn spans_in(line: &DiffLine, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    find_in_line(line, &query.to_lowercase(), "", 0, &mut found);
    found
        .into_iter()
        .map(|found| (found.column, found.width))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::review::diff::build_diff_lines;

    const PATCH: &str = "@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"one\");\n+    println!(\"two\");\n+    println!(\"Two more\");\n }\n";

    fn lines() -> Vec<DiffLine> {
        build_diff_lines(PATCH)
    }

    #[test]
    fn a_span_covers_the_query_where_it_sits_in_the_body() {
        let lines = lines();
        let line = lines
            .iter()
            .find(|line| line.body().contains("one"))
            .expect("expected the removed line");

        let spans = spans_in(line, "one");
        assert_eq!(spans.len(), 1);
        let (column, width) = spans[0];
        assert_eq!(width, 3);
        // The marker is not part of the body, so the column is an index into the code.
        assert_eq!(
            line.body().chars().skip(column).take(width).collect::<String>(),
            "one"
        );
    }

    #[test]
    fn case_is_not_what_a_search_is_about() {
        let lines = lines();
        let line = lines
            .iter()
            .find(|line| line.body().contains("Two more"))
            .expect("expected the second added line");

        assert_eq!(spans_in(line, "two").len(), 1);
        assert_eq!(spans_in(line, "TWO").len(), 1);
    }

    #[test]
    fn a_query_that_is_not_there_spans_nothing() {
        let lines = lines();

        assert!(lines.iter().all(|line| spans_in(line, "absent").is_empty()));
        assert!(lines.iter().all(|line| spans_in(line, "").is_empty()));
    }

    #[test]
    fn every_occurrence_on_a_line_gets_its_own_span() {
        let line = build_diff_lines("@@ -1 +1 @@\n+one one one\n")
            .into_iter()
            .find(|line| line.body().contains("one"))
            .expect("expected the added line");

        assert_eq!(spans_in(&line, "one").len(), 3);
    }
}
