//! Finding a repository, running git inside it, and reading back its files and commits.

mod hunks;
#[cfg(test)]
mod tests;

pub(crate) use hunks::{
    apply_patch, build_partial_patch_from_selection, collect_session_hunks,
    local_change_summary_from_status, preview_patch,
};

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use crate::api::{CommitView, DiffTarget};

/// A git command that will not write the index just to read from the repo.
///
/// The review window re-reads the diff every second or so, and a plain `git status` or
/// `git diff` refreshes the on-disk index as it goes, which means taking `.git/index.lock`.
/// A person running `git commit` in the same repo at that moment gets "Unable to create
/// index.lock" from their own command. `GIT_OPTIONAL_LOCKS=0` drops only the locks git takes
/// for its own bookkeeping - the ones `git add` or `git commit` need to do their work are
/// still taken - so every git call here goes through this.
fn git_command(repo_path: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(repo_path).env("GIT_OPTIONAL_LOCKS", "0");
    command
}

pub(crate) fn canonicalize_repo(path: impl AsRef<Path>) -> Result<PathBuf> {
    let original_path = path.as_ref().to_path_buf();
    match find_repo_root(&original_path)? {
        Some(repo_path) => Ok(repo_path),
        None => bail!("{} is not inside a git repository", original_path.display()),
    }
}

/// The repo a path sits in, or `None` when it sits in no repo at all - which is an answer
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

pub(crate) fn append_pathspec<'a>(args: &mut Vec<&'a str>, pathspec: Option<&'a str>) {
    if let Some(pathspec) = pathspec.filter(|value| !value.is_empty()) {
        args.push("--");
        args.push(pathspec);
    }
}

pub(crate) fn run_git_allow_status(
    repo_path: &Path,
    args: &[&str],
    allowed: &[i32],
) -> Result<String> {
    let output = git_command(repo_path)
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
    let output = git_command(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(output.stdout)
}

pub(crate) fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(repo_path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn run_git_no_output(repo_path: &Path, args: &[&str]) -> Result<()> {
    let output = git_command(repo_path)
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

/// Where a plain `git push` would send the current branch, when git can tell from its config.
///
/// `None` when it cannot: a branch with no upstream, or - under `push.default=simple`, git's
/// default - one whose upstream is not named like it. No check that the ref exists: the first
/// push under `push.default=current` goes to a branch the remote has not got yet, and git
/// still knows where that is.
pub(crate) fn current_branch_push_ref(repo_path: &Path) -> Result<Option<String>> {
    let push_ref = run_git_allow_status(
        repo_path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{push}"],
        &[0, 128],
    )?;
    let push_ref = push_ref.trim();
    if push_ref.is_empty() {
        Ok(None)
    } else {
        Ok(Some(push_ref.to_string()))
    }
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
