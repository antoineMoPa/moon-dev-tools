//! The commented lines as a patch of their own, which is how a comment reaches an agent already
//! running on the task: a file, an `@@` line with the real line numbers, and the selection with
//! a little of the code around it.

use std::ops::RangeInclusive;

use crate::native::review::diff::{DiffLine, LineKind};

/// How many lines of the patch either side of the selection go with it, which is git's own
/// default.
const CONTEXT_LINES: usize = 3;

/// `None` when nothing in the range can be commented on.
pub(super) fn patch_excerpt(
    file_path: &str,
    lines: &[DiffLine],
    selected: RangeInclusive<usize>,
) -> Option<String> {
    let in_a_hunk = |line: &DiffLine| line.kind.commentable();
    let first_selected = selected
        .clone()
        .find(|&index| lines.get(index).is_some_and(in_a_hunk))?;
    let last_selected = selected
        .rev()
        .find(|&index| lines.get(index).is_some_and(in_a_hunk))?;

    // Context stops at whatever is not code of this hunk, so it never crosses an `@@`.
    let context_before = lines[..first_selected]
        .iter()
        .rev()
        .take(CONTEXT_LINES)
        .take_while(|line| in_a_hunk(line))
        .count();
    let context_after = lines[last_selected + 1..]
        .iter()
        .take(CONTEXT_LINES)
        .take_while(|line| in_a_hunk(line))
        .count();
    let start = first_selected - context_before;
    let shown: Vec<&DiffLine> = lines[start..=last_selected + context_after]
        .iter()
        .filter(|line| in_a_hunk(line))
        .collect();

    let before = &lines[..start];
    let old_range = range_of(&shown, before, |line| line.old_line_number);
    let new_range = range_of(&shown, before, |line| line.new_line_number);
    let section = before
        .iter()
        .rev()
        .find(|line| line.kind == LineKind::Header)
        .and_then(|line| line.text.splitn(3, "@@").nth(2))
        .unwrap_or("");

    let mut excerpt = format!(
        "--- a/{file_path}\n+++ b/{file_path}\n@@ -{old_range} +{new_range} @@{}",
        section.trim_end()
    );
    for line in shown {
        excerpt.push('\n');
        excerpt.push_str(&line.text);
    }
    Some(excerpt)
}

/// One side of an `@@` line: where the lines start and how many there are. A side with no
/// lines names the line before the change, as git does.
fn range_of(
    shown: &[&DiffLine],
    before: &[DiffLine],
    number_of: impl Fn(&DiffLine) -> Option<usize>,
) -> String {
    let numbers = shown.iter().filter_map(|line| number_of(line));
    let count = numbers.clone().count();
    let start = numbers
        .clone()
        .next()
        .or_else(|| before.iter().rev().find_map(&number_of))
        .unwrap_or(0);
    format!("{start},{count}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::review::diff::build_diff_lines;

    const PATCH: &str = "@@ -10,7 +10,8 @@ fn draw_rows(\n one\n two\n three\n-old\n+new\n+added\n four\n five\n six\n@@ -40,2 +41,2 @@ fn other(\n far\n-gone\n";

    fn index_of(lines: &[DiffLine], text: &str) -> usize {
        lines
            .iter()
            .position(|line| line.text == text)
            .expect("the line is in the patch")
    }

    #[test]
    fn a_selected_line_comes_with_three_lines_either_side_and_real_numbers() {
        let lines = build_diff_lines(PATCH);
        let selected = index_of(&lines, "+new");

        assert_eq!(
            patch_excerpt("src/rows.rs", &lines, selected..=selected).as_deref(),
            Some(
                "--- a/src/rows.rs\n+++ b/src/rows.rs\n@@ -11,5 +11,6 @@ fn draw_rows(\n two\n three\n-old\n+new\n+added\n four\n five"
            )
        );
    }

    #[test]
    fn context_stops_at_the_edges_of_the_hunk() {
        let lines = build_diff_lines(PATCH);
        let selected = index_of(&lines, " six");

        let excerpt = patch_excerpt("a.rs", &lines, selected..=selected).expect("a hunk line");

        assert!(excerpt.contains("@@ -14,3 +14,4 @@"), "{excerpt}");
        assert!(excerpt.ends_with(" six"), "{excerpt}");
    }

    #[test]
    fn nothing_commentable_has_no_excerpt() {
        let lines = build_diff_lines(PATCH);
        assert_eq!(patch_excerpt("a.rs", &lines, 0..=0), None);
    }
}
