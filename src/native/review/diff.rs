//! Turning a patch into the lines the review draws: what kind each line is, the old and new
//! line numbers it carries, and which words actually changed.
//!
//! Ported from `web/src/components/hunks/HunkDiffCode.tsx` and `wordDiff.tsx` so a hunk
//! reads identically in both frontends.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LineKind {
    Header,
    Added,
    Removed,
    Context,
    /// File headers and `\ No newline at end of file`: shown, but not part of the change.
    Other,
}

impl LineKind {
    /// Whether a comment or a partial stage can be anchored to this line.
    pub(crate) fn commentable(self) -> bool {
        matches!(self, Self::Added | Self::Removed | Self::Context)
    }

    pub(crate) fn prefix(self) -> char {
        match self {
            Self::Added => '+',
            Self::Removed => '-',
            _ => ' ',
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DiffLine {
    /// The raw patch line, prefix included. This is what a selection is matched against.
    pub(crate) text: String,
    pub(crate) old_line_number: Option<usize>,
    pub(crate) new_line_number: Option<usize>,
    pub(crate) kind: LineKind,
    /// Word-level runs, present only on a removed line paired with the added line below it.
    pub(crate) words: Option<Vec<WordPart>>,
}

/// Prefixes of the git bookkeeping a patch carries before its first `@@`.
const PATCH_METADATA_PREFIXES: &[&str] = &[
    "diff --git ",
    "index ",
    "--- ",
    "+++ ",
    "old mode ",
    "new mode ",
    "new file mode ",
    "deleted file mode ",
    "similarity index ",
    "dissimilarity index ",
    "rename from ",
    "rename to ",
    "copy from ",
    "copy to ",
    "Binary files ",
    "GIT binary patch",
];

impl DiffLine {
    /// Whether this line is git bookkeeping rather than part of the change.
    ///
    /// The review shows the file path in its own heading and the `@@` header in the hunk's
    /// toolbar, so repeating either inside the code would only push the diff down the page.
    pub(crate) fn is_chrome(&self) -> bool {
        match self.kind {
            LineKind::Header => true,
            LineKind::Other => PATCH_METADATA_PREFIXES
                .iter()
                .any(|prefix| self.text.starts_with(prefix)),
            _ => false,
        }
    }

    /// The line without its diff prefix, which is the code the user reads.
    pub(crate) fn body(&self) -> &str {
        match self.kind {
            LineKind::Added | LineKind::Removed | LineKind::Context => {
                self.text.get(1..).unwrap_or("")
            }
            _ => &self.text,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct WordPart {
    pub(crate) text: String,
    pub(crate) changed: bool,
}

pub(crate) fn build_diff_lines(patch: &str) -> Vec<DiffLine> {
    let mut old_line_number: Option<usize> = None;
    let mut new_line_number: Option<usize> = None;
    let mut lines = Vec::new();

    for text in patch.split('\n') {
        let line = if text.starts_with("@@") {
            let (old_start, new_start) = parse_hunk_header(text);
            old_line_number = old_start;
            new_line_number = new_start;
            DiffLine {
                text: text.to_string(),
                old_line_number: None,
                new_line_number: None,
                kind: LineKind::Header,
                words: None,
            }
        } else if text.starts_with('+') && !text.starts_with("+++") {
            let line = DiffLine {
                text: text.to_string(),
                old_line_number: None,
                new_line_number,
                kind: LineKind::Added,
                words: None,
            };
            new_line_number = new_line_number.map(|number| number + 1);
            line
        } else if text.starts_with('-') && !text.starts_with("---") {
            let line = DiffLine {
                text: text.to_string(),
                old_line_number,
                new_line_number: None,
                kind: LineKind::Removed,
                words: None,
            };
            old_line_number = old_line_number.map(|number| number + 1);
            line
        } else if text.starts_with(' ') {
            let line = DiffLine {
                text: text.to_string(),
                old_line_number,
                new_line_number,
                kind: LineKind::Context,
                words: None,
            };
            old_line_number = old_line_number.map(|number| number + 1);
            new_line_number = new_line_number.map(|number| number + 1);
            line
        } else {
            DiffLine {
                text: text.to_string(),
                old_line_number: None,
                new_line_number: None,
                kind: LineKind::Other,
                words: None,
            }
        };

        lines.push(line);
    }

    attach_word_diffs(&mut lines);
    lines
}

/// A removed line directly above an added line is almost always an edit of that line, so
/// those two get compared word by word.
fn attach_word_diffs(lines: &mut [DiffLine]) {
    for index in 0..lines.len().saturating_sub(1) {
        if lines[index].kind != LineKind::Removed || lines[index + 1].kind != LineKind::Added {
            continue;
        }
        let (old_parts, new_parts) =
            word_diff_parts(lines[index].body(), lines[index + 1].body());
        lines[index].words = Some(old_parts);
        lines[index + 1].words = Some(new_parts);
    }
}

fn parse_hunk_header(header: &str) -> (Option<usize>, Option<usize>) {
    let Some(ranges) = header.split("@@").nth(1) else {
        return (None, None);
    };
    let mut parts = ranges.trim().split_whitespace();
    let old = parts
        .next()
        .and_then(|part| range_start(part.trim_start_matches('-')));
    let new = parts
        .next()
        .and_then(|part| range_start(part.trim_start_matches('+')));
    (old, new)
}

fn range_start(value: &str) -> Option<usize> {
    value
        .split(',')
        .next()
        .and_then(|start| start.parse().ok())
}

/// Identifiers, runs of whitespace, and single punctuation characters. Splitting this way
/// keeps a renamed symbol as one token instead of a smear of characters.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_alphanumeric() || ch == '_' {
            let mut token = String::from(ch);
            while let Some(next) = chars.peek() {
                if next.is_alphanumeric() || *next == '_' {
                    token.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(token);
        } else if ch.is_whitespace() {
            let mut token = String::from(ch);
            while let Some(next) = chars.peek() {
                if next.is_whitespace() {
                    token.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(token);
        } else {
            tokens.push(ch.to_string());
        }
    }

    tokens
}

/// The tokens on each side that are *not* part of the longest common subsequence.
fn changed_token_indexes(old: &[String], new: &[String]) -> (Vec<bool>, Vec<bool>) {
    let rows = old.len() + 1;
    let cols = new.len() + 1;
    let mut lengths = vec![0usize; rows * cols];

    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            let at = old_index * cols + new_index;
            lengths[at] = if old[old_index] == new[new_index] {
                lengths[(old_index + 1) * cols + new_index + 1] + 1
            } else {
                lengths[(old_index + 1) * cols + new_index]
                    .max(lengths[old_index * cols + new_index + 1])
            };
        }
    }

    let mut old_changed = vec![true; old.len()];
    let mut new_changed = vec![true; new.len()];
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < old.len() && new_index < new.len() {
        if old[old_index] == new[new_index] {
            old_changed[old_index] = false;
            new_changed[new_index] = false;
            old_index += 1;
            new_index += 1;
        } else if lengths[(old_index + 1) * cols + new_index]
            >= lengths[old_index * cols + new_index + 1]
        {
            old_index += 1;
        } else {
            new_index += 1;
        }
    }

    (old_changed, new_changed)
}

/// How long a line can get before word diffing it stops being worth the quadratic table.
/// Minified bundles and generated files hit this; a plain line-level diff still shows.
const MAX_WORD_DIFF_TOKENS: usize = 400;

pub(crate) fn word_diff_parts(old_line: &str, new_line: &str) -> (Vec<WordPart>, Vec<WordPart>) {
    let old_tokens = tokenize(old_line);
    let new_tokens = tokenize(new_line);
    if old_tokens.len() > MAX_WORD_DIFF_TOKENS || new_tokens.len() > MAX_WORD_DIFF_TOKENS {
        return (
            vec![WordPart {
                text: old_line.to_string(),
                changed: false,
            }],
            vec![WordPart {
                text: new_line.to_string(),
                changed: false,
            }],
        );
    }

    let (old_changed, new_changed) = changed_token_indexes(&old_tokens, &new_tokens);
    (
        merge_adjacent(&old_tokens, &old_changed),
        merge_adjacent(&new_tokens, &new_changed),
    )
}

fn merge_adjacent(tokens: &[String], changed: &[bool]) -> Vec<WordPart> {
    let mut merged: Vec<WordPart> = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        // Whitespace is never highlighted on its own: a shifted indent would otherwise light
        // up the whole line.
        let is_changed = changed.get(index).copied().unwrap_or(false)
            && !token.chars().all(char::is_whitespace);
        match merged.last_mut() {
            Some(previous) if previous.changed == is_changed => previous.text.push_str(token),
            _ => merged.push(WordPart {
                text: token.clone(),
                changed: is_changed,
            }),
        }
    }

    merged
}

/// Where an anchored comment belongs in a hunk: after the first line that contains the
/// first non-blank line of what it was anchored to.
pub(crate) fn insertion_line(lines: &[DiffLine], selection: &str, used: &[usize]) -> Option<usize> {
    let needle = selection
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;

    lines
        .iter()
        .enumerate()
        .position(|(index, line)| !used.contains(&index) && line.text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context line starts with a space, so this cannot use `\`-continuations: those strip
    /// the leading whitespace of the next line along with the newline.
    const PATCH: &str = concat!(
        "diff --git a/src/lib.rs b/src/lib.rs\n",
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -10,4 +10,5 @@\n",
        " fn keep() {}\n",
        "-fn old_name(a: u8) {}\n",
        "+fn new_name(a: u8) {}\n",
        "+fn extra() {}\n",
        " fn tail() {}",
    );

    #[test]
    fn line_numbers_advance_by_side() {
        let lines = build_diff_lines(PATCH);
        let context = lines
            .iter()
            .find(|line| line.body() == "fn keep() {}")
            .expect("expected the first context line");
        assert_eq!(context.old_line_number, Some(10));
        assert_eq!(context.new_line_number, Some(10));

        let removed = lines
            .iter()
            .find(|line| line.kind == LineKind::Removed)
            .expect("expected a removed line");
        assert_eq!(removed.old_line_number, Some(11));
        assert_eq!(removed.new_line_number, None);

        let tail = lines
            .iter()
            .find(|line| line.body() == "fn tail() {}")
            .expect("expected the trailing context line");
        assert_eq!(tail.old_line_number, Some(12));
        assert_eq!(tail.new_line_number, Some(13));
    }

    #[test]
    fn file_headers_are_not_commentable() {
        let lines = build_diff_lines(PATCH);
        assert!(!lines[0].kind.commentable());
        assert_eq!(lines[1].kind, LineKind::Other);
        assert_eq!(lines[2].kind, LineKind::Other);
        assert_eq!(lines[3].kind, LineKind::Header);
    }

    #[test]
    fn git_bookkeeping_is_chrome_and_the_code_is_not() {
        let lines = build_diff_lines(PATCH);

        let hidden: Vec<&str> = lines
            .iter()
            .filter(|line| line.is_chrome())
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(
            hidden,
            vec![
                "diff --git a/src/lib.rs b/src/lib.rs",
                "--- a/src/lib.rs",
                "+++ b/src/lib.rs",
                "@@ -10,4 +10,5 @@",
            ]
        );
        assert!(
            lines
                .iter()
                .filter(|line| !line.is_chrome())
                .all(|line| line.kind.commentable()),
            "everything left should be part of the change"
        );
    }

    #[test]
    fn a_missing_newline_marker_is_kept_visible() {
        let lines = build_diff_lines("@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file");

        let marker = lines.last().expect("expected a trailing line");
        assert_eq!(marker.kind, LineKind::Other);
        assert!(
            !marker.is_chrome(),
            "the missing-newline marker says something about the change"
        );
    }

    #[test]
    fn a_replaced_line_gets_word_level_parts() {
        let lines = build_diff_lines(PATCH);
        let removed = lines
            .iter()
            .find(|line| line.kind == LineKind::Removed)
            .expect("expected a removed line");
        let words = removed.words.as_ref().expect("expected word parts");

        let changed: Vec<&str> = words
            .iter()
            .filter(|part| part.changed)
            .map(|part| part.text.as_str())
            .collect();
        assert_eq!(changed, vec!["old_name"]);
    }

    #[test]
    fn an_added_line_with_no_partner_has_no_word_parts() {
        let lines = build_diff_lines(PATCH);
        let extra = lines
            .iter()
            .find(|line| line.body() == "fn extra() {}")
            .expect("expected the added line");
        assert!(extra.words.is_none());
    }

    #[test]
    fn shifted_indentation_alone_is_not_highlighted() {
        let (old_parts, new_parts) = word_diff_parts("  value", "    value");

        assert!(old_parts.iter().all(|part| !part.changed));
        assert!(new_parts.iter().all(|part| !part.changed));
    }

    #[test]
    fn very_long_lines_fall_back_to_a_line_level_diff() {
        let long = "a ".repeat(MAX_WORD_DIFF_TOKENS);
        let (old_parts, _) = word_diff_parts(&long, &format!("{long}b"));

        assert_eq!(old_parts.len(), 1);
        assert!(!old_parts[0].changed);
    }

    #[test]
    fn a_comment_anchors_after_the_line_it_quotes() {
        let lines = build_diff_lines(PATCH);

        let at = insertion_line(&lines, "+fn extra() {}", &[]).expect("expected a line");

        assert_eq!(lines[at].body(), "fn extra() {}");
    }

    #[test]
    fn a_second_comment_on_the_same_text_takes_the_next_match() {
        let lines = build_diff_lines("@@ -1,2 +1,2 @@\n+same\n+same");

        let first = insertion_line(&lines, "same", &[]).expect("expected a first line");
        let second = insertion_line(&lines, "same", &[first]).expect("expected a second line");

        assert_ne!(first, second);
    }
}
