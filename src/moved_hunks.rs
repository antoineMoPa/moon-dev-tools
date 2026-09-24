use std::collections::{HashMap, HashSet};

use crate::api::{DiffHunk, HunkMoveHint};

const MIN_CHANGED_LINES: usize = 6;
const MIN_MOVE_SCORE: f64 = 0.58;

/// The lines a hunk removed or added, and the code tokens in them, each counted once. Every
/// removed side of a diff is scored against every added side, which in a diff of hundreds of
/// hunks is tens of thousands of pairs - so what a pair costs is two multiset intersections,
/// and nothing is counted again per pair.
struct Candidate {
    hunk_index: usize,
    lines: Bag,
    tokens: Bag,
}

#[derive(Clone)]
struct Match {
    removed_index: usize,
    added_index: usize,
    score: f64,
}

pub(crate) struct HunkMoveHints {
    pub(crate) moved_from: HashMap<String, HunkMoveHint>,
    pub(crate) moved_to: HashMap<String, HunkMoveHint>,
}

pub(crate) fn detect_hunk_moves(hunks: &[DiffHunk]) -> HunkMoveHints {
    let removed = hunks
        .iter()
        .enumerate()
        .filter_map(|(hunk_index, hunk)| candidate(hunk_index, removed_lines(&hunk.patch)))
        .collect::<Vec<_>>();
    let added = hunks
        .iter()
        .enumerate()
        .filter_map(|(hunk_index, hunk)| candidate(hunk_index, added_lines(&hunk.patch)))
        .collect::<Vec<_>>();

    let mut matches = Vec::new();
    for old in &removed {
        for new in &added {
            if old.hunk_index == new.hunk_index || !could_be_a_move(old, new) {
                continue;
            }

            let score = similarity_score(old, new);
            if score >= MIN_MOVE_SCORE {
                matches.push(Match {
                    removed_index: old.hunk_index,
                    added_index: new.hunk_index,
                    score,
                });
            }
        }
    }

    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut used_removed = HashSet::new();
    let mut used_added = HashSet::new();
    let mut moved_from = HashMap::new();
    let mut moved_to = HashMap::new();

    for matched in matches {
        if !used_removed.insert(matched.removed_index) || !used_added.insert(matched.added_index) {
            continue;
        }

        let source = &hunks[matched.removed_index];
        let target = &hunks[matched.added_index];
        moved_from.insert(target.id.clone(), hint_for(source, matched.score));
        moved_to.insert(source.id.clone(), hint_for(target, matched.score));
    }

    HunkMoveHints {
        moved_from,
        moved_to,
    }
}

/// How alike two sides are: the Jaccard similarity of their lines, or the Sørensen-Dice
/// similarity of their tokens, whichever is higher. (Dice is never below Jaccard on the same
/// two multisets, so the tokens' Jaccard would add nothing.)
fn similarity_score(old: &Candidate, new: &Candidate) -> f64 {
    old.lines
        .jaccard(&new.lines)
        .max(old.tokens.sorensen_dice(&new.tokens))
}

/// Whether the two could score [`MIN_MOVE_SCORE`] at all, told from their sizes alone: two
/// sides can share no more than the smaller has, which caps both similarities. Most pairs in
/// a large diff are nothing alike in size, and are dropped here without an intersection.
fn could_be_a_move(old: &Candidate, new: &Candidate) -> bool {
    old.lines.most_jaccard_with(&new.lines) >= MIN_MOVE_SCORE
        || old.tokens.most_sorensen_dice_with(&new.tokens) >= MIN_MOVE_SCORE
}

fn candidate(hunk_index: usize, lines: Vec<String>) -> Option<Candidate> {
    if lines.len() < MIN_CHANGED_LINES {
        return None;
    }

    let tokens = token_fingerprints(&lines);
    if lines.is_empty() && tokens.is_empty() {
        return None;
    }

    Some(Candidate {
        hunk_index,
        lines: Bag::of(lines),
        tokens: Bag::of(tokens),
    })
}

/// A multiset: each distinct item with how many times it was there.
struct Bag {
    counts: HashMap<String, usize>,
    /// How many items went in, repeats included.
    size: usize,
}

impl Bag {
    fn of(items: Vec<String>) -> Self {
        let size = items.len();
        let mut counts = HashMap::new();
        for item in items {
            *counts.entry(item).or_insert(0) += 1;
        }
        Self { counts, size }
    }

    /// How many items the two have in common - an item in both three and five times counts
    /// three. Walks the smaller of the two.
    fn shared_with(&self, other: &Bag) -> usize {
        let (fewer, more) = if self.counts.len() <= other.counts.len() {
            (self, other)
        } else {
            (other, self)
        };
        fewer
            .counts
            .iter()
            .map(|(item, count)| {
                more.counts
                    .get(item)
                    .map_or(0, |theirs| (*count).min(*theirs))
            })
            .sum()
    }

    /// Shared over the union, repeats included. Two empty bags are alike.
    fn jaccard(&self, other: &Bag) -> f64 {
        let shared = self.shared_with(other);
        let union = self.size + other.size - shared;
        if union == 0 {
            1.0
        } else {
            shared as f64 / union as f64
        }
    }

    /// Twice the shared over both sizes. Two empty bags are alike.
    fn sorensen_dice(&self, other: &Bag) -> f64 {
        let total = self.size + other.size;
        if total == 0 {
            1.0
        } else {
            (2 * self.shared_with(other)) as f64 / total as f64
        }
    }

    /// The most [`jaccard`](Bag::jaccard) could answer, with the shared count at its largest:
    /// the smaller size.
    fn most_jaccard_with(&self, other: &Bag) -> f64 {
        let (smaller, larger) = (self.size.min(other.size), self.size.max(other.size));
        if larger == 0 {
            1.0
        } else {
            smaller as f64 / larger as f64
        }
    }

    /// The most [`sorensen_dice`](Bag::sorensen_dice) could answer, the same way.
    fn most_sorensen_dice_with(&self, other: &Bag) -> f64 {
        let total = self.size + other.size;
        if total == 0 {
            1.0
        } else {
            (2 * self.size.min(other.size)) as f64 / total as f64
        }
    }
}

fn hint_for(hunk: &DiffHunk, score: f64) -> HunkMoveHint {
    HunkMoveHint {
        target_hunk_id: hunk.id.clone(),
        target_file_path: hunk.file_path.clone(),
        target_header: hunk.header.clone(),
        score,
    }
}

fn token_fingerprints(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| split_code_tokens(line))
        .collect::<Vec<_>>()
}

fn split_code_tokens(line: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(line.len());
    let mut previous: Option<char> = None;

    for current in line.chars() {
        if let Some(previous) = previous {
            if should_split_camel_case(previous, current) {
                normalized.push(' ');
            }
        }

        if current.is_ascii_alphanumeric() {
            normalized.push(current.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
        previous = Some(current);
    }

    normalized
        .split_whitespace()
        .filter(|token| token.len() > 1)
        .map(ToOwned::to_owned)
        .collect()
}

fn should_split_camel_case(previous: char, current: char) -> bool {
    (previous.is_ascii_lowercase() || previous.is_ascii_digit()) && current.is_ascii_uppercase()
}

fn removed_lines(patch: &str) -> Vec<String> {
    changed_lines(patch, '-', "---")
}

fn added_lines(patch: &str) -> Vec<String> {
    changed_lines(patch, '+', "+++")
}

fn changed_lines(patch: &str, prefix: char, metadata_prefix: &str) -> Vec<String> {
    patch
        .lines()
        .filter(|line| line.starts_with(prefix) && !line.starts_with(metadata_prefix))
        .filter_map(|line| normalize_changed_line(&line[1..]))
        .collect()
}

fn normalize_changed_line(line: &str) -> Option<String> {
    let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FileChangeKind, stable_id};

    fn hunk(id_hint: &str, file_path: &str, header: &str, body: &[&str]) -> DiffHunk {
        let patch = format!(
            "diff --git a/{file_path} b/{file_path}\n--- a/{file_path}\n+++ b/{file_path}\n{header}\n{}\n",
            body.join("\n")
        );
        DiffHunk {
            id: stable_id(&(id_hint, file_path, header)),
            file_path: file_path.to_string(),
            change_kind: FileChangeKind::Modified,
            header: header.to_string(),
            patch,
            staged: false,
            image_diff: None,
        }
    }

    #[test]
    fn links_removed_and_added_hunks_with_similar_content() {
        let source = hunk(
            "source",
            "src/old.rs",
            "@@ -10,8 +10,0 @@",
            &[
                "-fn moved() {",
                "-    let first = 1;",
                "-    let second = 2;",
                "-    let third = 3;",
                "-    let fourth = 4;",
                "-    println!(\"{}\", first + second + third + fourth);",
                "-}",
            ],
        );
        let target = hunk(
            "target",
            "src/new.rs",
            "@@ -30,0 +30,8 @@",
            &[
                "+fn moved() {",
                "+    let first = 1;",
                "+    let second = 2;",
                "+    let third = 30;",
                "+    let fourth = 4;",
                "+    println!(\"{}\", first + second + third + fourth);",
                "+}",
            ],
        );

        let hints = detect_hunk_moves(&[source.clone(), target.clone()]);

        assert_eq!(
            hints
                .moved_to
                .get(&source.id)
                .map(|hint| hint.target_hunk_id.as_str()),
            Some(target.id.as_str())
        );
        assert_eq!(
            hints
                .moved_from
                .get(&target.id)
                .map(|hint| hint.target_hunk_id.as_str()),
            Some(source.id.as_str())
        );
    }

    #[test]
    fn ignores_small_hunks() {
        let source = hunk(
            "source",
            "src/old.rs",
            "@@ -10,2 +10,0 @@",
            &["-let a = 1;"],
        );
        let target = hunk(
            "target",
            "src/new.rs",
            "@@ -10,0 +10,2 @@",
            &["+let a = 1;"],
        );

        let hints = detect_hunk_moves(&[source, target]);

        assert!(hints.moved_to.is_empty());
        assert!(hints.moved_from.is_empty());
    }

    #[test]
    fn links_parameterized_moves_with_low_exact_line_overlap() {
        let source = hunk(
            "source",
            "src/original_processor.rs",
            "@@ -109,18 +109,0 @@",
            &[
                "-let report_builder = ReportBuilder::new(\"daily-report\", input_source);",
                "-report_builder.configure(ReportOptions {",
                "-    output_dir: output_dir.clone(),",
                "-    retry_limit: 3,",
                "-    include_summary: true,",
                "-    labels: vec![\"daily\".to_string(), \"summary\".to_string()],",
                "-});",
                "-report_builder.add_step(\"load-records\", load_records);",
                "-report_builder.add_step(\"normalize-records\", normalize_records);",
                "-report_builder.add_step(\"write-report\", write_report);",
                "-report_builder.set_metadata(\"owner\", owner_name);",
                "-report_builder.set_metadata(\"environment\", environment_name);",
                "-report_builder.run_with_cache(cache_store);",
            ],
        );
        let target = hunk(
            "target",
            "src/report_pipeline.rs",
            "@@ -0,0 +35,24 @@",
            &[
                "+pub fn create_report_pipeline(config: ReportPipelineConfig) -> ReportPipeline {",
                "+    let mut builder = ReportBuilder::new(config.pipeline_name, config.input_source);",
                "+    builder.configure(ReportOptions {",
                "+        output_dir: config.output_dir.clone(),",
                "+        retry_limit: config.retry_limit,",
                "+        include_summary: config.include_summary,",
                "+        labels: config.labels,",
                "+    });",
                "+    builder.add_step(\"load-records\", load_records);",
                "+    builder.add_step(\"normalize-records\", normalize_records);",
                "+    builder.add_step(\"write-report\", write_report);",
                "+    builder.set_metadata(\"owner\", config.owner_name);",
                "+    builder.set_metadata(\"environment\", config.environment_name);",
                "+    builder.run_with_cache(config.cache_store);",
                "+    ReportPipeline::from_builder(builder)",
                "+}",
            ],
        );

        let hints = detect_hunk_moves(&[source.clone(), target.clone()]);

        assert_eq!(
            hints
                .moved_to
                .get(&source.id)
                .map(|hint| hint.target_hunk_id.as_str()),
            Some(target.id.as_str())
        );
    }
}
