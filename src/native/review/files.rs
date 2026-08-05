//! Rolling a diff's hunks up into one row per file, which is what the sidebar lists.
//!
//! Ported from `web/src/components/sidebarFiles.ts`, including the whole-file-move detection
//! that lets a delete and an add be shown as one rename.

use std::collections::HashMap;

use crate::api::{FileChangeKind, HunkView, LocalChangeSummary, SessionPayload};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileStageStatus {
    Staged,
    Unstaged,
    Partial,
}

#[derive(Clone, Debug)]
pub(crate) struct SidebarFile {
    pub(crate) file_path: String,
    pub(crate) file_name: String,
    pub(crate) change_kind: FileChangeKind,
    pub(crate) status: FileStageStatus,
    pub(crate) added_line_count: usize,
    pub(crate) removed_line_count: usize,
    pub(crate) hunk_count: usize,
    pub(crate) reviewed_hunk_count: usize,
    pub(crate) reviewed: bool,
    pub(crate) moved_from_file_path: Option<String>,
    pub(crate) moved_to_file_path: Option<String>,
}

fn file_name_of(file_path: &str) -> String {
    file_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(file_path)
        .to_string()
}

fn merge_change_kind(left: FileChangeKind, right: FileChangeKind) -> FileChangeKind {
    if left == right {
        left
    } else {
        FileChangeKind::Modified
    }
}

pub(crate) fn build_sidebar_files(payload: &SessionPayload) -> Vec<SidebarFile> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, SidebarFile> = HashMap::new();

    for hunk in &payload.hunks {
        match grouped.get_mut(&hunk.file_path) {
            Some(existing) => {
                existing.change_kind = merge_change_kind(existing.change_kind, hunk.change_kind);
                existing.added_line_count += hunk.added_line_count;
                existing.removed_line_count += hunk.removed_line_count;
                existing.hunk_count += 1;
                if hunk.reviewed {
                    existing.reviewed_hunk_count += 1;
                }
                existing.reviewed = existing.reviewed_hunk_count == existing.hunk_count;
                if (existing.status == FileStageStatus::Staged && !hunk.staged)
                    || (existing.status == FileStageStatus::Unstaged && hunk.staged)
                {
                    existing.status = FileStageStatus::Partial;
                }
            }
            None => {
                order.push(hunk.file_path.clone());
                grouped.insert(
                    hunk.file_path.clone(),
                    SidebarFile {
                        file_path: hunk.file_path.clone(),
                        file_name: file_name_of(&hunk.file_path),
                        change_kind: hunk.change_kind,
                        status: if hunk.staged {
                            FileStageStatus::Staged
                        } else {
                            FileStageStatus::Unstaged
                        },
                        added_line_count: hunk.added_line_count,
                        removed_line_count: hunk.removed_line_count,
                        hunk_count: 1,
                        reviewed_hunk_count: usize::from(hunk.reviewed),
                        reviewed: hunk.reviewed,
                        moved_from_file_path: None,
                        moved_to_file_path: None,
                    },
                );
            }
        }
    }

    let mut files: Vec<SidebarFile> = order
        .into_iter()
        .filter_map(|path| grouped.remove(&path))
        .collect();
    annotate_moved_files(&mut files, &payload.hunks);
    files
}

/// A file whose every hunk moved to one other file, where that file's every hunk came from
/// this one, is a rename. Showing it as a pair keeps the sidebar honest about what happened.
fn annotate_moved_files(files: &mut [SidebarFile], hunks: &[HunkView]) {
    let mut move_counts: HashMap<(String, String), usize> = HashMap::new();

    for hunk in hunks {
        let Some(target) = hunk.moved_to.as_ref().map(|hint| &hint.target_file_path) else {
            continue;
        };
        if &hunk.file_path == target {
            continue;
        }
        let source_kind = files
            .iter()
            .find(|file| file.file_path == hunk.file_path)
            .map(|file| file.change_kind);
        let target_kind = files
            .iter()
            .find(|file| &file.file_path == target)
            .map(|file| file.change_kind);
        if source_kind != Some(FileChangeKind::Deleted)
            || target_kind != Some(FileChangeKind::Added)
        {
            continue;
        }
        *move_counts
            .entry((hunk.file_path.clone(), target.clone()))
            .or_default() += 1;
    }

    let mut pairs: Vec<((String, String), usize)> = move_counts.into_iter().collect();
    // The strongest pairing wins, so a file split across two others is not claimed twice.
    pairs.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));

    for ((source_path, target_path), _) in pairs {
        let already_paired = files.iter().any(|file| {
            (file.file_path == source_path && file.moved_to_file_path.is_some())
                || (file.file_path == target_path && file.moved_from_file_path.is_some())
        });
        if already_paired {
            continue;
        }
        if !is_whole_file_move(files, hunks, &source_path, &target_path) {
            continue;
        }

        for file in files.iter_mut() {
            if file.file_path == source_path {
                file.moved_to_file_path = Some(target_path.clone());
            } else if file.file_path == target_path {
                file.moved_from_file_path = Some(source_path.clone());
            }
        }
    }
}

fn is_whole_file_move(
    files: &[SidebarFile],
    hunks: &[HunkView],
    source_path: &str,
    target_path: &str,
) -> bool {
    let Some(source) = files.iter().find(|file| file.file_path == source_path) else {
        return false;
    };
    let Some(target) = files.iter().find(|file| file.file_path == target_path) else {
        return false;
    };

    let source_hunks: Vec<&HunkView> = hunks
        .iter()
        .filter(|hunk| hunk.file_path == source_path)
        .collect();
    let target_hunks: Vec<&HunkView> = hunks
        .iter()
        .filter(|hunk| hunk.file_path == target_path)
        .collect();

    if source_hunks.is_empty()
        || target_hunks.is_empty()
        || source.added_line_count > 0
        || target.removed_line_count > 0
        || source.removed_line_count != target.added_line_count
    {
        return false;
    }

    source_hunks.iter().all(|hunk| {
        hunk.moved_to
            .as_ref()
            .is_some_and(|hint| hint.target_file_path == target_path)
    }) && target_hunks.iter().all(|hunk| {
        hunk.moved_from
            .as_ref()
            .is_some_and(|hint| hint.target_file_path == source_path)
    })
}

fn pluralize(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

pub(crate) fn local_changes_summary(summary: LocalChangeSummary) -> String {
    if summary.modified == 0 && summary.added == 0 && summary.deleted == 0 {
        return "no unstaged changes".to_string();
    }

    [
        (summary.modified > 0).then(|| pluralize(summary.modified, "modified file")),
        (summary.added > 0).then(|| pluralize(summary.added, "new file")),
        (summary.deleted > 0).then(|| pluralize(summary.deleted, "deleted file")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::HunkMoveHint;

    fn hunk(file_path: &str, staged: bool, added: usize, removed: usize) -> HunkView {
        HunkView {
            id: format!("{file_path}:{added}:{removed}:{staged}"),
            file_path: file_path.to_string(),
            change_kind: FileChangeKind::Modified,
            header: "@@ -1 +1 @@".to_string(),
            staged,
            reviewed: false,
            comment: String::new(),
            comment_dispatches: Vec::new(),
            patch_preview: String::new(),
            patch_line_count: 0,
            added_line_count: added,
            removed_line_count: removed,
            moved_from: None,
            moved_to: None,
            image_diff: None,
        }
    }

    fn payload(hunks: Vec<HunkView>) -> SessionPayload {
        SessionPayload {
            repo_name: "repo".to_string(),
            branch_name: None,
            commit_base: None,
            commits: Vec::new(),
            history_commits: Vec::new(),
            history_has_more: false,
            local_change_summary: LocalChangeSummary::default(),
            active_commit: None,
            repo_path: "/repo".to_string(),
            read_only: false,
            patch_preview_line_limit: 500,
            available_agents: Vec::new(),
            selected_agent: crate::api::AgentKind::None,
            full_file_path: None,
            hunks,
            review_comments: Vec::new(),
            export_text: String::new(),
        }
    }

    #[test]
    fn hunks_of_one_file_roll_up_into_one_row() {
        let files = build_sidebar_files(&payload(vec![
            hunk("src/a.rs", false, 3, 1),
            hunk("src/a.rs", false, 2, 0),
        ]));

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunk_count, 2);
        assert_eq!(files[0].added_line_count, 5);
        assert_eq!(files[0].removed_line_count, 1);
        assert_eq!(files[0].file_name, "a.rs");
    }

    #[test]
    fn a_file_with_staged_and_unstaged_hunks_is_partial() {
        let files = build_sidebar_files(&payload(vec![
            hunk("src/a.rs", true, 1, 0),
            hunk("src/a.rs", false, 1, 0),
        ]));

        assert_eq!(files[0].status, FileStageStatus::Partial);
    }

    #[test]
    fn files_keep_the_order_they_first_appear_in() {
        let files = build_sidebar_files(&payload(vec![
            hunk("z.rs", false, 1, 0),
            hunk("a.rs", false, 1, 0),
            hunk("z.rs", false, 1, 0),
        ]));

        let paths: Vec<&str> = files.iter().map(|file| file.file_path.as_str()).collect();
        assert_eq!(paths, vec!["z.rs", "a.rs"]);
    }

    #[test]
    fn a_file_is_reviewed_only_once_all_its_hunks_are() {
        let mut first = hunk("src/a.rs", false, 1, 0);
        first.reviewed = true;
        let second = hunk("src/a.rs", false, 2, 0);

        let files = build_sidebar_files(&payload(vec![first.clone(), second]));
        assert!(!files[0].reviewed);
        assert_eq!(files[0].reviewed_hunk_count, 1);

        let mut both = first.clone();
        both.id = "other".to_string();
        both.reviewed = true;
        let files = build_sidebar_files(&payload(vec![first, both]));
        assert!(files[0].reviewed);
    }

    #[test]
    fn a_whole_file_move_is_reported_as_a_pair() {
        let mut removed = hunk("src/old.rs", false, 0, 20);
        removed.change_kind = FileChangeKind::Deleted;
        removed.moved_to = Some(HunkMoveHint {
            target_hunk_id: "added".to_string(),
            target_file_path: "src/new.rs".to_string(),
            target_header: String::new(),
            score: 0.98,
        });
        let mut added = hunk("src/new.rs", false, 20, 0);
        added.change_kind = FileChangeKind::Added;
        added.moved_from = Some(HunkMoveHint {
            target_hunk_id: removed.id.clone(),
            target_file_path: "src/old.rs".to_string(),
            target_header: String::new(),
            score: 0.98,
        });

        let files = build_sidebar_files(&payload(vec![removed, added]));

        let old = files
            .iter()
            .find(|file| file.file_path == "src/old.rs")
            .expect("expected the deleted file");
        let new = files
            .iter()
            .find(|file| file.file_path == "src/new.rs")
            .expect("expected the added file");
        assert_eq!(old.moved_to_file_path.as_deref(), Some("src/new.rs"));
        assert_eq!(new.moved_from_file_path.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn an_unbalanced_move_is_not_reported_as_a_rename() {
        let mut removed = hunk("src/old.rs", false, 0, 20);
        removed.change_kind = FileChangeKind::Deleted;
        removed.moved_to = Some(HunkMoveHint {
            target_hunk_id: "added".to_string(),
            target_file_path: "src/new.rs".to_string(),
            target_header: String::new(),
            score: 0.9,
        });
        // The new file gained more than the old one lost, so it is not the same content.
        let mut added = hunk("src/new.rs", false, 40, 0);
        added.change_kind = FileChangeKind::Added;
        added.moved_from = Some(HunkMoveHint {
            target_hunk_id: removed.id.clone(),
            target_file_path: "src/old.rs".to_string(),
            target_header: String::new(),
            score: 0.9,
        });

        let files = build_sidebar_files(&payload(vec![removed, added]));

        assert!(files.iter().all(|file| file.moved_to_file_path.is_none()));
    }

    #[test]
    fn the_local_change_summary_reads_as_a_sentence() {
        assert_eq!(
            local_changes_summary(LocalChangeSummary::default()),
            "no unstaged changes"
        );
        assert_eq!(
            local_changes_summary(LocalChangeSummary {
                modified: 1,
                added: 2,
                deleted: 0,
            }),
            "1 modified file, 2 new files"
        );
    }
}
