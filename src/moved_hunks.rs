use std::collections::{HashMap, HashSet};

use textdistance::{Algorithm, Jaccard, SorensenDice};

use crate::api::{DiffHunk, HunkMoveHint};

const MIN_CHANGED_LINES: usize = 6;
const MIN_MOVE_SCORE: f64 = 0.58;

#[derive(Clone)]
struct Candidate {
    hunk_index: usize,
    line_fingerprints: Vec<String>,
    token_fingerprints: Vec<String>,
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
    let scorer = SimilarityScorer {
        jaccard: Jaccard::default(),
        sorensen_dice: SorensenDice::default(),
    };

    for old in &removed {
        for new in &added {
            if old.hunk_index == new.hunk_index {
                continue;
            }

            let score = scorer.similarity_score(old, new);
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

struct SimilarityScorer {
    jaccard: Jaccard,
    sorensen_dice: SorensenDice,
}

impl SimilarityScorer {
    fn similarity_score(&self, old: &Candidate, new: &Candidate) -> f64 {
        let line_score = self
            .jaccard
            .for_vec(&old.line_fingerprints, &new.line_fingerprints)
            .nsim();
        let token_jaccard_score = self
            .jaccard
            .for_vec(&old.token_fingerprints, &new.token_fingerprints)
            .nsim();
        let token_dice_score = self
            .sorensen_dice
            .for_vec(&old.token_fingerprints, &new.token_fingerprints)
            .nsim();

        line_score.max(token_jaccard_score).max(token_dice_score)
    }
}

fn candidate(hunk_index: usize, lines: Vec<String>) -> Option<Candidate> {
    if lines.len() < MIN_CHANGED_LINES {
        return None;
    }

    let line_fingerprints = lines.clone();
    let token_fingerprints = token_fingerprints(&lines);
    if line_fingerprints.is_empty() && token_fingerprints.is_empty() {
        return None;
    }

    Some(Candidate {
        hunk_index,
        line_fingerprints,
        token_fingerprints,
    })
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
