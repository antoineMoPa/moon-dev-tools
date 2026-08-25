use std::{
    collections::HashSet,
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use crate::{
    agent::ChildExt,
    api::{
        CommitView, DiffHunk, DiffTarget, FileChangeKind, ImageDiffView, RepoSession, stable_id,
    },
};

const BINARY_DETECTION_READ_LIMIT: u64 = 8192;

pub(crate) fn canonicalize_repo(path: impl AsRef<Path>) -> Result<PathBuf> {
    let original_path = path.as_ref().to_path_buf();
    match find_repo_root(&original_path)? {
        Some(repo_path) => Ok(repo_path),
        None => bail!("{} is not inside a git repository", original_path.display()),
    }
}

/// The repo a path sits in, or `None` when it sits in no repo at all — which is an answer
/// rather than a failure for a window that can ask which repo to open.
pub(crate) fn find_repo_root(path: impl AsRef<Path>) -> Result<Option<PathBuf>> {
    let mut path = path
        .as_ref()
        .canonicalize()
        .context("failed to resolve path")?;

    loop {
        if path.join(".git").exists() {
            return Ok(Some(path));
        }
        if !path.pop() {
            return Ok(None);
        }
    }
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
    if let Some([before, after]) = &diff_target.comparison {
        let diff = run_git_allow_status(
            repo_path,
            &[
                "diff",
                "--no-index",
                "--diff-algorithm=histogram",
                "--no-color",
                "--no-prefix",
                "--unified=3",
                "--",
                before,
                after,
            ],
            &[0, 1],
        )?;
        let mut hunks = parse_diff(repo_path, &diff, false)?;
        for hunk in &mut hunks {
            hunk.file_path = after.clone();
        }
        return Ok(hunks);
    }

    if let Some(base) = &diff_target.base {
        let diff = run_target_diff(repo_path, base, diff_target.pathspec.as_deref())?;
        return parse_diff(repo_path, &diff, false);
    }

    let pathspec = diff_target
        .pathspec
        .as_deref()
        .filter(|value| !value.is_empty());
    let mut unstaged_args = vec![
        "diff",
        "--diff-algorithm=histogram",
        "--no-color",
        "--unified=3",
    ];
    append_pathspec(&mut unstaged_args, pathspec);
    let mut hunks = parse_diff(repo_path, &run_git(repo_path, &unstaged_args)?, false)?;

    let mut staged_args = vec![
        "diff",
        "--cached",
        "--diff-algorithm=histogram",
        "--no-color",
        "--unified=3",
    ];
    append_pathspec(&mut staged_args, pathspec);
    hunks.extend(parse_diff(
        repo_path,
        &run_git(repo_path, &staged_args)?,
        true,
    )?);
    for path in list_untracked_files(repo_path, pathspec)? {
        let full_path = repo_path.join(&path);
        let is_binary = match is_likely_binary_file(&full_path) {
            Ok(is_binary) => is_binary,
            Err(_) => continue,
        };
        if is_binary {
            if let Some(mime_type) = image_mime_type(&path) {
                let Ok(data) = fs::read(&full_path) else {
                    continue;
                };
                let image_diff = ImageDiffView {
                    before_src: None,
                    after_src: Some(data_uri(mime_type, &data)),
                };
                let patch = format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\nBinary image added\n"
                );
                let id = stable_id(&(path.clone(), "Binary image added", patch.clone()));
                hunks.push(DiffHunk {
                    id,
                    file_path: path,
                    change_kind: FileChangeKind::Added,
                    header: "Binary image added".to_string(),
                    patch,
                    staged: false,
                    image_diff: Some(image_diff),
                });
            }
            continue;
        }
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
        hunks.extend(parse_diff(repo_path, &diff, false)?);
    }
    Ok(hunks)
}

fn is_likely_binary_file(path: &Path) -> Result<bool> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    let mut buffer = Vec::new();
    file.take(BINARY_DETECTION_READ_LIMIT)
        .read_to_end(&mut buffer)
        .with_context(|| format!("failed to inspect {}", path.display()))?;

    Ok(buffer.contains(&0) || std::str::from_utf8(&buffer).is_err())
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
    parse_diff(repo_path, &diff, true)
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
    if !git_ref_exists(repo_path, "HEAD")? {
        return Ok((None, Vec::new()));
    }

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

pub(crate) fn commit_history_page(
    repo_path: &Path,
    excluded_shas: &HashSet<String>,
    offset: usize,
    limit: usize,
) -> Result<(Vec<CommitView>, bool)> {
    if limit == 0 {
        return Ok((Vec::new(), false));
    }
    if !git_ref_exists(repo_path, "HEAD")? {
        return Ok((Vec::new(), false));
    }

    let format = "%H%x1f%h%x1f%an%x1f%s";
    let output = run_git(repo_path, &["log", &format!("--format={format}")])?;
    let mut skipped = 0usize;
    let mut commits = Vec::new();
    let mut has_more = false;

    for commit in output.lines().filter_map(parse_commit_view) {
        if excluded_shas.contains(&commit.sha) {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        if commits.len() >= limit {
            has_more = true;
            break;
        }
        commits.push(commit);
    }

    Ok((commits, has_more))
}

pub(crate) fn commit_view(repo_path: &Path, commit: &str) -> Result<Option<CommitView>> {
    if commit.trim().is_empty() {
        return Ok(None);
    }

    let format = "%H%x1f%h%x1f%an%x1f%s";
    let output = run_git_allow_status(
        repo_path,
        &["show", "-s", &format!("--format={format}"), commit],
        &[0, 128],
    )?;
    Ok(output.lines().find_map(parse_commit_view))
}

fn default_branch_ref(repo_path: &Path) -> Result<Option<String>> {
    if let Some(origin_head) = origin_head_ref(repo_path)? {
        return Ok(Some(origin_head));
    }

    if let Some(upstream) = current_branch_upstream_ref(repo_path)? {
        return Ok(Some(upstream));
    }

    Ok(None)
}

fn origin_head_ref(repo_path: &Path) -> Result<Option<String>> {
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

pub(crate) fn current_branch_upstream_ref(repo_path: &Path) -> Result<Option<String>> {
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

/// Write a file in the working tree. Only a file that is already there can be written: this
/// is an editor for what is being reviewed, not a way to create files anywhere on disk.
pub(crate) fn write_repo_file(repo_path: &Path, file_path: &str, content: &str) -> Result<()> {
    if file_path.trim().is_empty() {
        bail!("file path cannot be empty");
    }

    // Both sides are resolved before they are compared: on macOS the repo may be reached
    // through a symlink (`/var` for `/private/var`), and comparing a resolved path against an
    // unresolved root would refuse a file that is plainly inside it.
    let repo_root = repo_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", repo_path.display()))?;
    let resolved = repo_root
        .join(file_path)
        .canonicalize()
        .with_context(|| format!("failed to resolve {file_path}"))?;
    if !resolved.starts_with(&repo_root) {
        bail!("file path is outside the repository");
    }
    if !resolved.is_file() {
        bail!("{file_path} is not a file in the working tree");
    }

    fs::write(&resolved, content)
        .with_context(|| format!("failed to write {}", resolved.display()))
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

pub(crate) fn append_pathspec<'a>(args: &mut Vec<&'a str>, pathspec: Option<&'a str>) {
    if let Some(pathspec) = pathspec.filter(|value| !value.is_empty()) {
        args.push("--");
        args.push(pathspec);
    }
}

fn parse_diff(repo_path: &Path, diff: &str, staged: bool) -> Result<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    for section in split_diff_sections(diff) {
        let file_path = parse_file_path(&section).unwrap_or_else(|| "unknown".to_string());
        let change_kind = parse_change_kind(&section);
        let image_diff = image_diff_from_section(repo_path, &file_path, &section)?;
        let mut prelude = Vec::new();
        let mut idx = 0usize;

        while idx < section.len() && !section[idx].starts_with("@@") {
            prelude.push(section[idx].clone());
            idx += 1;
        }

        if idx == section.len() && image_diff.is_some() {
            let header = match change_kind {
                FileChangeKind::Added => "Binary image added",
                FileChangeKind::Deleted => "Binary image deleted",
                FileChangeKind::Modified => "Binary image changed",
            }
            .to_string();
            let patch = format!("{}\n{}\n", prelude.join("\n"), header);
            let id = stable_id(&(file_path.clone(), header.clone(), patch.clone()));
            hunks.push(DiffHunk {
                id,
                file_path: file_path.clone(),
                change_kind,
                header,
                patch,
                staged,
                image_diff: image_diff.clone(),
            });
        }

        // An image git can still text-diff — an SVG — is one change to look at, not a card
        // per run of markup: the card shows the before/after pictures, so a hunk per `@@`
        // would repeat the same pair. The whole file section becomes the one hunk, which is
        // itself a complete patch, so staging it stages the file's change whole.
        if image_diff.is_some() && idx < section.len() {
            let header = match change_kind {
                FileChangeKind::Added => "Image added",
                FileChangeKind::Deleted => "Image deleted",
                FileChangeKind::Modified => "Image changed",
            }
            .to_string();
            let patch = format!("{}\n", section.join("\n"));
            let id = stable_id(&(file_path.clone(), header.clone(), patch.clone()));
            hunks.push(DiffHunk {
                id,
                file_path: file_path.clone(),
                change_kind,
                header,
                patch,
                staged,
                image_diff: image_diff.clone(),
            });
            continue;
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
                image_diff: image_diff.clone(),
            });
        }
    }

    Ok(hunks)
}

fn image_diff_from_section(
    repo_path: &Path,
    file_path: &str,
    section: &[String],
) -> Result<Option<ImageDiffView>> {
    let Some(mime_type) = image_mime_type(file_path) else {
        return Ok(None);
    };

    let Some((old_blob, new_blob)) = parse_index_blobs(section) else {
        return Ok(None);
    };

    let before_src = image_blob_data_uri(repo_path, old_blob.as_deref(), mime_type, None)?;
    let after_src =
        image_blob_data_uri(repo_path, new_blob.as_deref(), mime_type, Some(file_path))?;
    Ok(Some(ImageDiffView {
        before_src,
        after_src,
    }))
}

fn parse_index_blobs(section: &[String]) -> Option<(Option<&str>, Option<&str>)> {
    let line = section
        .iter()
        .find_map(|line| line.strip_prefix("index "))?;
    let blobs = line.split_whitespace().next()?;
    let (old_blob, new_blob) = blobs.split_once("..")?;
    Some((non_zero_blob(old_blob), non_zero_blob(new_blob)))
}

fn non_zero_blob(blob: &str) -> Option<&str> {
    if blob.chars().all(|ch| ch == '0') {
        None
    } else {
        Some(blob)
    }
}

fn image_blob_data_uri(
    repo_path: &Path,
    blob: Option<&str>,
    mime_type: &str,
    fallback_path: Option<&str>,
) -> Result<Option<String>> {
    let Some(blob) = blob else {
        return Ok(None);
    };
    let bytes = match run_git_bytes(repo_path, &["show", blob]) {
        Ok(bytes) => bytes,
        Err(error) => {
            let Some(fallback_path) = fallback_path else {
                return Err(error);
            };
            fs::read(repo_path.join(fallback_path))
                .with_context(|| format!("failed to read {}", fallback_path))?
        }
    };
    Ok(Some(data_uri(mime_type, &bytes)))
}

fn data_uri(mime_type: &str, data: &[u8]) -> String {
    format!("data:{mime_type};base64,{}", BASE64.encode(data))
}

fn image_mime_type(path: &str) -> Option<&'static str> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "apng" => Some("image/apng"),
        "avif" => Some("image/avif"),
        "gif" => Some("image/gif"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "svg" => Some("image/svg+xml"),
        "webp" => Some("image/webp"),
        _ => None,
    }
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
        if let Some(path) = line
            .strip_prefix("+++ ")
            .filter(|path| *path != "/dev/null")
        {
            return Some(path.strip_prefix("b/").unwrap_or(path).to_string());
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

fn list_untracked_files(repo_path: &Path, pathspec: Option<&str>) -> Result<Vec<String>> {
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard"];
    append_pathspec(&mut args, pathspec);
    let output = run_git_bytes(repo_path, &args)?;
    Ok(output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

pub(crate) fn local_change_summary_from_status(
    repo_path: &Path,
    pathspec: Option<&str>,
) -> Result<crate::api::LocalChangeSummary> {
    let mut args = vec!["status", "--porcelain"];
    append_pathspec(&mut args, pathspec);
    let status = run_git(repo_path, &args)?;
    let mut summary = crate::api::LocalChangeSummary::default();

    for line in status.lines() {
        if line.len() < 3 {
            continue;
        }

        let status_code = &line[..2];
        if status_code == "??" || status_code.contains('A') {
            summary.added += 1;
        } else if status_code.contains('D') {
            summary.deleted += 1;
        } else {
            summary.modified += 1;
        }
    }

    Ok(summary)
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

pub(crate) fn run_git_allow_status(
    repo_path: &Path,
    args: &[&str],
    allowed: &[i32],
) -> Result<String> {
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

pub(crate) fn run_git_bytes(repo_path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(output.stdout)
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
            comparison: None,
        });
    }

    Ok(DiffTarget {
        base: Some(value),
        pathspec: None,
        comparison: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        branch_commits_since_default, canonicalize_repo, collect_commit_hunks, collect_hunks,
        collect_session_hunks, commit_history_page, local_change_summary_from_status, run_git,
        run_git_no_output,
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
            commit_statuses: HashMap::new(),
            selected_agent: AgentKind::None,
            comment_dispatches: HashMap::new(),
        }
    }

    /// An SVG is reviewed as its picture, so however many `@@` runs git finds in the markup,
    /// the file is one change — and that one hunk's patch is the whole file section, which
    /// `git apply` takes as-is, so staging it stages the change whole.
    #[test]
    fn an_svg_is_one_hunk_however_many_places_it_changed_in() {
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        // Two edits far enough apart that git would otherwise split them into two hunks.
        let spacer = "  <rect width=\"1\" height=\"1\"/>\n".repeat(12);
        let svg = |first: &str, last: &str| {
            format!("<svg xmlns=\"http://www.w3.org/2000/svg\">\n  {first}\n{spacer}  {last}\n</svg>\n")
        };
        fs::write(repo_root.join("logo.svg"), svg("<g id=\"a\"/>", "<g id=\"z\"/>"))
            .expect("failed to write the svg");
        run_git_no_output(&repo_root, &["add", "logo.svg"]).expect("failed to add the svg");
        run_git_no_output(&repo_root, &["commit", "-m", "Add the logo"])
            .expect("failed to commit");
        fs::write(repo_root.join("logo.svg"), svg("<g id=\"b\"/>", "<g id=\"y\"/>"))
            .expect("failed to change the svg");

        let hunks = collect_hunks(&repo_root, &DiffTarget::default())
            .expect("failed to collect the hunks");

        let svg_hunks: Vec<_> = hunks
            .iter()
            .filter(|hunk| hunk.file_path == "logo.svg")
            .collect();
        assert_eq!(
            svg_hunks.len(),
            1,
            "an image file is one change, not a card per run of markup"
        );
        let hunk = svg_hunks[0];
        assert_eq!(hunk.header, "Image changed");
        assert!(hunk.image_diff.is_some(), "the card shows the pictures");
        assert!(
            hunk.patch.matches("@@").count() >= 2,
            "both edits are in the one patch: {}",
            hunk.patch
        );

        super::apply_patch(&repo_root, &hunk.patch, true, false)
            .expect("the whole-file patch should stage cleanly");
        let staged = run_git(&repo_root, &["diff", "--cached", "--name-only"])
            .expect("failed to list staged files");
        assert!(staged.contains("logo.svg"), "staging the hunk stages the file");
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
    fn collect_hunks_compares_two_files_without_a_git_repository() {
        // Arrange
        let temp = TestDir::new();
        let before = temp.path.join("before.json");
        let after = temp.path.join("after.json");
        fs::write(&before, "{\"value\": 1}\n").expect("failed to write before file");
        fs::write(&after, "{\"value\": 2}\n").expect("failed to write after file");
        let target = DiffTarget {
            base: None,
            pathspec: None,
            comparison: Some([before.display().to_string(), after.display().to_string()]),
        };

        // Act
        let hunks = collect_hunks(&temp.path, &target).expect("failed to compare files");

        // Assert
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, after.display().to_string());
        assert!(!hunks[0].staged);
        assert_eq!(diff_line_counts(&hunks[0].patch), (1, 1));
    }

    #[test]
    fn initial_staged_changes_are_available_before_first_commit() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        fs::write(repo_root.join("example.txt"), "initial contents\n")
            .expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add initial file");

        // Act
        let hunks = collect_hunks(&repo_root, &DiffTarget::default())
            .expect("failed to collect initial staged hunks");
        let (base, branch_commits) =
            branch_commits_since_default(&repo_root).expect("failed to collect branch commits");
        let (history_commits, history_has_more) =
            commit_history_page(&repo_root, &HashSet::new(), 0, 50)
                .expect("failed to collect commit history");

        // Assert
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "example.txt");
        assert!(hunks[0].staged);
        assert_eq!(diff_line_counts(&hunks[0].patch), (1, 0));
        assert_eq!(base, None);
        assert!(branch_commits.is_empty());
        assert!(history_commits.is_empty());
        assert!(!history_has_more);
    }

    /// `moonreview main..feature` reviews everything on one branch that is not on the other.
    /// The range goes to git as it was typed; nothing here has to understand it.
    #[test]
    fn a_file_in_the_working_tree_can_be_written_back() {
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);
        fs::write(repo_root.join("lib.rs"), "fn one() {}\n").expect("failed to write");

        super::write_repo_file(&repo_root, "lib.rs", "fn two() {}\n").expect("expected the write");

        assert_eq!(
            fs::read_to_string(repo_root.join("lib.rs")).expect("failed to read back"),
            "fn two() {}\n"
        );
    }

    #[test]
    fn writing_outside_the_repository_is_refused() {
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);
        fs::write(temp.path.join("outside.txt"), "secrets\n").expect("failed to write");

        let refused = super::write_repo_file(&repo_root, "../outside.txt", "changed\n");

        assert!(refused.is_err(), "a path out of the repo must be refused");
        assert_eq!(
            fs::read_to_string(temp.path.join("outside.txt")).expect("failed to read back"),
            "secrets\n",
            "and must not have written anything"
        );
    }

    #[test]
    fn a_revision_range_collects_the_hunks_between_two_branches() {
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);
        fs::write(repo_root.join("lib.rs"), "fn one() {}\n").expect("failed to write");
        run_git_no_output(&repo_root, &["add", "-A"]).expect("failed to stage");
        run_git_no_output(&repo_root, &["commit", "-m", "first"]).expect("failed to commit");
        run_git_no_output(&repo_root, &["branch", "-M", "main"]).expect("failed to name main");
        run_git_no_output(&repo_root, &["checkout", "-b", "feature"])
            .expect("failed to branch");
        fs::write(repo_root.join("lib.rs"), "fn one() {}\nfn two() {}\n")
            .expect("failed to write");
        run_git_no_output(&repo_root, &["add", "-A"]).expect("failed to stage");
        run_git_no_output(&repo_root, &["commit", "-m", "second"]).expect("failed to commit");

        let mut session = test_session(repo_root, None);
        session.diff_target = DiffTarget {
            base: Some("main..feature".to_string()),
            pathspec: None,
            comparison: None,
        };

        let hunks = collect_session_hunks(&session).expect("expected the range to diff");

        assert_eq!(hunks.len(), 1, "the branch adds one hunk");
        assert!(
            hunks[0].patch.contains("fn two()"),
            "the hunk should be the line the branch added, got:\n{}",
            hunks[0].patch
        );
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
    fn branch_commits_since_default_prefers_origin_head_over_current_branch_upstream() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let file_path = repo_root.join("example.txt");
        fs::write(&file_path, "base\n").expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");
        let default_head =
            run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");
        run_git_no_output(&repo_root, &["remote", "add", "origin", "."])
            .expect("failed to add remote");
        run_git_no_output(
            &repo_root,
            &["update-ref", "refs/remotes/origin/dev", default_head.trim()],
        )
        .expect("failed to create remote default ref");
        run_git_no_output(
            &repo_root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/dev",
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
        assert_eq!(base.as_deref(), Some("origin/dev"));
        assert_eq!(
            commits
                .iter()
                .map(|commit| commit.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["second change", "first change"]
        );
    }

    #[test]
    fn branch_commits_since_default_falls_back_to_current_branch_upstream() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let file_path = repo_root.join("example.txt");
        fs::write(&file_path, "base\n").expect("failed to write initial file");
        run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit initial file");

        run_git_no_output(&repo_root, &["remote", "add", "origin", "."])
            .expect("failed to add remote");
        run_git_no_output(&repo_root, &["checkout", "-b", "feature"])
            .expect("failed to create feature branch");
        fs::write(&file_path, "base\none\n").expect("failed to write feature change");
        run_git_no_output(&repo_root, &["add", "example.txt"])
            .expect("failed to add feature change");
        run_git_no_output(&repo_root, &["commit", "-m", "feature change"])
            .expect("failed to commit feature change");
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
    fn collect_hunks_skips_untracked_binary_files() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        fs::write(repo_root.join("note.txt"), "reviewable\ntext\n")
            .expect("failed to write text file");
        fs::write(repo_root.join("asset.bin"), [0, 159, 146, 150, 255])
            .expect("failed to write binary file");

        // Act
        let hunks =
            collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");

        // Assert
        assert!(hunks.iter().any(|hunk| hunk.file_path == "note.txt"));
        assert!(!hunks.iter().any(|hunk| hunk.file_path == "asset.bin"));
    }

    #[test]
    fn working_tree_pathspec_limits_hunks_and_status_summary() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        fs::create_dir_all(repo_root.join("src")).expect("failed to create src directory");
        fs::create_dir_all(repo_root.join("docs")).expect("failed to create docs directory");
        fs::write(repo_root.join("src/tracked.txt"), "before\n").expect("failed to write src file");
        fs::write(repo_root.join("docs/tracked.txt"), "before\n")
            .expect("failed to write docs file");
        run_git_no_output(&repo_root, &["add", "src/tracked.txt", "docs/tracked.txt"])
            .expect("failed to add tracked files");
        run_git_no_output(&repo_root, &["commit", "-m", "initial"])
            .expect("failed to commit tracked files");

        fs::write(repo_root.join("src/tracked.txt"), "after\n").expect("failed to modify src file");
        fs::write(repo_root.join("docs/tracked.txt"), "after\n")
            .expect("failed to modify docs file");
        fs::write(repo_root.join("src/new.txt"), "new\n").expect("failed to write src new file");
        fs::write(repo_root.join("docs/new.txt"), "new\n").expect("failed to write docs new file");

        // Act
        let hunks = collect_hunks(
            &repo_root,
            &DiffTarget {
                base: None,
                pathspec: Some("src".to_string()),
                comparison: None,
            },
        )
        .expect("failed to collect hunks");
        let summary = local_change_summary_from_status(&repo_root, Some("src"))
            .expect("failed to collect status summary");

        // Assert
        let paths = hunks
            .iter()
            .map(|hunk| hunk.file_path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"src/tracked.txt"));
        assert!(paths.contains(&"src/new.txt"));
        assert!(!paths.iter().any(|path| path.starts_with("docs/")));
        assert_eq!(summary.modified, 1);
        assert_eq!(summary.added, 1);
        assert_eq!(summary.deleted, 0);
    }

    #[test]
    fn collect_hunks_handles_untracked_image_paths_with_non_ascii_characters() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        let image_path = "tmp-images-upload/502 chiné gris dos.webp";
        fs::create_dir_all(repo_root.join("tmp-images-upload"))
            .expect("failed to create image directory");
        fs::write(repo_root.join(image_path), b"RIFF\0\0\0\0WEBPVP8 ")
            .expect("failed to write image file");

        // Act
        let hunks =
            collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.file_path == image_path)
            .expect("expected image hunk");

        // Assert
        assert_eq!(hunk.header, "Binary image added");
        assert!(
            hunk.image_diff
                .as_ref()
                .and_then(|image_diff| image_diff.after_src.as_deref())
                .is_some_and(|src| src.starts_with("data:image/webp;base64,"))
        );
    }

    #[test]
    fn collect_hunks_includes_image_diff_for_unstaged_binary_image() {
        // Arrange
        let temp = TestDir::new();
        let repo_root = temp.path.join("repo");
        init_test_repo(&repo_root);

        fs::write(repo_root.join("asset.png"), b"\x89PNG\r\n\x1a\n\0before")
            .expect("failed to write initial image");
        run_git_no_output(&repo_root, &["add", "asset.png"]).expect("failed to add image");
        run_git_no_output(&repo_root, &["commit", "-m", "initial image"])
            .expect("failed to commit initial image");
        fs::write(repo_root.join("asset.png"), b"\x89PNG\r\n\x1a\n\0after")
            .expect("failed to modify image");

        // Act
        let hunks =
            collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.file_path == "asset.png")
            .expect("expected image hunk");

        // Assert
        let image_diff = hunk.image_diff.as_ref().expect("expected image diff");
        assert!(
            image_diff
                .before_src
                .as_deref()
                .is_some_and(|src| src.starts_with("data:image/png;base64,"))
        );
        assert!(
            image_diff
                .after_src
                .as_deref()
                .is_some_and(|src| src.starts_with("data:image/png;base64,"))
        );
        assert_eq!(hunk.header, "Binary image changed");
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
    env::split_paths(crate::shell_path::agent_path()).any(|dir| {
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
        opencode: command_exists("opencode"),
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
        crate::api::AgentKind::OpenCode => availability.opencode,
    }
}

pub(crate) fn agent_options(
    availability: crate::api::AgentAvailability,
) -> Vec<crate::api::AgentOption> {
    [
        (crate::api::AgentKind::None, "No agent"),
        (crate::api::AgentKind::Claude, "Claude"),
        (crate::api::AgentKind::Codex, "Codex"),
        (crate::api::AgentKind::OpenCode, "OpenCode"),
    ]
    .into_iter()
    .map(|(kind, label)| crate::api::AgentOption {
        kind,
        label: label.to_string(),
        available: agent_is_available(availability, kind),
    })
    .collect()
}
