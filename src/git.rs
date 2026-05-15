use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    agent::ChildExt,
    api::{CommitView, DiffHunk, DiffTarget, FileChangeKind, RepoSession, stable_id},
};

pub(crate) fn canonicalize_repo(path: impl AsRef<Path>) -> Result<PathBuf> {
    let original_path = path.as_ref().to_path_buf();
    let mut path = path
        .as_ref()
        .canonicalize()
        .context("failed to resolve path")?;

    loop {
        if path.join(".git").exists() {
            return Ok(path);
        }
        if !path.pop() {
            break;
        }
    }

    bail!("{} is not inside a git repository", original_path.display())
}

pub(crate) fn list_changed_submodule_repos(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let submodule_paths = run_git(repo_path, &["submodule", "status", "--recursive"])?
        .lines()
        .filter_map(parse_submodule_status_path)
        .map(|relative_path| repo_path.join(relative_path))
        .collect::<Vec<_>>();

    let mut changed = Vec::new();
    for submodule_path in submodule_paths {
        let status = run_git(
            &submodule_path,
            &["status", "--short", "--ignore-submodules=none"],
        )?;
        if !status.trim().is_empty() {
            changed.push(canonicalize_repo(&submodule_path)?);
        }
    }

    changed.sort();
    changed.dedup();
    Ok(changed)
}

pub(crate) fn collect_hunks(repo_path: &Path, diff_target: &DiffTarget) -> Result<Vec<DiffHunk>> {
    if let Some(base) = &diff_target.base {
        let diff = run_target_diff(repo_path, base, diff_target.pathspec.as_deref())?;
        return parse_diff(&diff, false);
    }

    let mut hunks = parse_diff(
        &run_git(
            repo_path,
            &[
                "diff",
                "--diff-algorithm=histogram",
                "--no-color",
                "--unified=3",
            ],
        )?,
        false,
    )?;
    hunks.extend(parse_diff(
        &run_git(
            repo_path,
            &[
                "diff",
                "--cached",
                "--diff-algorithm=histogram",
                "--no-color",
                "--unified=3",
            ],
        )?,
        true,
    )?);
    for path in list_untracked_files(repo_path)? {
        let untracked_args = vec![
            "diff",
            "--no-index",
            "--diff-algorithm=histogram",
            "--no-color",
            "--unified=3",
            "--",
            "/dev/null",
            &path,
        ];
        let diff = run_git_allow_status(repo_path, &untracked_args, &[0, 1])?;
        hunks.extend(parse_diff(&diff, false)?);
    }
    Ok(hunks)
}

pub(crate) fn collect_session_hunks(session: &RepoSession) -> Result<Vec<DiffHunk>> {
    if let Some(commit) = &session.active_commit {
        return collect_commit_hunks(&session.repo_path, commit);
    }

    collect_hunks(&session.repo_path, &session.diff_target)
}

pub(crate) fn collect_commit_hunks(repo_path: &Path, commit: &str) -> Result<Vec<DiffHunk>> {
    if commit.trim().is_empty() {
        bail!("commit cannot be empty");
    }

    let diff = run_git(
        repo_path,
        &[
            "show",
            "--format=",
            "--diff-algorithm=histogram",
            "--no-color",
            "--unified=3",
            commit,
        ],
    )?;
    parse_diff(&diff, true)
}

pub(crate) fn current_branch_name(repo_path: &Path) -> Result<Option<String>> {
    let branch = run_git_allow_status(repo_path, &["symbolic-ref", "--short", "HEAD"], &[0, 128])?;
    let branch = branch.trim();
    if branch.is_empty() {
        Ok(None)
    } else {
        Ok(Some(branch.to_string()))
    }
}

pub(crate) fn branch_commits_since_default(
    repo_path: &Path,
) -> Result<(Option<String>, Vec<CommitView>)> {
    let Some(base_ref) = default_branch_ref(repo_path)? else {
        return Ok((None, Vec::new()));
    };
    let range = format!("{base_ref}..HEAD");
    // Pretty format fields, separated by ASCII unit separators:
    // %H = full commit SHA, %h = abbreviated SHA, %an = author name, %s = subject.
    let format = "%H%x1f%h%x1f%an%x1f%s";
    let output = run_git(
        repo_path,
        &[
            "log",
            "--date=relative",
            &format!("--format={format}"),
            &range,
        ],
    )?;
    let commits = output
        .lines()
        .filter_map(parse_commit_view)
        .collect::<Vec<_>>();

    Ok((Some(base_ref), commits))
}

fn default_branch_ref(repo_path: &Path) -> Result<Option<String>> {
    if let Some(upstream) = current_branch_upstream_ref(repo_path)? {
        return Ok(Some(upstream));
    }

    let origin_head = run_git_allow_status(
        repo_path,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        &[0, 1, 128],
    )?;
    let origin_head = origin_head.trim();
    if !origin_head.is_empty() && git_ref_exists(repo_path, origin_head)? {
        return Ok(Some(origin_head.to_string()));
    }

    Ok(None)
}

fn current_branch_upstream_ref(repo_path: &Path) -> Result<Option<String>> {
    let upstream = run_git_allow_status(
        repo_path,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        &[0, 128],
    )?;
    let upstream = upstream.trim();
    if !upstream.is_empty() && git_ref_exists(repo_path, upstream)? {
        return Ok(Some(upstream.to_string()));
    }

    Ok(None)
}

fn git_ref_exists(repo_path: &Path, git_ref: &str) -> Result<bool> {
    Ok(!run_git_allow_status(
        repo_path,
        &["rev-parse", "--verify", "--quiet", git_ref],
        &[0, 1],
    )?
    .trim()
    .is_empty())
}

fn parse_commit_view(line: &str) -> Option<CommitView> {
    let mut fields = line.splitn(4, '\x1f');
    let sha = fields.next()?.to_string();
    let short_sha = fields.next()?.to_string();
    let author = fields.next()?.to_string();
    let subject = fields.next()?.to_string();

    Some(CommitView {
        sha,
        short_sha,
        subject,
        author,
        review_status: Default::default(),
    })
}

pub(crate) fn read_repo_file(repo_path: &Path, file_path: &str) -> Result<String> {
    if file_path.trim().is_empty() {
        bail!("file path cannot be empty");
    }

    let candidate = repo_path.join(file_path);
    if let Ok(resolved) = candidate.canonicalize() {
        if !resolved.starts_with(repo_path) {
            bail!("file path is outside the repository");
        }

        return fs::read_to_string(&resolved)
            .with_context(|| format!("failed to read {}", resolved.display()));
    }

    let head_spec = format!("HEAD:{file_path}");
    let content = run_git_allow_status(repo_path, &["show", &head_spec], &[0, 128])?;
    if content.trim().is_empty() {
        bail!("file is not available in the working tree or HEAD");
    }

    Ok(content)
}

fn run_target_diff(repo_path: &Path, base: &str, pathspec: Option<&str>) -> Result<String> {
    let mut args = vec![
        "diff",
        "--diff-algorithm=histogram",
        "--no-color",
        "--unified=3",
    ];
    args.push(base);
    if let Some(pathspec) = pathspec.filter(|value| !value.is_empty()) {
        args.push("--");
        args.push(pathspec);
    }
    run_git(repo_path, &args)
}

fn parse_diff(diff: &str, staged: bool) -> Result<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    for section in split_diff_sections(diff) {
        let file_path = parse_file_path(&section).unwrap_or_else(|| "unknown".to_string());
        let change_kind = parse_change_kind(&section);
        let mut prelude = Vec::new();
        let mut idx = 0usize;

        while idx < section.len() && !section[idx].starts_with("@@") {
            prelude.push(section[idx].clone());
            idx += 1;
        }

        while idx < section.len() {
            let header = section[idx].clone();
            let mut patch_lines = prelude.clone();
            patch_lines.push(header.clone());
            idx += 1;

            while idx < section.len()
                && !section[idx].starts_with("@@")
                && !section[idx].starts_with("diff --git ")
            {
                patch_lines.push(section[idx].clone());
                idx += 1;
            }

            let patch = format!("{}\n", patch_lines.join("\n"));
            let id = stable_id(&(file_path.clone(), header.clone(), patch.clone()));
            hunks.push(DiffHunk {
                id,
                file_path: file_path.clone(),
                change_kind,
                header,
                patch,
                staged,
            });
        }
    }

    Ok(hunks)
}

fn split_diff_sections(diff: &str) -> Vec<Vec<String>> {
    let mut sections = Vec::new();
    let mut current = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(current);
            current = Vec::new();
        }
        current.push(line.to_string());
    }

    if !current.is_empty() {
        sections.push(current);
    }

    sections
}

fn parse_file_path(section: &[String]) -> Option<String> {
    for line in section {
        if let Some(path) = line.strip_prefix("+++ b/") {
            return Some(path.to_string());
        }
    }

    section.first().and_then(|line| {
        line.strip_prefix("diff --git a/")
            .and_then(|rest| rest.split_once(" b/").map(|(_, right)| right.to_string()))
    })
}

fn parse_change_kind(section: &[String]) -> FileChangeKind {
    let has_new_file = section
        .iter()
        .any(|line| line.starts_with("new file mode "));
    let has_deleted_file = section
        .iter()
        .any(|line| line.starts_with("deleted file mode "));
    let added_from_dev_null = section.iter().any(|line| line == "--- /dev/null");
    let deleted_to_dev_null = section.iter().any(|line| line == "+++ /dev/null");

    if has_new_file || added_from_dev_null {
        FileChangeKind::Added
    } else if has_deleted_file || deleted_to_dev_null {
        FileChangeKind::Deleted
    } else {
        FileChangeKind::Modified
    }
}

fn list_untracked_files(repo_path: &Path) -> Result<Vec<String>> {
    Ok(
        run_git(repo_path, &["ls-files", "--others", "--exclude-standard"])?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn parse_submodule_status_path(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let rest = trimmed[1..].trim_start();
    let (_, path_and_rest) = rest.split_once(' ')?;
    let path = path_and_rest
        .split_once(" (")
        .map_or(path_and_rest, |(path, _)| path);
    let path = path.trim();
    if path.is_empty() { None } else { Some(path) }
}

fn run_git_allow_status(repo_path: &Path, args: &[&str], allowed: &[i32]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    let status = output.status.code().unwrap_or(-1);
    if !allowed.contains(&status) {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn preview_patch(patch: &str, lines: usize) -> String {
    patch.lines().take(lines).collect::<Vec<_>>().join("\n")
}

pub(crate) fn build_partial_patch_from_selection(patch: &str, selection: &str) -> Result<String> {
    let lines = patch.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let header_index = lines
        .iter()
        .position(|line| line.starts_with("@@"))
        .ok_or_else(|| anyhow!("patch has no hunk header"))?;
    let prelude = &lines[..header_index];
    let header = &lines[header_index];
    let body = &lines[header_index + 1..];

    let selection_lines = selection
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if selection_lines.is_empty() {
        bail!("selection is empty");
    }

    let start = body
        .windows(selection_lines.len())
        .position(|window| {
            window
                .iter()
                .map(String::as_str)
                .eq(selection_lines.iter().map(String::as_str))
        })
        .ok_or_else(|| anyhow!("selected lines were not found in the hunk"))?;
    let end = start + selection_lines.len() - 1;

    let selected_slice = &body[start..=end];
    if !selected_slice
        .iter()
        .any(|line| line.starts_with('+') || line.starts_with('-'))
    {
        bail!("selection does not contain diff lines to stage");
    }

    let context_start = start.saturating_sub(3);
    let context_end = (end + 3).min(body.len().saturating_sub(1));
    let subset = &body[context_start..=context_end];

    let (old_start, new_start, _, _) = parse_hunk_header(header)?;
    let old_offset = body[..context_start]
        .iter()
        .filter(|line| !line.starts_with('+'))
        .count();
    let new_offset = body[..context_start]
        .iter()
        .filter(|line| !line.starts_with('-'))
        .count();
    let subset_old_count = subset.iter().filter(|line| !line.starts_with('+')).count();
    let subset_new_count = subset.iter().filter(|line| !line.starts_with('-')).count();

    let subset_header = format_hunk_header(
        old_start + old_offset,
        subset_old_count,
        new_start + new_offset,
        subset_new_count,
    );

    let mut out = String::new();
    out.push_str(&prelude.join("\n"));
    out.push('\n');
    out.push_str(&subset_header);
    out.push('\n');
    out.push_str(&subset.join("\n"));
    out.push('\n');
    Ok(out)
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize)> {
    let raw = header
        .split("@@")
        .nth(1)
        .map(str::trim)
        .ok_or_else(|| anyhow!("invalid hunk header"))?;
    let mut parts = raw.split_whitespace();
    let old_part = parts
        .next()
        .ok_or_else(|| anyhow!("invalid old hunk header"))?;
    let new_part = parts
        .next()
        .ok_or_else(|| anyhow!("invalid new hunk header"))?;

    let (old_start, old_count) = parse_header_range(old_part.trim_start_matches('-'))?;
    let (new_start, new_count) = parse_header_range(new_part.trim_start_matches('+'))?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_header_range(value: &str) -> Result<(usize, usize)> {
    if let Some((start, count)) = value.split_once(',') {
        Ok((start.parse()?, count.parse()?))
    } else {
        Ok((value.parse()?, 1))
    }
}

fn format_hunk_header(
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
) -> String {
    format!(
        "@@ -{},{} +{},{} @@",
        old_start, old_count, new_start, new_count
    )
}

pub(crate) fn apply_patch(
    repo_path: &Path,
    patch: &str,
    cached: bool,
    reverse: bool,
) -> Result<()> {
    let mut command = Command::new("git");
    command.current_dir(repo_path).arg("apply");
    if cached {
        command.arg("--cached");
    }
    if reverse {
        command.arg("--reverse");
    }
    command.arg("-");

    let output = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start git apply")?
        .wait_with_output_from_stdin(patch.as_bytes(), "failed to write patch to git apply")?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

pub(crate) fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn run_git_no_output(repo_path: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(())
}

pub(crate) fn parse_review_target(raw: Option<String>) -> Result<DiffTarget> {
    let Some(value) = raw else {
        return Ok(DiffTarget::default());
    };

    if value == "serve" {
        return Ok(DiffTarget::default());
    }

    if let Some((base, pathspec)) = value.split_once(':') {
        if base.trim().is_empty() {
            bail!("diff target base cannot be empty");
        }

        return Ok(DiffTarget {
            base: Some(base.trim().to_string()),
            pathspec: Some(pathspec.trim().to_string()),
        });
    }

    Ok(DiffTarget {
        base: Some(value),
        pathspec: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        branch_commits_since_default, canonicalize_repo, collect_commit_hunks, collect_hunks,
        collect_session_hunks, run_git, run_git_no_output,
    };
    use crate::api::{AgentKind, DiffTarget, RepoSession};
    use std::collections::{HashMap, HashSet};
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
                "moonreview-test-{}-{}",
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

    fn diff_line_counts(patch: &str) -> (usize, usize) {
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

    fn init_test_repo(repo_root: &PathBuf) {
        fs::create_dir_all(repo_root).expect("failed to create repo directory");
        run_git_no_output(repo_root, &["init"]).expect("failed to init repo");
        run_git_no_output(repo_root, &["config", "user.email", "test@example.com"])
            .expect("failed to configure git email");
        run_git_no_output(repo_root, &["config", "user.name", "Test User"])
            .expect("failed to configure git user");
        run_git_no_output(repo_root, &["config", "commit.gpgsign", "false"])
            .expect("failed to disable git signing");
    }

    fn test_session(repo_root: PathBuf, active_commit: Option<String>) -> RepoSession {
        RepoSession {
            repo_path: repo_root,
            diff_target: DiffTarget::default(),
            active_commit,
            comments: HashMap::new(),
            comment_contexts: HashMap::new(),
            reviewed: HashSet::new(),
            selected_agent: AgentKind::None,
            comment_dispatches: HashMap::new(),
        }
    }

    #[test]
    fn collect_commit_hunks_returns_hunks_for_single_commit() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let file_path = repo_root.join("example.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");

        fs::write(&file_path, "one\nTWO\nthree\n").expect("failed to write changed file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add change");
        run_git_no_output(&repo_root, &["commit", "-m", "change example"])
            .expect("failed to commit change");
        let commit = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");

        // Act
        let hunks = collect_commit_hunks(&repo_root, commit.trim())
            .expect("failed to collect commit hunks");

        // Assert
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "example.txt");
        assert!(hunks[0].staged);
        assert_eq!(diff_line_counts(&hunks[0].patch), (1, 1));
    }

    #[test]
    fn collect_session_hunks_uses_active_commit_when_present() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let committed_path = repo_root.join("committed.txt");
        fs::write(&committed_path, "before\n").expect("failed to write committed file");
        run_git_no_output(&repo_root, &["add", "committed.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");

        fs::write(&committed_path, "after\n").expect("failed to change committed file");
        run_git_no_output(&repo_root, &["add", "committed.txt"]).expect("failed to add change");
        run_git_no_output(&repo_root, &["commit", "-m", "change committed"])
            .expect("failed to commit change");
        let commit = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");

        let local_path = repo_root.join("local.txt");
        fs::write(&local_path, "local\n").expect("failed to write local file");

        // Act
        let local_hunks = collect_session_hunks(&test_session(repo_root.clone(), None))
            .expect("failed to collect local hunks");
        let commit_hunks = collect_session_hunks(&test_session(
            repo_root.clone(),
            Some(commit.trim().to_string()),
        ))
        .expect("failed to collect active commit hunks");

        // Assert
        assert_eq!(local_hunks.len(), 1);
        assert_eq!(local_hunks[0].file_path, "local.txt");
        assert!(!local_hunks[0].staged);

        assert_eq!(commit_hunks.len(), 1);
        assert_eq!(commit_hunks[0].file_path, "committed.txt");
        assert!(commit_hunks[0].staged);
    }

    #[test]
    fn branch_commits_since_default_prefers_current_branch_upstream() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let file_path = repo_root.join("example.txt");
        fs::write(&file_path, "base\n").expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");
        let main_head = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");
        run_git_no_output(&repo_root, &["remote", "add", "origin", "."])
            .expect("failed to add remote");
        run_git_no_output(
            &repo_root,
            &["update-ref", "refs/remotes/origin/main", main_head.trim()],
        )
        .expect("failed to create remote default ref");
        run_git_no_output(
            &repo_root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )
        .expect("failed to set remote default ref");

        run_git_no_output(&repo_root, &["checkout", "-b", "feature"])
            .expect("failed to create feature branch");
        fs::write(&file_path, "base\none\n").expect("failed to write first change");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add first change");
        run_git_no_output(&repo_root, &["commit", "-m", "first change"])
            .expect("failed to commit first change");
        fs::write(&file_path, "base\none\ntwo\n").expect("failed to write second change");
        run_git_no_output(&repo_root, &["add", "example.txt"])
            .expect("failed to add second change");
        run_git_no_output(&repo_root, &["commit", "-m", "second change"])
            .expect("failed to commit second change");
        let feature_head =
            run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read feature HEAD");
        run_git_no_output(
            &repo_root,
            &[
                "update-ref",
                "refs/remotes/origin/feature",
                feature_head.trim(),
            ],
        )
        .expect("failed to create remote feature ref");
        run_git_no_output(&repo_root, &["config", "branch.feature.remote", "origin"])
            .expect("failed to configure upstream remote");
        run_git_no_output(
            &repo_root,
            &["config", "branch.feature.merge", "refs/heads/feature"],
        )
        .expect("failed to configure upstream branch");

        // Act
        let (base, commits) =
            branch_commits_since_default(&repo_root).expect("failed to collect commits");

        // Assert
        assert_eq!(base.as_deref(), Some("origin/feature"));
        assert!(commits.is_empty());
    }

    #[test]
    fn collect_hunks_keeps_partially_staged_file_counts_separate() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        fs::create_dir_all(&repo_root).expect("failed to create repo directory");
        run_git_no_output(&repo_root, &["init"]).expect("failed to init repo");
        run_git_no_output(&repo_root, &["config", "user.email", "test@example.com"])
            .expect("failed to configure git email");
        run_git_no_output(&repo_root, &["config", "user.name", "Test User"])
            .expect("failed to configure git user");
        run_git_no_output(&repo_root, &["config", "commit.gpgsign", "false"])
            .expect("failed to disable git signing");

        let file_path = repo_root.join("example.txt");
        fs::write(&file_path, "one\ntwo\nthree\nfour\n").expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");

        fs::write(&file_path, "one\nTWO staged\nthree\nfour\n")
            .expect("failed to write staged change");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to stage change");
        fs::write(&file_path, "one\nTWO staged\nTHREE unstaged\nfour\n")
            .expect("failed to write unstaged change");

        // Act
        let hunks =
            collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
        let staged = hunks
            .iter()
            .filter(|hunk| hunk.file_path == "example.txt" && hunk.staged)
            .map(|hunk| diff_line_counts(&hunk.patch))
            .fold((0, 0), |sum, item| (sum.0 + item.0, sum.1 + item.1));
        let unstaged = hunks
            .iter()
            .filter(|hunk| hunk.file_path == "example.txt" && !hunk.staged)
            .map(|hunk| diff_line_counts(&hunk.patch))
            .fold((0, 0), |sum, item| (sum.0 + item.0, sum.1 + item.1));

        // Assert
        assert_eq!(staged, (1, 1));
        assert_eq!(unstaged, (1, 1));
    }

    #[test]
    fn canonicalize_repo_walks_up_to_git_root() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        let nested = repo_root.join("src/components");
        fs::create_dir_all(repo_root.join(".git")).expect("failed to create fake git dir");
        fs::create_dir_all(&nested).expect("failed to create nested directory");

        // Act
        let resolved = canonicalize_repo(&nested).expect("expected repo root to resolve");

        // Assert
        assert_eq!(resolved, repo_root.canonicalize().unwrap());
    }

    #[test]
    fn canonicalize_repo_errors_outside_git_repo() {
        // Arrange
        let temp = TestDir::new();
        let dir = temp.path.join("plain/nested");
        fs::create_dir_all(&dir).expect("failed to create plain directory");

        // Act
        let error = canonicalize_repo(&dir).expect_err("expected resolution failure");

        // Assert
        assert!(error.to_string().contains("is not inside a git repository"));
    }

    #[test]
    fn parse_submodule_status_path_handles_plain_and_branch_lines() {
        // Arrange
        let plain_line = " 3f4a1c2 modules/libfoo";
        let branch_line = "+3f4a1c2 modules/libfoo (heads/main)";

        // Act
        let plain_path = super::parse_submodule_status_path(plain_line);
        let branch_path = super::parse_submodule_status_path(branch_line);

        // Assert
        assert_eq!(plain_path, Some("modules/libfoo"));
        assert_eq!(branch_path, Some("modules/libfoo"));
    }
}

fn command_exists(command: &str) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|dir| {
        let candidate = dir.join(command);
        std::fs::metadata(candidate)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    })
}

pub(crate) fn detect_agent_availability() -> crate::api::AgentAvailability {
    crate::api::AgentAvailability {
        claude: command_exists("claude"),
        codex: command_exists("codex"),
    }
}

pub(crate) fn agent_is_available(
    availability: crate::api::AgentAvailability,
    agent: crate::api::AgentKind,
) -> bool {
    match agent {
        crate::api::AgentKind::None => true,
        crate::api::AgentKind::Claude => availability.claude,
        crate::api::AgentKind::Codex => availability.codex,
    }
}

pub(crate) fn agent_options(
    availability: crate::api::AgentAvailability,
) -> Vec<crate::api::AgentOption> {
    [
        (crate::api::AgentKind::None, "No agent"),
        (crate::api::AgentKind::Claude, "Claude"),
        (crate::api::AgentKind::Codex, "Codex"),
    ]
    .into_iter()
    .map(|(kind, label)| crate::api::AgentOption {
        kind,
        label,
        available: agent_is_available(availability, kind),
    })
    .collect()
}
