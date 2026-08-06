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

/// What the window opens on, which is the whole difference between the three executables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// `moonreview`: the review of the repo.
    Review,
    /// `moontasks`: the task board, and the agents working through it.
    Tasks,
    /// `moonshell`: a shell in the repo.
    Shell,
}

/// What each executable is called, and the one line of help that says what it opens.
const FRAME_PROGRAMS: &[(Frame, &str, &str)] = &[
    (Frame::Review, "moonreview", "a review of the repo"),
    (Frame::Tasks, "moontasks", "the task board"),
    (Frame::Shell, "moonshell", "a shell in the repo"),
];

impl Frame {
    /// The name of the executable that opens on this frame.
    pub(crate) fn program(self) -> &'static str {
        FRAME_PROGRAMS
            .iter()
            .find(|(frame, _, _)| *frame == self)
            .map(|(_, program, _)| *program)
            .expect("every frame has an executable")
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CliCommand {
    Help,
    Version,
    Serve {
        logs: bool,
    },
    /// The moontasks MCP server, on stdio. Agents start this, not people.
    Mcp,
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

pub(crate) fn run(frame: Frame) -> Result<()> {
    match parse_cli_args(env::args().skip(1).collect::<Vec<_>>(), frame)? {
        CliCommand::Help => {
            print_help(frame);
            Ok(())
        }
        CliCommand::Version => {
            print_version(frame);
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
        CliCommand::Mcp => crate::moontasks::mcp::run(),
        CliCommand::Review {
            target,
            logs,
            frontend,
        } => launch_review(target, logs, frontend, frame),
    }
}

fn launch_review(
    target: ReviewTarget,
    logs: bool,
    frontend: Frontend,
    frame: Frame,
) -> Result<()> {
    #[cfg(feature = "native")]
    if let Frontend::Remote { target, repo_path } = &frontend {
        // The repo lives on the far side, so nothing here is resolved against this machine.
        let launch = crate::native::launch_remote(target, repo_path.clone(), frame)?;
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
            frame,
        )?;
        return crate::native::run(launch);
    }
    let _ = (&frontend, frame);

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
        ReviewTarget::Path(path) => {
            // `main..feature` is git's range syntax rather than a file. A file whose name
            // happens to contain two dots is still a file, so what is on disk wins — the same
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

/// Whether an argument reads as one of git's revision ranges: `main..feature`, or the
/// symmetric `main...feature`. Both sides have to name something for it to be a range, which
/// is what keeps `..`, `../` and a file called `a..b` out of it.
fn is_revision_range(value: &str) -> bool {
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

fn parse_cli_args(args: Vec<String>, frame: Frame) -> Result<CliCommand> {
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
            _ if arg.starts_with('-') => bail!("unknown option: {arg}\n\n{}", help_text_for(frame)),
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
        [command] if command == "mcp" => Ok(CliCommand::Mcp),
        [command] if command == "diff" => Ok(review(ReviewTarget::WorkingTree)),
        [command, target] if command == "diff" => Ok(review(ReviewTarget::Diff(target.clone()))),
        [target] if target == "." || target == "./" => Ok(review(ReviewTarget::CurrentDirectory)),
        [target] => Ok(review(if is_sha_like(target) {
            ReviewTarget::Commit(target.clone())
        } else {
            ReviewTarget::Path(target.clone())
        })),
        [command, ..] if command == "diff" || command == "serve" || command == "mcp" => {
            bail!("{}", help_text_for(frame))
        }
        [before, after] => Ok(review(ReviewTarget::Comparison([
            before.clone(),
            after.clone(),
        ]))),
        _ => bail!("{}", help_text_for(frame)),
    }
}

fn print_help(frame: Frame) {
    println!("{}", help_text_for(frame));
}

/// The help of whichever executable was run: the same review options either way, with the
/// frame it opens on at the top and the other two named at the bottom.
fn help_text_for(frame: Frame) -> String {
    let program = frame.program();
    let opens = FRAME_PROGRAMS
        .iter()
        .find(|(candidate, _, _)| *candidate == frame)
        .map(|(_, _, opens)| *opens)
        .expect("every frame says what it opens");
    let siblings: Vec<String> = FRAME_PROGRAMS
        .iter()
        .filter(|(candidate, _, _)| *candidate != frame)
        .map(|(_, name, opens)| format!("  {name} — opens on {opens}"))
        .collect();

    format!(
        "{program}

Tiny local code review UI for git. This one opens on {opens}.

Usage:
  {program}
  {program} .
  {program} <path>
  {program} <before-path> <after-path>
  {program} <commit>
  {program} diff <target>
  {program} --web
  {program} --remote <host> [--repo <path>]
  {program} serve --logs
  {program} mcp
  {program} --version
  {program} --help

Examples:
  {program}
  {program} .
  {program} src/main.rs
  {program} before.json after.json
  {program} 4542abe
  {program} diff dev
  {program} --web
  {program} --remote dev-box --repo /home/you/project

Run `{program}` inside any git repository you want to work in.
Run `{program} .` to limit the review to the current directory.
Pass one path to review only that file or directory's working-tree changes.
Pass two paths to review a read-only comparison of those files.

`{program} <commit>` opens a read-only review of a single commit.
`{program} diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`.

The other frames, which are the same window opened on something else:
{siblings}

Frontends:
  By default the window carries the review server inside it, so the same review can be
  opened in a browser.
  `--web` opens a browser tab against a background server instead.
  `--remote <host>` opens the window against a `serve` on another machine, where the repo
  lives; `--repo <path>` names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.

Moontasks:
  The moontasks board is a sprint board over the `.moontasks` folder of the repo, with an
  agent running behind each card. `moontasks` opens on it; the other two reach it from the
  command palette.
  `mcp` is the MCP server those agents use to move their own card; moonreview starts it for
  them, so there is no reason to run it by hand.

Use `--logs` with `--web` or `serve` to run the server in the foreground and print
agent/failure logs until you stop it with Ctrl+C.
Changed submodules are offered inside the review, as extra reviews you can open from the
command palette.",
        siblings = siblings.join("\n")
    )
}

fn print_version(frame: Frame) {
    println!("{} {}", frame.program(), env!("MOONREVIEW_VERSION"));
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
        parse_cli_args(
            args.iter().map(|arg| arg.to_string()).collect(),
            Frame::Review,
        )
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
    fn a_revision_range_is_told_apart_from_a_path() {
        assert!(is_revision_range("main..egui-version"));
        assert!(is_revision_range("main...egui-version"));
        assert!(is_revision_range("release/1.0..main"));
        assert!(!is_revision_range("src/main.rs"));
        assert!(!is_revision_range(".."));
        assert!(!is_revision_range("../sibling/file.rs"));
        assert!(!is_revision_range("main.."));
    }

    #[test]
    fn a_range_of_branches_is_reviewed_as_a_diff_against_its_base() {
        let request = review_open_request(
            Path::new("/repo"),
            ReviewTarget::Path("main..egui-version".to_string()),
            None,
            Path::new("/repo"),
        )
        .expect("expected review request");

        assert_eq!(request.diff_target.base.as_deref(), Some("main..egui-version"));
        assert_eq!(request.diff_target.pathspec, None);
        assert_eq!(request.active_commit, None);
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
        let error = parse_cli_args(vec!["-ns".to_string()], Frame::Review)
            .expect_err("expected an unknown option to be rejected");

        assert!(error.to_string().contains("unknown option: -ns"));
    }
}
