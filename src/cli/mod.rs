//! The three executables' command line: which frame a window opens on, and how it gets there.

mod args;
#[cfg(test)]
mod tests;

use std::{env, path::Path};

use anyhow::{Context, Result};

use crate::{
    api::{DiffTarget, OpenSessionRequest},
    git::{canonicalize_repo, find_repo_root},
    server,
};
use args::{
    CliCommand, ReviewSource, ReviewTarget, current_dir_pathspec, parse_cli_args,
    review_open_request,
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
    /// What the screen between that button and the open window says it is doing.
    opening: &'static str,
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
        opening: "opening the review…",
    },
    FrameProgram {
        frame: Frame::Tasks,
        program: "moontasks",
        display_name: "Moontasks",
        opens: "the task board",
        asks_for_repo: "Which repo to open the board of:",
        asks_for_remote_repo: "Path of the repo to open the board of, on that machine:",
        opens_button: "Open board",
        opening: "opening the board…",
    },
    FrameProgram {
        frame: Frame::Shell,
        program: "moonshell",
        display_name: "Moonshell",
        opens: "a shell in the repo",
        asks_for_repo: "Which repo to open a shell in:",
        asks_for_remote_repo: "Path of the repo to open a shell in, on that machine:",
        opens_button: "Open shell",
        opening: "opening the shell…",
    },
];

impl Frame {
    /// The name of the executable that opens on this frame.
    pub(crate) fn program(self) -> &'static str {
        self.entry().program
    }

    /// The name a desktop launcher shows: the one the OS puts under the icon.
    pub(crate) fn display_name(self) -> &'static str {
        self.entry().display_name
    }

    /// What the window opens on, as one line of prose.
    pub(crate) fn opens(self) -> &'static str {
        self.entry().opens
    }

    /// How the launch screen asks which repo to open, which depends on whether this machine
    /// can browse for it.
    pub(crate) fn asks_for_repo(self, picks_folders: bool) -> &'static str {
        let entry = self.entry();
        if picks_folders {
            entry.asks_for_repo
        } else {
            entry.asks_for_remote_repo
        }
    }

    /// What the launch screen's button says.
    pub(crate) fn opens_button(self) -> &'static str {
        self.entry().opens_button
    }

    /// What the screen between that button and the open window says it is doing.
    pub(crate) fn opening(self) -> &'static str {
        self.entry().opening
    }

    fn entry(self) -> &'static FrameProgram {
        FRAME_PROGRAMS
            .iter()
            .find(|entry| entry.frame == self)
            .expect("every frame has an executable")
    }
}

pub(crate) fn run(frame: Frame) -> Result<()> {
    // Before anything starts a thread or a child: a window launched from the Dock has no
    // locale, and every tool it runs would read and write bytes outside ASCII as something
    // other than UTF-8 - see `crate::shell_locale`.
    crate::shell_locale::adopt_utf8_locale();

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
        CliCommand::Review { target, source } => launch_review(target, source, frame),
    }
}

/// `install-launchers` from a terminal: the same writing the window's menu item does, with
/// what landed where printed rather than shown as a toast.
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

/// The window opened on nothing, asking which repo to open.
fn pick_project(frame: Frame) -> Result<()> {
    crate::native::run(crate::native::launch_prompt(frame)?)
}

/// The window on a named repo rather than on the one the shell it was started from is in.
///
/// It opens on the whole working tree: a path names the repo here, not a part of it to
/// narrow the review to.
fn open_repo(path: &str, frame: Frame) -> Result<()> {
    let repo_path = canonicalize_repo(Path::new(path))?;
    let launch = crate::native::launch_local(
        OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(DiffTarget::default()),
            active_commit: None,
        },
        frame,
    )?;
    crate::native::run(launch)
}

fn launch_review(target: ReviewTarget, source: ReviewSource, frame: Frame) -> Result<()> {
    if let ReviewSource::Remote { target, repo_path } = &source {
        // The repo lives on the far side, so nothing here is resolved against this machine.
        let launch = crate::native::launch_remote(target, repo_path.clone(), frame)?;
        return crate::native::run(launch);
    }

    let current_dir = env::current_dir()?;

    if target == ReviewTarget::WorkingTree && find_repo_root(&current_dir)?.is_none() {
        // A launcher opened from the OS starts outside any repo - there is no terminal it could
        // have inherited one from - so the window asks which repo to open.
        let launch = crate::native::launch_prompt(frame)?;
        return crate::native::run(launch);
    }

    let repo_path = canonicalize_repo(&current_dir)?;
    let current_dir_pathspec = current_dir_pathspec(&repo_path, &current_dir)?;
    let open_request = review_open_request(&repo_path, target, current_dir_pathspec, &current_dir)?;

    // The window is the app: it carries the review server with it, so another window can be
    // pointed at the same repo through `--remote`.
    let launch = crate::native::launch_local(
        OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(open_request.diff_target),
            active_commit: open_request.active_commit,
        },
        frame,
    )?;
    crate::native::run(launch)
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

Reviewing another machine's repo:
  The window carries the review server inside it, so a window elsewhere can be pointed at
  this repo.
  `--remote <host>` opens the window against a `serve` on another machine, where the repo
  lives; `--repo <path>` then names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.

Moontasks:
  The moontasks board is a sprint board over the `.moontasks` folder of the repo, with an
  agent running behind each card. `moontasks` opens on it; the other two reach it from the
  command palette.
  The columns are the board's own - rename them, reorder them, add and remove them - and a
  finished agent is reflected on its card the next time the board reads the folder.

Use `--logs` with `serve` to run the server in the foreground and print agent/failure logs
until you stop it with Ctrl+C.
Changed submodules are offered inside the review, as extra reviews you can open from the
command palette.",
        siblings = siblings.join("\n")
    )
}

fn print_version(frame: Frame) {
    println!("{} {}", frame.program(), env!("CARGO_PKG_VERSION"));
}
