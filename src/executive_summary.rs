use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::api::{DiffHunk, ExecutiveSummary, ExecutiveSummaryItem, FileChangeKind};

const LARGE_FILE_BYTES: usize = 500 * 1024;
const LARGE_FILE_LINES: usize = 1_000;
const LARGE_NEW_FILE_BYTES: usize = 200 * 1024;
const LARGE_NEW_FILE_LINES: usize = 500;
const HOTSPOT_CHANGED_LINES: usize = 100;
const HOTSPOT_HUNKS: usize = 4;
const LONG_BLOCK_LINES: usize = 80;
const MAX_ITEMS_PER_SECTION: usize = 5;
const MAX_TEXT_SCAN_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct FileSummary {
    file_path: String,
    change_kind: FileChangeKind,
    byte_size: Option<usize>,
    line_count: Option<usize>,
    added_line_count: usize,
    removed_line_count: usize,
    hunk_count: usize,
    binary_like: bool,
    generated_like: bool,
    longest_block_lines: Option<usize>,
}

pub(crate) fn build_executive_summary(
    repo_path: &Path,
    hunks: &[DiffHunk],
) -> Result<ExecutiveSummary> {
    let mut files = HashMap::<String, FileSummary>::new();

    for hunk in hunks {
        let (added, removed) = diff_line_stats(&hunk.patch);
        files
            .entry(hunk.file_path.clone())
            .and_modify(|summary| {
                summary.change_kind = merge_file_change_kind(summary.change_kind, hunk.change_kind);
                summary.added_line_count += added;
                summary.removed_line_count += removed;
                summary.hunk_count += 1;
            })
            .or_insert_with(|| {
                let stats = file_stats(repo_path, &hunk.file_path).unwrap_or_default();
                FileSummary {
                    file_path: hunk.file_path.clone(),
                    change_kind: hunk.change_kind,
                    byte_size: stats.byte_size,
                    line_count: stats.line_count,
                    added_line_count: added,
                    removed_line_count: removed,
                    hunk_count: 1,
                    binary_like: stats.binary_like,
                    generated_like: stats.generated_like,
                    longest_block_lines: stats.longest_block_lines,
                }
            });
    }

    let files = files.into_values().collect::<Vec<_>>();
    Ok(ExecutiveSummary {
        large_files: large_files(&files),
        large_new_files: large_new_files(&files),
        hotspots: hotspots(&files),
        complexity_hints: complexity_hints(&files),
    })
}

fn large_files(files: &[FileSummary]) -> Vec<ExecutiveSummaryItem> {
    let mut items = files
        .iter()
        .filter(|file| {
            file.change_kind != FileChangeKind::Added
                && (file.byte_size.unwrap_or(0) > LARGE_FILE_BYTES
                    || file.line_count.unwrap_or(0) > LARGE_FILE_LINES)
        })
        .map(|file| item(file, "Large file", large_reason(file, false)))
        .collect::<Vec<_>>();
    sort_by_size(&mut items);
    items.truncate(MAX_ITEMS_PER_SECTION);
    items
}

fn large_new_files(files: &[FileSummary]) -> Vec<ExecutiveSummaryItem> {
    let mut items = files
        .iter()
        .filter(|file| {
            file.change_kind == FileChangeKind::Added
                && (file.byte_size.unwrap_or(0) > LARGE_NEW_FILE_BYTES
                    || file.line_count.unwrap_or(0) > LARGE_NEW_FILE_LINES)
        })
        .map(|file| item(file, "Large new file", large_reason(file, true)))
        .collect::<Vec<_>>();
    sort_by_size(&mut items);
    items.truncate(MAX_ITEMS_PER_SECTION);
    items
}

fn hotspots(files: &[FileSummary]) -> Vec<ExecutiveSummaryItem> {
    let mut items = files
        .iter()
        .filter(|file| {
            file.changed_lines() >= HOTSPOT_CHANGED_LINES || file.hunk_count >= HOTSPOT_HUNKS
        })
        .map(|file| {
            item(
                file,
                "Hotspot",
                format!(
                    "{} changed lines across {} hunks",
                    file.changed_lines(),
                    file.hunk_count
                ),
            )
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|item| {
        std::cmp::Reverse((
            item.added_line_count + item.removed_line_count,
            item.hunk_count,
        ))
    });
    items.truncate(MAX_ITEMS_PER_SECTION);
    items
}

fn complexity_hints(files: &[FileSummary]) -> Vec<ExecutiveSummaryItem> {
    let mut items = Vec::new();
    for file in files {
        if file.binary_like {
            items.push(item(
                file,
                "Binary-looking file",
                "binary-looking content or file type changed".to_string(),
            ));
        } else if file.generated_like {
            items.push(item(
                file,
                "Generated-looking file",
                "generated or bundled file path changed".to_string(),
            ));
        } else if let Some(lines) = file
            .longest_block_lines
            .filter(|lines| *lines >= LONG_BLOCK_LINES)
        {
            items.push(item(
                file,
                "Long block",
                format!("largest function-like block is about {lines} lines"),
            ));
        }
    }
    items.sort_by_key(|item| {
        std::cmp::Reverse((
            item.line_count.unwrap_or(0),
            item.added_line_count + item.removed_line_count,
        ))
    });
    items.truncate(MAX_ITEMS_PER_SECTION);
    items
}

impl FileSummary {
    fn changed_lines(&self) -> usize {
        self.added_line_count + self.removed_line_count
    }
}

fn item(file: &FileSummary, label: &str, reason: String) -> ExecutiveSummaryItem {
    ExecutiveSummaryItem {
        file_path: file.file_path.clone(),
        label: label.to_string(),
        reason,
        byte_size: file.byte_size,
        line_count: file.line_count,
        added_line_count: file.added_line_count,
        removed_line_count: file.removed_line_count,
        hunk_count: file.hunk_count,
    }
}

fn large_reason(file: &FileSummary, new_file: bool) -> String {
    let threshold = if new_file {
        format!(
            "new-file threshold is {} KB or {} lines",
            LARGE_NEW_FILE_BYTES / 1024,
            LARGE_NEW_FILE_LINES
        )
    } else {
        format!(
            "threshold is {} KB or {} lines",
            LARGE_FILE_BYTES / 1024,
            LARGE_FILE_LINES
        )
    };
    format!(
        "{}; current size is {} and {}",
        threshold,
        format_bytes(file.byte_size),
        format_lines(file.line_count)
    )
}

fn sort_by_size(items: &mut [ExecutiveSummaryItem]) {
    items.sort_by_key(|item| {
        std::cmp::Reverse((item.byte_size.unwrap_or(0), item.line_count.unwrap_or(0)))
    });
}

fn format_bytes(value: Option<usize>) -> String {
    match value {
        Some(bytes) if bytes >= 1024 => format!("{} KB", (bytes + 1023) / 1024),
        Some(bytes) => format!("{bytes} bytes"),
        None => "unknown bytes".to_string(),
    }
}

fn format_lines(value: Option<usize>) -> String {
    match value {
        Some(lines) => format!("{lines} lines"),
        None => "unknown lines".to_string(),
    }
}

#[derive(Default)]
struct FileStats {
    byte_size: Option<usize>,
    line_count: Option<usize>,
    binary_like: bool,
    generated_like: bool,
    longest_block_lines: Option<usize>,
}

fn file_stats(repo_path: &Path, file_path: &str) -> Result<FileStats> {
    let path = safe_repo_path(repo_path, file_path);
    let generated_like = generated_like_path(file_path);
    let Ok(metadata) = fs::metadata(&path) else {
        return Ok(FileStats {
            generated_like,
            ..Default::default()
        });
    };
    if !metadata.is_file() {
        return Ok(FileStats {
            generated_like,
            ..Default::default()
        });
    }

    let byte_size = metadata.len() as usize;
    let extension_binary = binary_like_path(file_path);
    if extension_binary || byte_size > MAX_TEXT_SCAN_BYTES {
        return Ok(FileStats {
            byte_size: Some(byte_size),
            binary_like: extension_binary,
            generated_like,
            ..Default::default()
        });
    }

    let bytes = fs::read(&path)?;
    let binary_like = bytes.contains(&0);
    if binary_like {
        return Ok(FileStats {
            byte_size: Some(byte_size),
            binary_like: true,
            generated_like,
            ..Default::default()
        });
    }

    let text = String::from_utf8_lossy(&bytes);
    Ok(FileStats {
        byte_size: Some(byte_size),
        line_count: Some(text.lines().count()),
        binary_like,
        generated_like,
        longest_block_lines: longest_function_like_block(&text),
    })
}

fn safe_repo_path(repo_path: &Path, file_path: &str) -> PathBuf {
    file_path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .fold(repo_path.to_path_buf(), |path, segment| path.join(segment))
}

fn binary_like_path(file_path: &str) -> bool {
    let lower = file_path.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".pdf", ".zip", ".gz", ".tar", ".woff",
        ".woff2", ".ttf", ".otf", ".mp4", ".mov", ".mp3",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}

fn generated_like_path(file_path: &str) -> bool {
    let lower = file_path.to_ascii_lowercase();
    lower.ends_with(".lock")
        || lower.ends_with(".min.js")
        || lower.contains("/generated/")
        || lower.contains("/dist/")
        || lower.contains("/build/")
}

fn longest_function_like_block(text: &str) -> Option<usize> {
    let mut brace_depth = 0isize;
    let mut block_start: Option<usize> = None;
    let mut longest = 0usize;

    for (index, line) in text.lines().enumerate() {
        if block_start.is_none() && looks_like_block_start(line) && line.contains('{') {
            block_start = Some(index);
        }
        brace_depth += line.matches('{').count() as isize;
        brace_depth -= line.matches('}').count() as isize;
        if brace_depth <= 0 {
            if let Some(start) = block_start.take() {
                longest = longest.max(index.saturating_sub(start) + 1);
            }
            brace_depth = 0;
        }
    }

    longest_python_like_block(text)
        .into_iter()
        .chain((longest > 0).then_some(longest))
        .max()
}

fn longest_python_like_block(text: &str) -> Option<usize> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut longest = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ") || trimmed.starts_with("async def ")) {
            continue;
        }
        let base_indent = line.len() - trimmed.len();
        let mut end = index;
        for (next_index, next_line) in lines.iter().enumerate().skip(index + 1) {
            let next_trimmed = next_line.trim_start();
            if next_trimmed.is_empty() {
                continue;
            }
            let indent = next_line.len() - next_trimmed.len();
            if indent <= base_indent {
                break;
            }
            end = next_index;
        }
        longest = longest.max(end.saturating_sub(index) + 1);
    }
    (longest > 0).then_some(longest)
}

fn looks_like_block_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.contains("function ")
        || trimmed.contains("=>")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("async fn ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("func ")
}

fn diff_line_stats(patch: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

fn merge_file_change_kind(left: FileChangeKind, right: FileChangeKind) -> FileChangeKind {
    if left == right {
        left
    } else {
        FileChangeKind::Modified
    }
}

#[cfg(test)]
mod tests {
    use super::build_executive_summary;
    use crate::api::{DiffHunk, FileChangeKind};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moonreview-summary-test-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("failed to create temp test directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn hunk(file_path: &str, change_kind: FileChangeKind, body: &str) -> DiffHunk {
        DiffHunk {
            id: file_path.to_string(),
            file_path: file_path.to_string(),
            change_kind,
            header: "@@ -1 +1 @@".to_string(),
            patch: format!(
                "diff --git a/{file_path} b/{file_path}\n--- a/{file_path}\n+++ b/{file_path}\n@@ -1 +1 @@\n{body}\n"
            ),
            staged: false,
        }
    }

    #[test]
    fn flags_large_existing_and_new_files() {
        let temp = TestDir::new();
        fs::write(temp.path.join("existing.txt"), "line\n".repeat(1_100)).unwrap();
        fs::write(temp.path.join("new.txt"), "line\n".repeat(600)).unwrap();

        let summary = build_executive_summary(
            &temp.path,
            &[
                hunk("existing.txt", FileChangeKind::Modified, "+changed"),
                hunk("new.txt", FileChangeKind::Added, "+created"),
            ],
        )
        .unwrap();

        assert_eq!(summary.large_files[0].file_path, "existing.txt");
        assert_eq!(summary.large_new_files[0].file_path, "new.txt");
    }

    #[test]
    fn ranks_hotspots_from_changed_lines_and_hunks() {
        let temp = TestDir::new();
        fs::write(temp.path.join("busy.txt"), "small\n").unwrap();
        let body = (0..120)
            .map(|index| format!("+line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let summary = build_executive_summary(
            &temp.path,
            &[hunk("busy.txt", FileChangeKind::Modified, &body)],
        )
        .unwrap();

        assert_eq!(summary.hotspots[0].file_path, "busy.txt");
        assert!(summary.hotspots[0].reason.contains("120 changed lines"));
    }

    #[test]
    fn skips_binary_content_for_block_scanning() {
        let temp = TestDir::new();
        fs::write(temp.path.join("asset.bin"), b"abc\0def").unwrap();

        let summary = build_executive_summary(
            &temp.path,
            &[hunk("asset.bin", FileChangeKind::Modified, "+binary")],
        )
        .unwrap();

        assert_eq!(summary.complexity_hints[0].label, "Binary-looking file");
    }

    #[test]
    fn reports_long_function_like_blocks() {
        let temp = TestDir::new();
        let mut content = String::from("function largeThing() {\n");
        content.push_str(&"  call();\n".repeat(90));
        content.push_str("}\n");
        fs::write(temp.path.join("code.js"), content).unwrap();

        let summary = build_executive_summary(
            &temp.path,
            &[hunk("code.js", FileChangeKind::Modified, "+call();")],
        )
        .unwrap();

        assert_eq!(summary.complexity_hints[0].file_path, "code.js");
        assert_eq!(summary.complexity_hints[0].label, "Long block");
    }
}
