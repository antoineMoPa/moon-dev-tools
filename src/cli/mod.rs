//! The three executables' command line: which frame a window opens on, and how it gets there.

mod args;
#[cfg(test)]
mod tests;

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

use args::{
    CliCommand, Frontend, ReviewOpenRequest, ReviewTarget, current_dir_pathspec, parse_cli_args,
    review_open_request,
};
use crate::{
    api::{DiffTarget, OpenSessionRequest, SessionOpened, client_host, port, server_url},
    git::{canonicalize_repo, find_repo_root},
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

/// Every frame, in the order they are named in help and given launchers.
pub(crate) const FRAMES: &[Frame] = &[Frame::Review, Frame::Tasks, Frame::Shell];

/// The same three in the order a window offers to open another one, which is not the order
/// they are written about in: the board comes first, because a new window is usually a new
/// piece of work rather than a second look at this one.
pub(crate) const NEW_WINDOW_FRAMES: &[Frame] = &[Frame::Tasks, Frame::Review, Frame::Shell];

/// Everything that differs between the three executables in name and wording, kept in one
/// place so a new frame is a row here rather than a branch wherever text is written.
struct FrameProgram {
    frame: Frame,
    /// The name of the executable that opens on this frame.
    program: &'static str,
    /// The name a desktop launcher shows: the one the OS puts under the icon.
    display_name: &'static str,
    /// What the window opens on, as one line of prose, for the CLI's help.
    opens: &'static str,
    /// How the launch screen asks which repo to open.
    asks_for_repo: &'static str,
    /// The same, when the repo is on the far side of a remote connection and can only be
    /// typed out.
    asks_for_remote_repo: &'static str,
    /// What the launch screen's button says.
    opens_button: &'static str,
}

const FRAME_PROGRAMS: &[FrameProgram] = &[
    FrameProgram {
        frame: Frame::Review,
        program: "moonreview",
        display_name: "Moonreview",
        opens: "a review of the repo",
        asks_for_repo: "Which repo to review:",
        asks_for_remote_repo: "Path of the repo to review, on that machine:",
        opens_button: "Open review",
    },
    FrameProgram {
        frame: Frame::Tasks,
        program: "moontasks",
        display_name: "Moontasks",
        opens: "the task board",
        asks_for_repo: "Which repo to open the board of:",
        asks_for_remote_repo: "Path of the repo to open the board of, on that machine:",
        opens_button: "Open board",
    },
    FrameProgram {
        frame: Frame::Shell,
        program: "moonshell",
        display_name: "Moonshell",
        opens: "a shell in the repo",
        asks_for_repo: "Which repo to open a shell in:",
        asks_for_remote_repo: "Path of the repo to open a shell in, on that machine:",
        opens_button: "Open shell",
    },
];

impl Frame {
    /// The name of the executable that opens on this frame.
    pub(crate) fn program(self) -> &'static str {
        self.entry().program
    }

    /// The name a desktop launcher shows: the one the OS puts under the icon.
    #[cfg(feature = "native")]
    pub(crate) fn display_name(self) -> &'static str {
        self.entry().display_name
    }

    /// What the window opens on, as one line of prose.
    pub(crate) fn opens(self) -> &'static str {
        self.entry().opens
    }

    /// How the launch screen asks which repo to open, which depends on whether this machine
    /// can browse for it.
    #[cfg(feature = "native")]
    pub(crate) fn asks_for_repo(self, picks_folders: bool) -> &'static str {
        let entry = self.entry();
        if picks_folders {
            entry.asks_for_repo
        } else {
            entry.asks_for_remote_repo
        }
    }

    /// What the launch screen's button says.
    #[cfg(feature = "native")]
    pub(crate) fn opens_button(self) -> &'static str {
        self.entry().opens_button
    }

    fn entry(self) -> &'static FrameProgram {
        FRAME_PROGRAMS
            .iter()
            .find(|entry| entry.frame == self)
            .expect("every frame has an executable")
    }
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
        CliCommand::InstallLaunchers => install_launchers(),
        CliCommand::PickProject => pick_project(frame),
        CliCommand::OpenRepo(path) => open_repo(&path, frame),
        CliCommand::Review {
            target,
            logs,
            frontend,
        } => launch_review(target, logs, frontend, frame),
    }
}

/// `install-launchers` from a terminal: the same writing the window's menu item does, with
/// what landed where printed rather than shown as a toast.
#[cfg(feature = "native")]
fn install_launchers() -> Result<()> {
    use crate::native::launchers;

    for launcher in launchers::install()? {
        println!(
            "{} → {}",
            launcher.frame.display_name(),
            launcher.path.display()
        );
    }
    println!(
        "The OS lists them from {}; rerun this after moving the executables.",
        launchers::destination_hint()
    );
    Ok(())
}

#[cfg(not(feature = "native"))]
fn install_launchers() -> Result<()> {
    bail!("this build has no desktop frontend, so a launcher would have no window to open")
}

/// The window opened on nothing, asking which repo to open.
#[cfg(feature = "native")]
fn pick_project(frame: Frame) -> Result<()> {
    crate::native::run(crate::native::launch_prompt(frame)?)
}

#[cfg(not(feature = "native"))]
fn pick_project(_frame: Frame) -> Result<()> {
    bail!("this build has no desktop frontend, so there is no window to pick a repo in")
}

/// The window on a named repo rather than on the one the shell it was started from is in.
///
/// It opens on the whole working tree: a path names the repo here, not a part of it to
/// narrow the review to.
#[cfg(feature = "native")]
fn open_repo(path: &str, frame: Frame) -> Result<()> {
    let repo_path = canonicalize_repo(Path::new(path))?;
    let launch = crate::native::launch_local(
        OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(DiffTarget::default()),
            active_commit: None,
        },
        true,
        frame,
    )?;
    crate::native::run(launch)
}

#[cfg(not(feature = "native"))]
fn open_repo(_path: &str, _frame: Frame) -> Result<()> {
    bail!("this build has no desktop frontend, so there is no window to open a repo in")
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

    #[cfg(feature = "native")]
    if frontend == Frontend::Native
        && target == ReviewTarget::WorkingTree
        && find_repo_root(&current_dir)?.is_none()
    {
        // A launcher opened from the OS starts outside any repo - there is no terminal it could
        // have inherited one from - so the window asks which repo to open.
        let launch = crate::native::launch_prompt(frame)?;
        return crate::native::run(launch);
    }

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

fn print_help(frame: Frame) {
    println!("{}", help_text_for(frame));
}

/// The help of whichever executable was run: the same review options either way, with the
/// frame it opens on at the top and the other two named at the bottom.
pub(super) fn help_text_for(frame: Frame) -> String {
    let program = frame.program();
    let opens = frame.opens();
    let siblings: Vec<String> = FRAMES
        .iter()
        .filter(|candidate| **candidate != frame)
        .map(|sibling| {
            format!(
                "  {name} - opens on {opens}",
                name = sibling.program(),
                opens = sibling.opens()
            )
        })
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
  {program} --pick
  {program} --repo <path>
  {program} --remote <host> [--repo <path>]
  {program} serve --logs
  {program} install-launchers
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
`--pick` opens the window on its launch screen instead, which is where recent projects and
the folder picker are; it is what the Window menu's New Window items open.
`--repo <path>` opens the window on that repo rather than on the one this shell is in; it is
what the Window menu's Restart hands the instance it starts.
Run `{program} .` to limit the review to the current directory.
Pass one path to review only that file or directory's working-tree changes.
Pass two paths to review a read-only comparison of those files.

`{program} <commit>` opens a read-only review of a single commit.
`{program} diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`.

The other frames, which are the same window opened on something else:
{siblings}

Desktop launchers:
  `install-launchers` gives each installed executable an entry the OS offers - an application
  bundle on macOS, a desktop entry on Linux - so they open from Spotlight, Launchpad or an
  application menu as well as from a shell. The window has the same thing in its menu.
  A window opened that way starts outside any repo, so it asks which repo to open.

Frontends:
  By default the window carries the review server inside it, so the same review can be
  opened in a browser.
  `--web` opens a browser tab against a background server instead.
  `--remote <host>` opens the window against a `serve` on another machine, where the repo
  lives; `--repo <path>` then names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.

Moontasks:
  The moontasks board is a sprint board over the `.moontasks` folder of the repo, with an
  agent running behind each card. `moontasks` opens on it; the other two reach it from the
  command palette.
  The columns are the board's own - rename them, reorder them, add and remove them - and a
  finished agent is reflected on its card the next time the board reads the folder.

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
