//! The command line: which command was asked for, and which frame a window opens on.

mod args;
mod open;
#[cfg(test)]
mod tests;

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    api::{DiffTarget, OpenSessionRequest},
    git::{find_repo_root, project_root},
    server,
};
use args::{
    CliCommand, ReviewSource, ReviewTarget, current_dir_pathspec, parse_cli_args,
    review_open_request,
};

/// What the window opens on, which is the whole difference between the three windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// `moon review`: the review of the repo.
    Review,
    /// `moon tasks`: the task board, and the agents working through it.
    Tasks,
    /// `moon shell`: a shell in the folder, which need not be a repo.
    Shell,
}

/// Every frame, in the order they are named in help and given launchers.
pub(crate) const FRAMES: &[Frame] = &[Frame::Review, Frame::Tasks, Frame::Shell];

/// The same three in the order a window offers to open another one, which is not the order
/// they are written about in: the board comes first, because a new window is usually a new
/// piece of work rather than a second look at this one.
pub(crate) const NEW_WINDOW_FRAMES: &[Frame] = &[Frame::Tasks, Frame::Review, Frame::Shell];

/// The one executable all of this is: the three windows, the server behind them, and the
/// commands that reach a window which is already open.
pub(crate) const PROGRAM: &str = "moon";

/// How a launcher says which window it is.
///
/// A desktop entry runs `moon <subcommand>` and needs none of this, but a macOS bundle runs
/// its executable with no arguments at all - and with the executable a link to the installed
/// `moon`, which the OS resolves before starting it, so not even the name it was started
/// under says which window was asked for. What a bundle *can* carry is `LSEnvironment`, so
/// that is where its window is written; see [`crate::native::launchers`].
pub(crate) const FRAME_ENV: &str = "MOON_FRAME";

/// Everything that differs between the three frames in name and wording, kept in one place so
/// a new frame is a row here rather than a branch wherever text is written.
struct FrameProgram {
    frame: Frame,
    /// The word after `moon` that opens a window on this frame.
    subcommand: &'static str,
    /// The name this frame goes by wherever a name has to be one token: its icon and
    /// launcher files, and its bundle identifier.
    slug: &'static str,
    /// The name a desktop launcher shows: the one the OS puts under the icon.
    display_name: &'static str,
    /// What the window opens on, as one line of prose, for the CLI's help.
    opens: &'static str,
    /// How the launch screen asks which repo to open.
    asks_for_repo: &'static str,
    /// The same, when the repo is on the far side of a remote connection and can only be
    /// typed out.
    asks_for_remote_repo: &'static str,
    /// What the launch screen's folder picker button says. A review needs a repo and asks
    /// for one; the other two are as much use in a folder git knows nothing about.
    picker_button: &'static str,
    /// What the launch screen's button says.
    opens_button: &'static str,
    /// What the screen between that button and the open window says it is doing.
    opening: &'static str,
    /// Whether this frame is any use in a folder git knows nothing about, which decides what
    /// a window does when the folder it was started in is in no repo: a shell and a task
    /// board just open on it, and a review - which is made entirely out of what git knows -
    /// asks for a repo instead.
    opens_without_a_repo: bool,
}

const FRAME_PROGRAMS: &[FrameProgram] = &[
    FrameProgram {
        frame: Frame::Review,
        subcommand: "review",
        slug: "moonreview",
        display_name: "Moonreview",
        opens: "a review of the repo",
        asks_for_repo: "Which repo to review:",
        asks_for_remote_repo: "Path of the repo to review, on that machine:",
        picker_button: "Choose a repo…",
        opens_button: "Open review",
        opening: "opening the review…",
        opens_without_a_repo: false,
    },
    FrameProgram {
        frame: Frame::Tasks,
        subcommand: "tasks",
        slug: "moontasks",
        display_name: "Moontasks",
        opens: "the task board",
        asks_for_repo: "Which folder to open the board of:",
        asks_for_remote_repo: "Path of the folder to open the board of, on that machine:",
        picker_button: "Choose a folder…",
        opens_button: "Open board",
        opening: "opening the board…",
        opens_without_a_repo: true,
    },
    FrameProgram {
        frame: Frame::Shell,
        subcommand: "shell",
        slug: "moonshell",
        display_name: "Moonshell",
        opens: "a shell in the folder",
        asks_for_repo: "Which folder to open a shell in:",
        asks_for_remote_repo: "Path of the folder to open a shell in, on that machine:",
        picker_button: "Choose a folder…",
        opens_button: "Open shell",
        opening: "opening the shell…",
        opens_without_a_repo: true,
    },
];

impl Frame {
    /// The word after `moon` that opens a window on this frame.
    pub(crate) fn subcommand(self) -> &'static str {
        self.entry().subcommand
    }

    /// What somebody types to open this window, which is what the window calls itself
    /// wherever it names itself to a person.
    pub(crate) fn command(self) -> String {
        format!("{PROGRAM} {}", self.subcommand())
    }

    /// The single token this frame's files and identifiers are named with - see the field.
    pub(crate) fn slug(self) -> &'static str {
        self.entry().slug
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

    /// What the launch screen's folder picker button says.
    pub(crate) fn picker_button(self) -> &'static str {
        self.entry().picker_button
    }

    /// What the launch screen's button says.
    pub(crate) fn opens_button(self) -> &'static str {
        self.entry().opens_button
    }

    /// What the screen between that button and the open window says it is doing.
    pub(crate) fn opening(self) -> &'static str {
        self.entry().opening
    }

    /// Whether this frame is any use in a folder git knows nothing about.
    fn opens_without_a_repo(self) -> bool {
        self.entry().opens_without_a_repo
    }

    fn entry(self) -> &'static FrameProgram {
        FRAME_PROGRAMS
            .iter()
            .find(|entry| entry.frame == self)
            .expect("every frame is in the table")
    }
}

/// What the command line asked for. One executable, so this is the whole of what it does.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum MoonCommand {
    Help,
    Version,
    /// A window on one of the frames. What it opens on is the rest of the command line,
    /// which is [`args`]' business rather than this one's.
    Window {
        frame: Frame,
        args: Vec<String>,
    },
    /// The review server, in this terminal, for a window on another machine to read.
    Serve {
        logs: bool,
    },
    /// Write the desktop launcher of each frame, so the OS offers them too.
    InstallLaunchers,
    /// One file, in the window that is already open on its project.
    Open {
        path: String,
        line: Option<usize>,
    },
    /// Which windows are open, and what they are open on.
    ListWindows,
}

pub(crate) fn run() -> Result<()> {
    // Before anything starts a thread or a child: a window launched from the Dock has no
    // locale, and every tool it runs would read and write bytes outside ASCII as something
    // other than UTF-8 - see `crate::shell_locale`.
    crate::shell_locale::adopt_utf8_locale();

    let launched_on = frame_of_launcher()?;

    match parse_command(launched_on, env::args().skip(1).collect())? {
        MoonCommand::Help => {
            println!("{}", help_text());
            Ok(())
        }
        MoonCommand::Version => {
            print_version();
            Ok(())
        }
        MoonCommand::Serve { logs } => {
            if logs {
                eprintln!("Moon Review server logs enabled.");
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to build tokio runtime")?;
            runtime.block_on(server::run_server())
        }
        MoonCommand::InstallLaunchers => install_launchers(),
        MoonCommand::Open { path, line } => open::open_file(&path, line),
        MoonCommand::ListWindows => open::list_windows(),
        MoonCommand::Window { frame, args } => open_window(frame, args),
    }
}

/// Which command a command line names.
///
/// `launched_on` is the window a macOS launcher asked for, which it says in the environment
/// rather than in the arguments - see [`FRAME_ENV`]. It comes first: a bundle passes no
/// arguments, so there is nothing else to read the window out of.
pub(super) fn parse_command(launched_on: Option<Frame>, args: Vec<String>) -> Result<MoonCommand> {
    if let Some(frame) = launched_on {
        return Ok(MoonCommand::Window { frame, args });
    }

    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(MoonCommand::Help);
    };
    let rest: Vec<String> = args.collect();

    if let Some(frame) = frame_named(&command) {
        return Ok(MoonCommand::Window { frame, args: rest });
    }

    match command.as_str() {
        "--help" | "-h" | "help" => Ok(MoonCommand::Help),
        "--version" | "-v" => Ok(MoonCommand::Version),
        // `edit` and `open` are one thing said two ways: the tab it lands in is one that
        // edits the file, and both are words a hand reaches for.
        "open" | "edit" => open::parse_open(rest),
        "list" => match rest.is_empty() {
            true => Ok(MoonCommand::ListWindows),
            false => bail!("`{PROGRAM} list` says which windows are open, so it takes nothing"),
        },
        "serve" => parse_serve(rest),
        "install-launchers" => match rest.is_empty() {
            true => Ok(MoonCommand::InstallLaunchers),
            false => bail!("`{PROGRAM} install-launchers` takes nothing else"),
        },
        other => bail!("`{PROGRAM} {other}` is not a command\n\n{}", help_text()),
    }
}

/// The frame a word names, for the word after `moon` and for what a launcher wrote in the
/// environment - they are the same word.
fn frame_named(name: &str) -> Option<Frame> {
    FRAME_PROGRAMS
        .iter()
        .find(|entry| entry.subcommand == name)
        .map(|entry| entry.frame)
}

/// The window a macOS launcher asked for, and `None` for every other way of starting.
///
/// The variable is taken out of the environment as it is read, so that the shells this
/// window starts do not inherit it: to them `moon` is the command line's `moon`, and a
/// `moon` typed with no arguments in one of them says what it can do rather than opening
/// another window.
fn frame_of_launcher() -> Result<Option<Frame>> {
    let Some(named) = env::var_os(FRAME_ENV) else {
        return Ok(None);
    };
    // SAFETY: nothing else has run yet - no thread has been started and no child spawned -
    // so there is no other reader of the environment to race with.
    unsafe { env::remove_var(FRAME_ENV) };

    let named = named
        .to_str()
        .with_context(|| format!("{FRAME_ENV} is not a window this program has"))?
        .to_string();
    frame_named(&named)
        .map(Some)
        .with_context(|| format!("{FRAME_ENV}={named} is not a window this program has"))
}

fn parse_serve(args: Vec<String>) -> Result<MoonCommand> {
    let mut logs = false;
    for arg in args {
        match arg.as_str() {
            "--logs" => logs = true,
            other => bail!("`{PROGRAM} serve` takes `--logs` and nothing else, not {other}"),
        }
    }
    Ok(MoonCommand::Serve { logs })
}

/// Open a window on a frame. What it opens on is the rest of the command line.
fn open_window(frame: Frame, args: Vec<String>) -> Result<()> {
    match parse_cli_args(args, frame)? {
        CliCommand::Help => {
            println!("{}", help_text_for(frame));
            Ok(())
        }
        CliCommand::Version => {
            print_version();
            Ok(())
        }
        CliCommand::PickProject => pick_project(frame),
        CliCommand::OpenRepo(path) => open_repo(Path::new(&path), frame),
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
        "The OS lists them from {}; rerun this after moving the executable.",
        launchers::destination_hint()
    );
    Ok(())
}

/// The window opened on nothing, asking which repo to open.
fn pick_project(frame: Frame) -> Result<()> {
    crate::native::run(crate::native::launch_prompt(frame)?)
}

/// The window on a named folder rather than on the one the shell it was started from is in.
///
/// It opens on the whole working tree: a path names the folder here, not a part of it to
/// narrow the review to.
fn open_repo(path: &Path, frame: Frame) -> Result<()> {
    let repo_path = project_root(path)?;
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

/// Where a window opens when nothing named a folder and the directory it was started in is
/// in no repo: that directory, when it is one somebody could be working in.
///
/// A window opened from a desktop launcher is started by the OS rather than by a shell, and
/// macOS starts it at the root of the filesystem. Nobody works there, so that one opens on
/// the last project this machine opened, and on the home folder when there has not been one.
fn folder_when_there_is_no_repo(current_dir: &Path) -> Result<PathBuf> {
    if current_dir != Path::new("/") {
        return Ok(current_dir.to_path_buf());
    }
    let last_project = crate::settings::load()
        .recent_projects
        .into_iter()
        .map(PathBuf::from)
        .find(|project| project.is_dir());
    match last_project {
        Some(project) => Ok(project),
        None => env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set, so there is no folder to open on"),
    }
}

fn launch_review(target: ReviewTarget, source: ReviewSource, frame: Frame) -> Result<()> {
    if let ReviewSource::Remote { target, repo_path } = &source {
        // The repo lives on the far side, so nothing here is resolved against this machine.
        let launch = crate::native::launch_remote(target, repo_path.clone(), frame)?;
        return crate::native::run(launch);
    }

    let current_dir = env::current_dir()?;

    if target == ReviewTarget::WorkingTree && find_repo_root(&current_dir)?.is_none() {
        // A shell and a board need no repo, so they never ask for one: they open on the
        // folder the window was started in.
        if frame.opens_without_a_repo() {
            return open_repo(&folder_when_there_is_no_repo(&current_dir)?, frame);
        }
        // A launcher opened from the OS starts outside any repo - there is no terminal it could
        // have inherited one from - so the window asks which repo to open.
        let launch = crate::native::launch_prompt(frame)?;
        return crate::native::run(launch);
    }

    let repo_path = project_root(&current_dir)?;
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

fn print_version() {
    println!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"));
}

/// `moon --help`: every command there is, with what each window opens on.
fn help_text() -> String {
    let windows: Vec<String> = FRAMES
        .iter()
        .map(|frame| {
            format!(
                "  {command:<30} {opens}",
                command = format!("{} [<target>]", frame.command()),
                opens = frame.opens()
            )
        })
        .collect();

    format!(
        "{PROGRAM}

Tiny local dev tools: a task board, a code review and a shell, one window each.

Usage:
{windows}
  {PROGRAM} open <path>[:<line>]      a file, in the window already open on its project
  {PROGRAM} list                      which windows are open, and what they are on
  {PROGRAM} serve [--logs]            the review server, for a window on another machine
  {PROGRAM} install-launchers         entries the OS offers for the three windows
  {PROGRAM} --version
  {PROGRAM} --help

Examples:
  {PROGRAM} tasks
  {PROGRAM} review src/main.rs
  {PROGRAM} shell
  {PROGRAM} open src/main.rs:42

Open a window inside any git repository you want to work in. The board and the shell run just
as well in a folder that is no repository: the review is the part that needs one.

`{PROGRAM} <window> --help` says what that window can be opened on; `--pick` opens it on its
launch screen instead, and `--remote <host>` opens it against a `serve` on another machine.

Desktop launchers:
  `install-launchers` gives each window an entry the OS offers - an application bundle on
  macOS, a desktop entry on Linux - so they open from Spotlight, Launchpad or an application
  menu as well as from a shell. The window has the same thing in its menu.
  A window opened that way starts outside any repo, so it opens on the project the last one
  did - and the review, which has nothing to show without a repo, asks which one instead.

Moontasks:
  The moontasks board is a sprint board over the `.moontasks` folder of the repo, with an
  agent running behind each card. `{PROGRAM} tasks` opens on it; the other two windows reach
  it from the command palette.
  The columns are the board's own - rename them, reorder them, add and remove them - and a
  finished agent is reflected on its card the next time the board reads the folder.",
        windows = windows.join("\n")
    )
}

/// `moon review --help` and its siblings: what that window can be opened on.
pub(super) fn help_text_for(frame: Frame) -> String {
    let command = frame.command();
    let opens = frame.opens();

    // The one line of help that is only true of the two frames that need no repo.
    let opens_without_a_repo = if frame.opens_without_a_repo() {
        "A folder that is no git repository works just as well: the review is the part
that needs one.\n"
    } else {
        ""
    };

    format!(
        "{command}

Opens a window on {opens}.

Usage:
  {command}
  {command} .
  {command} <path>
  {command} <before-path> <after-path>
  {command} <commit>
  {command} diff <target>
  {command} --pick
  {command} --repo <path>
  {command} --remote <host> [--repo <path>]

Examples:
  {command}
  {command} .
  {command} src/main.rs
  {command} before.json after.json
  {command} 4542abe
  {command} diff dev
  {command} --remote dev-box --repo /home/you/project

Run it inside any git repository you want to work in.
{opens_without_a_repo}`--pick` opens the window on its launch screen instead, which is where recent projects and
the folder picker are; it is what the Window menu's New Window items open.
`--repo <path>` opens the window on that repo rather than on the one this shell is in; it is
what the Window menu's Restart hands the instance it starts.
Run `{command} .` to limit the review to the current directory.
Pass one path to review only that file or directory's working-tree changes.
Pass two paths to review a read-only comparison of those files.

`{command} <commit>` opens a read-only review of a single commit.
`{command} diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`.

Reviewing another machine's repo:
  The window carries the review server inside it, so a window elsewhere can be pointed at
  this repo - `{PROGRAM} serve` is the same server without a window.
  `--remote <host>` opens the window against a `serve` on another machine, where the repo
  lives; `--repo <path>` then names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.
Changed submodules are offered inside the review, as extra reviews you can open from the
command palette.

Every other command - the other two windows, `open`, `list`, `serve` and the desktop
launchers - is in `{PROGRAM} --help`."
    )
}
