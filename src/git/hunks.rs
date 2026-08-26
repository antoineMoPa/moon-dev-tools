//! Turning a git diff into the hunks the review draws, and turning a selection back into a
//! patch git can apply.

use std::{
    fs,
    io::Read,
    path::Path,
    process::Stdio,
};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

use super::{append_pathspec, git_command, run_git, run_git_allow_status, run_git_bytes};
use crate::{
    agent::ChildExt,
    api::{DiffHunk, DiffTarget, FileChangeKind, ImageDiffView, RepoSession, stable_id},
};

const BINARY_DETECTION_READ_LIMIT: u64 = 8192;

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

        // An image git can still text-diff - an SVG - is one change to look at, not a card
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
    let mut command = git_command(repo_path);
    command.arg("apply");
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
