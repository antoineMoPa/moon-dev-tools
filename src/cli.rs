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
    git::{canonicalize_repo, list_changed_submodule_repos, parse_review_target},
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
        target: Option<String>,
        logs: bool,
        no_submodules: bool,
    },
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

fn launch_review(raw_target: Option<String>, logs: bool, no_submodules: bool) -> Result<()> {
    let diff_target = parse_review_target(raw_target)?;
    let repo_path = canonicalize_repo(env::current_dir()?)?;
    if logs {
        return launch_review_with_foreground_server(repo_path, diff_target, no_submodules);
    }

    ensure_server_running(logs)?;
    open_review_session(&repo_path, diff_target, no_submodules)?;
    Ok(())
}

fn launch_review_with_foreground_server(
    repo_path: PathBuf,
    diff_target: DiffTarget,
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
            open_review_session(&repo_path, diff_target, no_submodules)?;
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
    diff_target: DiffTarget,
    no_submodules: bool,
) -> Result<()> {
    let extra_repo_paths = if diff_target.base.is_none() && !no_submodules {
        list_changed_submodule_repos(repo_path)?
    } else {
        Vec::new()
    };

    let mut opened_urls = Vec::new();
    opened_urls.push(open_review_url_for_session(repo_path, &diff_target)?);
    for submodule_path in extra_repo_paths {
        opened_urls.push(open_review_url_for_session(&submodule_path, &diff_target)?);
    }

    for url in &opened_urls {
        webbrowser::open(url).context("failed to open browser")?;
        println!("Opened {url}");
    }

    Ok(())
}

fn open_review_url_for_session(repo_path: &Path, diff_target: &DiffTarget) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("failed to create client")?;

    let opened: SessionOpened = client
        .post(format!("{}/api/session/open", server_url()))
        .json(&OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(diff_target.clone()),
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
            target: None,
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
            target: None,
            logs,
            no_submodules,
        }),
        [command, target] if command == "diff" => Ok(CliCommand::Review {
            target: Some(target.clone()),
            logs,
            no_submodules,
        }),
        [target] => Ok(CliCommand::Review {
            target: Some(target.clone()),
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
  moonreview diff dev
  moonreview diff dev:./

Run `moonreview` inside any git repository you want to review.

Use `--logs` to run the server in the foreground and print agent/failure logs until you stop it with Ctrl+C.
Use `-ns` or `--no-submodules` to open only the current repository when changed submodules are present.

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
                target: None,
                logs: false,
                no_submodules: true,
            }
        );
    }

    #[test]
    fn parse_no_submodules_long_option_for_diff_review() {
        assert_eq!(
            parse(&["diff", "dev", "--no-submodules"]),
            CliCommand::Review {
                target: Some("dev".to_string()),
                logs: false,
                no_submodules: true,
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
