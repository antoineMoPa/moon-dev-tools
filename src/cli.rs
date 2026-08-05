use std::{
    env,
    net::TcpStream,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;

use crate::{
    api::{DiffTarget, OpenSessionRequest, SessionOpened, client_host, port, server_url},
    git::{canonicalize_repo, parse_review_target, run_git},
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
        frontend: Frontend,
    },
}

/// Which frontend a review opens in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Frontend {
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
enum ReviewTarget {
    WorkingTree,
    CurrentDirectory,
    Path(String),
    Comparison([String; 2]),
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
            frontend,
        } => launch_review(target, logs, frontend),
    }
}

fn launch_review(target: ReviewTarget, logs: bool, frontend: Frontend) -> Result<()> {
    #[cfg(feature = "native")]
    if let Frontend::Remote { target, repo_path } = &frontend {
        // The repo lives on the far side, so nothing here is resolved against this machine.
        let launch = crate::native::launch_remote(target, repo_path.clone())?;
        return crate::native::run(launch);
    }

    let current_dir = env::current_dir()?;
    let repo_path = canonicalize_repo(&current_dir)?;
    let current_dir_pathspec = current_dir_pathspec(&repo_path, &current_dir)?;
    let open_request = review_open_request(&repo_path, target, current_dir_pathspec, &current_dir)?;

    #[cfg(feature = "native")]
    if frontend == Frontend::Native {
        // The window is the app: it carries the review server with it, so a browser can be
        // pointed at the same review without a second process.
        let launch = crate::native::launch_local(
            OpenSessionRequest {
                repo_path: repo_path.display().to_string(),
                diff_target: Some(open_request.diff_target.clone()),
                active_commit: open_request.active_commit.clone(),
            },
            true,
        )?;
        return crate::native::run(launch);
    }
    let _ = &frontend;

    if logs {
        return launch_review_with_foreground_server(repo_path, open_request);
    }

    ensure_server_running(logs)?;
    open_review_session(&repo_path, &open_request)?;
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
        ReviewTarget::Path(path) => Ok(ReviewOpenRequest {
            diff_target: DiffTarget {
                base: None,
                pathspec: Some(repo_relative_pathspec(repo_path, current_dir, &path)?),
                comparison: None,
            },
            active_commit: None,
        }),
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

fn launch_review_with_foreground_server(
    repo_path: PathBuf,
    open_request: ReviewOpenRequest,
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
            open_review_session(&repo_path, &open_request)?;
            return server_thread
                .join()
                .map_err(|_| anyhow!("review server thread panicked"))?;
        }
        thread::sleep(Duration::from_millis(150));
    }

    bail!("review server did not become ready on {}", server_url())
}

/// One browser tab per run. Changed submodules are offered inside it, as review windows
/// the user opens from the workspace.
fn open_review_session(repo_path: &Path, open_request: &ReviewOpenRequest) -> Result<()> {
    let url = open_review_url_for_session(repo_path, open_request)?;
    webbrowser::open(&url).context("failed to open browser")?;
    println!("Opened {url}");
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
    let mut web = false;
    let mut remote: Option<String> = None;
    let mut remote_repo: Option<String> = None;
    let mut positional = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--logs" => logs = true,
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
                remote_repo = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--repo needs a path on the remote machine"))?,
                );
            }
            _ if arg.starts_with("--remote=") => {
                remote = Some(arg["--remote=".len()..].to_string());
            }
            _ if arg.starts_with("--repo=") => {
                remote_repo = Some(arg["--repo=".len()..].to_string());
            }
            _ if arg.starts_with('-') => bail!("unknown option: {arg}\n\n{}", help_text()),
            _ => positional.push(arg),
        }
    }

    if web && remote.is_some() {
        bail!("--web and --remote are different frontends; pick one");
    }
    if remote_repo.is_some() && remote.is_none() {
        bail!("--repo names a path on a remote machine, so it needs --remote too");
    }

    let frontend = match (web, remote) {
        (true, _) => Frontend::Web,
        (false, Some(target)) => Frontend::Remote {
            target,
            repo_path: remote_repo,
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
        [command] if command == "diff" => Ok(review(ReviewTarget::WorkingTree)),
        [command, target] if command == "diff" => Ok(review(ReviewTarget::Diff(target.clone()))),
        [target] if target == "." || target == "./" => Ok(review(ReviewTarget::CurrentDirectory)),
        [target] => Ok(review(if is_sha_like(target) {
            ReviewTarget::Commit(target.clone())
        } else {
            ReviewTarget::Path(target.clone())
        })),
        [command, ..] if command == "diff" || command == "serve" => bail!("{}", help_text()),
        [before, after] => Ok(review(ReviewTarget::Comparison([
            before.clone(),
            after.clone(),
        ]))),
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
  moonreview <path>
  moonreview <before-path> <after-path>
  moonreview <commit>
  moonreview diff <target>
  moonreview --web
  moonreview --remote <host> [--repo <path>]
  moonreview serve --logs
  moonreview --version
  moonreview --help

Examples:
  moonreview
  moonreview .
  moonreview src/main.rs
  moonreview before.json after.json
  moonreview 4542abe
  moonreview diff dev
  moonreview diff dev:./
  moonreview --web
  moonreview --remote dev-box --repo /home/you/project

Run `moonreview` inside any git repository you want to review.
Run `moonreview .` to review only the current directory.
Pass one path to review only that file or directory's working-tree changes.
Pass two paths to review a read-only comparison of those files.

`moonreview <commit>` opens a read-only review of a single commit.
`moonreview diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`.

Frontends:
  By default the review opens in a native window, with the review server inside it. That
  window also serves the web frontend, so the same review can be opened in a browser.
  `--web` opens a browser tab against a background server instead.
  `--remote <host>` opens the window against a `moonreview serve` on another machine, where
  the repo lives; `--repo <path>` names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.

Use `--logs` with `--web` or `serve` to run the server in the foreground and print
agent/failure logs until you stop it with Ctrl+C.
Changed submodules are offered inside the review, as extra reviews you can open from the
command palette."
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
    fn parse_bare_review_of_the_working_tree() {
        assert_eq!(
            parse(&[]),
            CliCommand::Review {
                target: ReviewTarget::WorkingTree,
                logs: false,
                frontend: Frontend::Native,
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
                frontend: Frontend::Native,
            }
        );
    }

    #[test]
    fn parse_single_path_as_working_tree_pathspec() {
        assert_eq!(
            parse(&["packages/app/src/example.ts"]),
            CliCommand::Review {
                target: ReviewTarget::Path("packages/app/src/example.ts".to_string()),
                logs: false,
                frontend: Frontend::Native,
            }
        );
    }

    #[test]
    fn working_tree_review_ignores_current_directory_pathspec() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::WorkingTree,
            Some("src".to_string()),
            Path::new("/repo/src"),
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
            Path::new("/repo/src"),
        )
        .expect("expected review request");

        assert_eq!(request.diff_target.base, None);
        assert_eq!(request.diff_target.pathspec.as_deref(), Some("src"));
        assert_eq!(request.active_commit, None);
    }

    #[test]
    fn single_path_review_uses_repo_relative_pathspec() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::Path("src/example.ts".to_string()),
            Some("packages/app".to_string()),
            Path::new("/repo/packages/app"),
        )
        .expect("expected review request");

        assert_eq!(request.diff_target.base, None);
        assert_eq!(
            request.diff_target.pathspec.as_deref(),
            Some("packages/app/src/example.ts")
        );
        assert_eq!(request.active_commit, None);
    }

    #[test]
    fn parse_two_paths_as_file_comparison() {
        assert_eq!(
            parse(&["a.txt", "b.txt"]),
            CliCommand::Review {
                target: ReviewTarget::Comparison(["a.txt".to_string(), "b.txt".to_string()]),
                logs: false,
                frontend: Frontend::Native,
            }
        );
    }

    #[test]
    fn comparison_paths_are_relative_to_current_directory() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::Comparison(["a.txt".to_string(), "nested/b.txt".to_string()]),
            Some("src".to_string()),
            Path::new("/repo/src"),
        )
        .expect("expected review request");

        assert_eq!(
            request.diff_target.comparison,
            Some([
                "/repo/src/a.txt".to_string(),
                "/repo/src/nested/b.txt".to_string()
            ])
        );
        assert_eq!(request.active_commit, None);
    }

    #[test]
    fn parse_diff_review_against_a_named_ref() {
        assert_eq!(
            parse(&["diff", "dev"]),
            CliCommand::Review {
                target: ReviewTarget::Diff("dev".to_string()),
                logs: false,
                frontend: Frontend::Native,
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
                frontend: Frontend::Native,
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
                frontend: Frontend::Native,
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
    fn parse_rejects_unknown_options() {
        let error = parse_cli_args(vec!["-ns".to_string()])
            .expect_err("expected an unknown option to be rejected");

        assert!(error.to_string().contains("unknown option: -ns"));
    }
}
