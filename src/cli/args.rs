//! What the arguments mean: the command to run, and the review a path or revision names.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use super::{Frame, help_text_for};
use crate::{
    api::DiffTarget,
    git::{parse_review_target, run_git},
};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CliCommand {
    Help,
    Version,
    Serve {
        logs: bool,
    },
    /// Write the desktop launcher of each installed executable, so the OS offers them too.
    InstallLaunchers,
    /// The window with no repo: it asks which one to open, the same as a launcher started
    /// from the OS. This is what "New Window" in the menu bar opens.
    PickProject,
    /// The window on the repo at this path, wherever it was started from. This is what a
    /// restarted window is given, so it comes back on the repo it was on.
    OpenRepo(String),
    Review {
        target: ReviewTarget,
        logs: bool,
        frontend: Frontend,
    },
}

/// Which frontend a review opens in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Frontend {
    /// The desktop window, with the server in the same process.
    Native,
    /// A browser tab against a background server, which is how moonreview started out.
    Web,
    /// The desktop window, reviewing a repo on another machine through its `serve`.
    Remote {
        target: String,
        repo_path: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ReviewTarget {
    WorkingTree,
    CurrentDirectory,
    Path(String),
    Comparison([String; 2]),
    Diff(String),
    Commit(String),
}

#[derive(Clone)]
pub(super) struct ReviewOpenRequest {
    pub(super) diff_target: DiffTarget,
    pub(super) active_commit: Option<String>,
}

pub(super) fn parse_cli_args(args: Vec<String>, frame: Frame) -> Result<CliCommand> {
    let mut logs = false;
    let mut web = false;
    let mut pick = false;
    let mut remote: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut positional = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--logs" => logs = true,
            "--pick" => pick = true,
            "--web" => web = true,
            "--help" | "-h" | "help" => return Ok(CliCommand::Help),
            "--version" | "-v" => return Ok(CliCommand::Version),
            "--remote" => {
                remote = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--remote needs an address, e.g. --remote dev-box"))?,
                );
            }
            "--repo" => {
                repo = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--repo needs the path of a repo"))?,
                );
            }
            _ if arg.starts_with("--remote=") => {
                remote = Some(arg["--remote=".len()..].to_string());
            }
            _ if arg.starts_with("--repo=") => {
                repo = Some(arg["--repo=".len()..].to_string());
            }
            _ if arg.starts_with('-') => bail!("unknown option: {arg}\n\n{}", help_text_for(frame)),
            _ => positional.push(arg),
        }
    }

    if web && remote.is_some() {
        bail!("--web and --remote are different frontends; pick one");
    }
    // A remote window with no --repo already opens on the repo prompt, and a browser tab has
    // no launch screen to show, so --pick is the window's own and asks for nothing else.
    if pick && (web || remote.is_some() || logs || !positional.is_empty()) {
        bail!("--pick opens the window on its launch screen, so it takes nothing else");
    }
    if pick {
        return Ok(CliCommand::PickProject);
    }
    // Without --remote, --repo names the repo this window opens on, which is the whole of
    // what it was asked for: a restarted window passes it and nothing else.
    if let Some(path) = &repo
        && remote.is_none()
    {
        if web || logs || !positional.is_empty() {
            bail!("--repo opens the window on that repo, so it takes nothing else");
        }
        return Ok(CliCommand::OpenRepo(path.clone()));
    }

    let frontend = match (web, remote) {
        (true, _) => Frontend::Web,
        (false, Some(target)) => Frontend::Remote {
            target,
            repo_path: repo,
        },
        (false, None) => Frontend::Native,
    };
    let review = |target: ReviewTarget| CliCommand::Review {
        target,
        logs,
        frontend: frontend.clone(),
    };

    match positional.as_slice() {
        [] => Ok(review(ReviewTarget::WorkingTree)),
        [command] if command == "serve" => Ok(CliCommand::Serve { logs }),
        [command] if command == "install-launchers" => Ok(CliCommand::InstallLaunchers),
        [command] if command == "diff" => Ok(review(ReviewTarget::WorkingTree)),
        [command, target] if command == "diff" => Ok(review(ReviewTarget::Diff(target.clone()))),
        [target] if target == "." || target == "./" => Ok(review(ReviewTarget::CurrentDirectory)),
        [target] => Ok(review(if is_sha_like(target) {
            ReviewTarget::Commit(target.clone())
        } else {
            ReviewTarget::Path(target.clone())
        })),
        [command, ..]
            if command == "diff" || command == "serve" || command == "install-launchers" =>
        {
            bail!("{}", help_text_for(frame))
        }
        [before, after] => Ok(review(ReviewTarget::Comparison([
            before.clone(),
            after.clone(),
        ]))),
        _ => bail!("{}", help_text_for(frame)),
    }
}

pub(super) fn review_open_request(
    repo_path: &Path,
    target: ReviewTarget,
    current_dir_pathspec: Option<String>,
    current_dir: &Path,
) -> Result<ReviewOpenRequest> {
    match target {
        ReviewTarget::WorkingTree => Ok(ReviewOpenRequest {
            diff_target: DiffTarget::default(),
            active_commit: None,
        }),
        ReviewTarget::CurrentDirectory => Ok(ReviewOpenRequest {
            diff_target: DiffTarget {
                base: None,
                pathspec: current_dir_pathspec,
                comparison: None,
            },
            active_commit: None,
        }),
        ReviewTarget::Path(path) => {
            // `main..feature` is git's range syntax rather than a file. A file whose name
            // happens to contain two dots is still a file, so what is on disk wins - the same
            // way `git diff` decides between a revision and a path.
            if is_revision_range(&path) && !current_dir.join(&path).exists() {
                return Ok(ReviewOpenRequest {
                    diff_target: DiffTarget {
                        base: Some(path),
                        pathspec: None,
                        comparison: None,
                    },
                    active_commit: None,
                });
            }

            Ok(ReviewOpenRequest {
                diff_target: DiffTarget {
                    base: None,
                    pathspec: Some(repo_relative_pathspec(repo_path, current_dir, &path)?),
                    comparison: None,
                },
                active_commit: None,
            })
        }
        ReviewTarget::Comparison(paths) => Ok(ReviewOpenRequest {
            diff_target: DiffTarget {
                base: None,
                pathspec: None,
                comparison: Some(paths.map(|path| {
                    let path = Path::new(&path);
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        current_dir.join(path)
                    }
                    .display()
                    .to_string()
                })),
            },
            active_commit: None,
        }),
        ReviewTarget::Commit(commit) => {
            let Some(commit) = resolve_commit(repo_path, &commit)? else {
                return Ok(ReviewOpenRequest {
                    diff_target: parse_review_target(Some(commit))?,
                    active_commit: None,
                });
            };

            Ok(ReviewOpenRequest {
                diff_target: DiffTarget::default(),
                active_commit: Some(commit),
            })
        }
        ReviewTarget::Diff(target) => Ok(ReviewOpenRequest {
            diff_target: parse_review_target(Some(target))?,
            active_commit: None,
        }),
    }
}

pub(super) fn current_dir_pathspec(repo_path: &Path, current_dir: &Path) -> Result<Option<String>> {
    let current_dir = current_dir
        .canonicalize()
        .context("failed to resolve current directory")?;
    let relative = current_dir
        .strip_prefix(repo_path)
        .context("current directory is outside the repository")?;
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }

    Ok(Some(
        relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/"),
    ))
}

/// Whether an argument reads as one of git's revision ranges: `main..feature`, or the
/// symmetric `main...feature`. Both sides have to name something for it to be a range, which
/// is what keeps `..`, `../` and a file called `a..b` out of it.
pub(super) fn is_revision_range(value: &str) -> bool {
    let Some((left, right)) = value.split_once("..") else {
        return false;
    };
    let right = right.strip_prefix('.').unwrap_or(right);
    !left.is_empty() && !right.is_empty() && !right.starts_with('/') && !right.starts_with('.')
}

fn repo_relative_pathspec(repo_path: &Path, current_dir: &Path, path: &str) -> Result<String> {
    let path = Path::new(path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    let resolved = candidate
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&candidate));
    let relative = resolved
        .strip_prefix(repo_path)
        .context("review path is outside the repository")?;
    if relative.as_os_str().is_empty() {
        bail!("review path must identify a file or directory inside the repository");
    }

    Ok(relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn is_sha_like(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn resolve_commit(repo_path: &Path, commit: &str) -> Result<Option<String>> {
    let commit_ref = format!("{commit}^{{commit}}");
    match run_git(repo_path, &["rev-parse", "--verify", &commit_ref]) {
        Ok(resolved) => Ok(Some(resolved.trim().to_string())),
        Err(error) => {
            let message = error.to_string();
            if message.contains("Needed a single revision")
                || message.contains("unknown revision")
                || message.contains("not a valid object name")
            {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}
