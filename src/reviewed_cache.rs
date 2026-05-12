use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};

use crate::git::run_git;

const CACHE_LIMIT: usize = 1000;
const CACHE_PATH: &[&str] = &["moonreview", "reviewed_hunks"];

fn hunk_patch_content(patch: &str) -> String {
    format!("{}\n", patch.lines().skip(2).collect::<Vec<_>>().join("\n"))
}

pub(crate) fn hunk_patch_hash(patch: &str) -> String {
    format!("{:x}", Sha1::digest(hunk_patch_content(patch).as_bytes()))
}

fn cache_file_path(repo_path: &Path) -> Result<PathBuf> {
    let git_dir = run_git(repo_path, &["rev-parse", "--git-dir"])?
        .trim()
        .to_string();
    let git_dir = PathBuf::from(git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        repo_path.join(git_dir)
    };
    Ok(CACHE_PATH
        .iter()
        .fold(git_dir, |path, segment| path.join(segment)))
}

fn read_cache_entries(repo_path: &Path) -> Result<VecDeque<String>> {
    let path = cache_file_path(repo_path)?;
    let Ok(content) = fs::read_to_string(&path) else {
        return Ok(VecDeque::new());
    };

    Ok(content
        .lines()
        .map(str::trim)
        .filter(|line| line.len() == 40 && line.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|line| line.to_ascii_lowercase())
        .collect())
}

fn write_cache_entries(repo_path: &Path, entries: &VecDeque<String>) -> Result<()> {
    let path = cache_file_path(repo_path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let content = entries.iter().cloned().collect::<Vec<_>>().join("\n");
    fs::write(&path, format!("{content}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn read_reviewed_hunk_hashes(repo_path: &Path) -> Result<HashSet<String>> {
    Ok(read_cache_entries(repo_path)?.into_iter().collect())
}

pub(crate) fn mark_hunk_patch_reviewed(repo_path: &Path, patch: &str) -> Result<()> {
    let hash = hunk_patch_hash(patch);
    let mut entries = read_cache_entries(repo_path)?;
    entries.retain(|entry| entry != &hash);
    entries.push_back(hash);
    while entries.len() > CACHE_LIMIT {
        entries.pop_front();
    }
    write_cache_entries(repo_path, &entries)
}

pub(crate) fn unmark_hunk_patch_reviewed(repo_path: &Path, patch: &str) -> Result<()> {
    let hash = hunk_patch_hash(patch);
    let mut entries = read_cache_entries(repo_path)?;
    entries.retain(|entry| entry != &hash);
    write_cache_entries(repo_path, &entries)
}

#[cfg(test)]
mod tests {
    use super::{hunk_patch_content, hunk_patch_hash};

    #[test]
    fn hunk_patch_hash_uses_sha1() {
        assert_eq!(
            hunk_patch_hash("diff --git a/file b/file\nindex 123..456 100644\nhello"),
            "f572d396fae9206628714fb2ce00f72e94f2258f"
        );
    }

    #[test]
    fn hunk_patch_content_skips_diff_and_index_metadata() {
        let patch = "\
diff --git a/src/main.rs b/src/main.rs
index 30525f5..4e1916b 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -3,6 +3,7 @@ mod api;
 mod cli;
 mod comments;
 mod git;
+mod reviewed_cache;
 mod server;
";

        assert_eq!(
            hunk_patch_content(patch),
            "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -3,6 +3,7 @@ mod api;
 mod cli;
 mod comments;
 mod git;
+mod reviewed_cache;
 mod server;
"
        );
    }
}
