use std::{
    env,
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;

use crate::{
    api::{DiffTarget, OpenSessionRequest, SessionOpened, client_host, port, server_url},
    git::{canonicalize_repo, list_changed_submodule_repos, parse_review_target, run_git},
    server,
};

#[derive(Debug, PartialEq, Eq)]
enum CliCommand {
    Help,
    Version,
    Serve {
        logs: bool,
    },
    Review {
        target: ReviewTarget,
        logs: bool,
        no_submodules: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum ReviewTarget {
    WorkingTree,
    CurrentDirectory,
    Diff(String),
    Commit(String),
}

#[derive(Clone)]
struct ReviewOpenRequest {
    diff_target: DiffTarget,
    active_commit: Option<String>,
}

pub(crate) fn run() -> Result<()> {
    match parse_cli_args(env::args().skip(1).collect::<Vec<_>>())? {
        CliCommand::Help => {
            print_help();
            Ok(())
        }
        CliCommand::Version => {
            print_version();
            Ok(())
        }
        CliCommand::Serve { logs } => {
            if logs {
                eprintln!("Moon Review server logs enabled.");
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            runtime.block_on(server::run_server())
        }
        CliCommand::Review {
            target,
            logs,
            no_submodules,
        } => launch_review(target, logs, no_submodules),
    }
}

fn launch_review(target: ReviewTarget, logs: bool, no_submodules: bool) -> Result<()> {
    let current_dir = env::current_dir()?;
    let repo_path = canonicalize_repo(&current_dir)?;
    let current_dir_pathspec = current_dir_pathspec(&repo_path, &current_dir)?;
    let open_request = review_open_request(&repo_path, target, current_dir_pathspec)?;
    if logs {
        return launch_review_with_foreground_server(repo_path, open_request, no_submodules);
    }

    ensure_server_running(logs)?;
    open_review_session(&repo_path, &open_request, no_submodules)?;
    Ok(())
}

fn current_dir_pathspec(repo_path: &Path, current_dir: &Path) -> Result<Option<String>> {
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

fn review_open_request(
    repo_path: &Path,
    target: ReviewTarget,
    current_dir_pathspec: Option<String>,
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

fn launch_review_with_foreground_server(
    repo_path: PathBuf,
    open_request: ReviewOpenRequest,
    no_submodules: bool,
) -> Result<()> {
    if server_is_running()? {
        bail!("moonreview server already running; stop it first to use --logs in the foreground");
    }

    println!("Moon Review server logs attached to this terminal. Press Ctrl+C to stop.");
    let server_thread = thread::spawn(|| -> Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime")?;
        runtime.block_on(server::run_server())
    });

    for _ in 0..30 {
        if server_is_running()? {
            open_review_session(&repo_path, &open_request, no_submodules)?;
            return server_thread
                .join()
                .map_err(|_| anyhow!("review server thread panicked"))?;
        }
        thread::sleep(Duration::from_millis(150));
    }

    bail!("review server did not become ready on {}", server_url())
}

fn open_review_session(
    repo_path: &Path,
    open_request: &ReviewOpenRequest,
    no_submodules: bool,
) -> Result<()> {
    let extra_repo_paths = if open_request.diff_target.base.is_none()
        && open_request.diff_target.pathspec.is_none()
        && open_request.active_commit.is_none()
        && !no_submodules
    {
        list_changed_submodule_repos(repo_path)?
    } else {
        Vec::new()
    };

    let mut opened_urls = Vec::new();
    opened_urls.push(open_review_url_for_session(repo_path, open_request)?);
    for submodule_path in extra_repo_paths {
        opened_urls.push(open_review_url_for_session(&submodule_path, open_request)?);
    }

    for url in &opened_urls {
        webbrowser::open(url).context("failed to open browser")?;
        println!("Opened {url}");
    }

    Ok(())
}

fn open_review_url_for_session(
    repo_path: &Path,
    open_request: &ReviewOpenRequest,
) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("failed to create client")?;

    let opened: SessionOpened = client
        .post(format!("{}/api/session/open", server_url()))
        .json(&OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(open_request.diff_target.clone()),
            active_commit: open_request.active_commit.clone(),
        })
        .send()
        .context("failed to connect to review server")?
        .error_for_status()
        .context("server refused to open session")?
        .json()
        .context("failed to decode session response")?;

    Ok(format!("{}/review/{}", server_url(), opened.session_id))
}

fn parse_cli_args(args: Vec<String>) -> Result<CliCommand> {
    let mut logs = false;
    let mut no_submodules = false;
    let mut positional = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--logs" => logs = true,
            "-ns" | "--no-submodules" => no_submodules = true,
            "--help" | "-h" | "help" => return Ok(CliCommand::Help),
            "--version" | "-v" => return Ok(CliCommand::Version),
            _ if arg.starts_with('-') => bail!("unknown option: {arg}\n\n{}", help_text()),
            _ => positional.push(arg),
        }
    }

    match positional.as_slice() {
        [] => Ok(CliCommand::Review {
            target: ReviewTarget::WorkingTree,
            logs,
            no_submodules,
        }),
        [command] if command == "serve" && no_submodules => {
            bail!(
                "--no-submodules only applies when opening a review\n\n{}",
                help_text()
            )
        }
        [command] if command == "serve" => Ok(CliCommand::Serve { logs }),
        [command] if command == "diff" => Ok(CliCommand::Review {
            target: ReviewTarget::WorkingTree,
            logs,
            no_submodules,
        }),
        [command, target] if command == "diff" => Ok(CliCommand::Review {
            target: ReviewTarget::Diff(target.clone()),
            logs,
            no_submodules,
        }),
        [target] if target == "." || target == "./" => Ok(CliCommand::Review {
            target: ReviewTarget::CurrentDirectory,
            logs,
            no_submodules,
        }),
        [target] => Ok(CliCommand::Review {
            target: if is_sha_like(target) {
                ReviewTarget::Commit(target.clone())
            } else {
                ReviewTarget::Diff(target.clone())
            },
            logs,
            no_submodules,
        }),
        _ => bail!("{}", help_text()),
    }
}

fn print_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    "moonreview

Tiny local code review UI for git.

Usage:
  moonreview
  moonreview .
  moonreview <commit>
  moonreview -ns
  moonreview --logs
  moonreview diff <target>
  moonreview diff <target> -ns
  moonreview diff <target> --logs
  moonreview serve --logs
  moonreview --version
  moonreview --help

Examples:
  moonreview
  moonreview .
  moonreview 4542abe
  moonreview diff dev
  moonreview diff dev:./

Run `moonreview` inside any git repository you want to review.
Run `moonreview .` to review only the current directory.

Use `--logs` to run the server in the foreground and print agent/failure logs until you stop it with Ctrl+C.
Use `-ns` or `--no-submodules` to open only the current repository when changed submodules are present.

`moonreview <commit>` opens a read-only review of a single commit.
`moonreview diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`."
}

fn print_version() {
    println!("moonreview {}", env!("MOONREVIEW_VERSION"));
}

fn ensure_server_running(logs: bool) -> Result<()> {
    if server_is_running()? {
        if logs {
            eprintln!(
                "moonreview server already running; restart it to attach logs to this terminal"
            );
        }
        return Ok(());
    }

    let exe = env::current_exe().context("failed to locate current executable")?;
    let mut command = Command::new(exe);
    command.arg("serve").stdin(Stdio::null());
    if !logs {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    command.spawn().context("failed to spawn review server")?;

    if logs {
        println!("Moon Review server logs attached to this terminal.");
    }

    for _ in 0..30 {
        if server_is_running()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }

    bail!("review server did not become ready on {}", server_url())
}

fn server_is_running() -> Result<bool> {
    Ok(TcpStream::connect((client_host(), port()?)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> CliCommand {
        parse_cli_args(args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected CLI args to parse")
    }

    #[test]
    fn parse_no_submodules_short_option_for_default_review() {
        assert_eq!(
            parse(&["-ns"]),
            CliCommand::Review {
                target: ReviewTarget::WorkingTree,
                logs: false,
                no_submodules: true,
            }
        );
    }

    #[test]
    fn parse_dot_as_current_directory_review() {
        assert_eq!(
            parse(&["."]),
            CliCommand::Review {
                target: ReviewTarget::CurrentDirectory,
                logs: false,
                no_submodules: false,
            }
        );
    }

    #[test]
    fn working_tree_review_ignores_current_directory_pathspec() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::WorkingTree,
            Some("src".to_string()),
        )
        .expect("expected review request");

        assert_eq!(request.diff_target.base, None);
        assert_eq!(request.diff_target.pathspec, None);
        assert_eq!(request.active_commit, None);
    }

    #[test]
    fn current_directory_review_uses_current_directory_pathspec() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::CurrentDirectory,
            Some("src".to_string()),
        )
        .expect("expected review request");

        assert_eq!(request.diff_target.base, None);
        assert_eq!(request.diff_target.pathspec.as_deref(), Some("src"));
        assert_eq!(request.active_commit, None);
    }

    #[test]
    fn parse_no_submodules_long_option_for_diff_review() {
        assert_eq!(
            parse(&["diff", "dev", "--no-submodules"]),
            CliCommand::Review {
                target: ReviewTarget::Diff("dev".to_string()),
                logs: false,
                no_submodules: true,
            }
        );
    }

    #[test]
    fn parse_bare_short_sha_as_commit_review() {
        assert_eq!(
            parse(&["4542abe"]),
            CliCommand::Review {
                target: ReviewTarget::Commit("4542abe".to_string()),
                logs: false,
                no_submodules: false,
            }
        );
    }

    #[test]
    fn parse_diff_short_sha_as_range_diff_review() {
        assert_eq!(
            parse(&["diff", "4542abe"]),
            CliCommand::Review {
                target: ReviewTarget::Diff("4542abe".to_string()),
                logs: false,
                no_submodules: false,
            }
        );
    }

    #[test]
    fn parse_short_version_option() {
        assert_eq!(parse(&["-v"]), CliCommand::Version);
    }

    #[test]
    fn parse_long_version_option() {
        assert_eq!(parse(&["--version"]), CliCommand::Version);
    }

    #[test]
    fn parse_no_submodules_rejects_serve_command() {
        let error = parse_cli_args(vec!["serve".to_string(), "-ns".to_string()])
            .expect_err("expected no-submodules to be rejected for serve");

        assert!(
            error
                .to_string()
                .contains("--no-submodules only applies when opening a review")
        );
    }
}
