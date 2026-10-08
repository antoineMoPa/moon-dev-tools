//! What each command line does: parsed in [`super::args`] and [`super::open`], run here.

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::{
    FRAME_ENV, FRAMES, Frame, PROGRAM, agent,
    args::{
        CliCommand, ReviewSource, ReviewTarget, current_dir_pathspec, parse_cli_args,
        review_open_request,
    },
    frame::frame_named,
    launch, open, tasks, wire,
};
use crate::{
    api::{DiffTarget, OpenSessionRequest},
    git::{find_repo_root, project_root},
    instances, server,
};

/// What the command line asked for. One executable, so this is the whole of what it does.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum MoonCommand {
    Help,
    Version,
    /// A window on one of the frames. What it opens on is the rest of the command line,
    /// which is [`super::args`]' business rather than this one's.
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
    /// A pass key for this machine's server so it can be piped - see
    /// [`crate::pass_keys`].
    GeneratePassKey,
    /// One file, in the window already open on its project - and in the window last in
    /// front when no window is open on it. A folder, once the path is read against the disk
    /// and turns out to be one, brings that window's file picker up on it instead.
    Open {
        path: String,
        line: Option<usize>,
        /// `--wait`: return only once the file's tab has been closed, which is what git
        /// needs of the editor it hands a commit message to.
        wait: bool,
    },
    /// What `open` and `edit` take, printed and nothing opened.
    OpenHelp,
    /// `moon shell <folder>`: a shell in that folder, as a tab of the window already open on
    /// its project - and of the window last in front when no window is open on it - the way
    /// [`MoonCommand::Open`] puts a file in one. A window of its own only when none is open.
    OpenShell {
        path: String,
    },
    /// `moon launch <command>`: a program with windows, started in the moon whose shell this
    /// was typed in - see [`super::launch`].
    Launch {
        /// A line of shell: the words of the command, each quoted to be read back as it was.
        command: String,
    },
    /// What `launch` takes, printed and nothing started.
    LaunchHelp,
    /// Which windows are open, and what they are open on.
    ListWindows,
    /// moon's license, and the licenses and notices of everything it is built from.
    Licenses,
    /// A card on the board of the repo this shell is in, without opening a window on it. The
    /// task folder it made is printed, which is what the caller wanted it for.
    NewTask {
        title: String,
    },
    /// The cards of that board, printed column by column.
    ListTasks,
    /// The card of the task this shell belongs to, moved to the column of the board that is
    /// called this.
    MoveTask {
        column: String,
    },
    /// A line from the task this shell belongs to, to the other agents of its board, and the
    /// wire's own help - see [`super::wire`].
    Wire(wire::WireCommand),
    /// An agent of this repo's board started, looked at or told something, and the help for
    /// doing so - see [`super::agent`].
    Agent(agent::AgentCommand),
    /// The window as the desktop of the X11 session it was started in: full screen, and the
    /// window manager of every program started from it - see
    /// [`crate::native::application_pane`]. This is what the session file of `os/` runs.
    Desktop {
        /// The folder the window opens on, which is the home directory unless one is named.
        path: Option<String>,
    },
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
        MoonCommand::GeneratePassKey => {
            println!(
                "{}",
                crate::pass_keys::PassKeys::for_this_machine()?.generate()
            );
            Ok(())
        }
        MoonCommand::Open { path, line, wait } => open::open(&path, line, wait),
        MoonCommand::OpenHelp => {
            println!("{}", open::help_text());
            Ok(())
        }
        MoonCommand::ListWindows => open::list_windows(),
        MoonCommand::OpenShell { path } => open_shell(&path),
        MoonCommand::Launch { command } => launch::launch(&command),
        MoonCommand::LaunchHelp => {
            println!("{}", launch::help_text());
            Ok(())
        }
        MoonCommand::Licenses => print_licenses(),
        MoonCommand::NewTask { title } => tasks::new_task(&title),
        MoonCommand::ListTasks => tasks::list_tasks(),
        MoonCommand::MoveTask { column } => tasks::move_task(&column),
        MoonCommand::Wire(command) => wire::run(command),
        MoonCommand::Agent(command) => agent::run(command),
        MoonCommand::Desktop { path } => open_desktop(path.as_deref()),
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
    // `--help` after any command is a question about that command, never a file to open or a
    // word of a card's title: `moon edit --help` used to open a tab on a file called `--help`,
    // which is what an agent finding its way around the CLI got for asking. A window's own
    // parser reads it for itself; every other command answers here.
    let asks_for_help = rest.iter().any(|arg| arg == "--help" || arg == "-h");

    if let Some(frame) = frame_named(&command) {
        // The words after a window's name that are not something to open it on. The board
        // is a folder of files, so its cards can be made, listed and moved without a window -
        // see [`super::tasks`].
        if frame == Frame::Tasks
            && let Some((word, words)) = rest.split_first()
            && let Some(card_command) = tasks::card_command_named(word)
        {
            // The window's help is where they are written up.
            if asks_for_help {
                return Ok(MoonCommand::Window {
                    frame,
                    args: vec!["--help".to_string()],
                });
            }
            return tasks::parse(card_command, words);
        }
        // `moon shell .` and `moon shell <folder>` are a shell in that folder, and a shell is
        // a tab: it joins a window that is already open, the way `moon edit` does, rather
        // than opening another window. Anything with an option in it is still the window's
        // own to read - `--pick`, `--repo`, `--remote` - and so is a bare `moon shell`.
        if frame == Frame::Shell
            && let [folder] = rest.as_slice()
            && !folder.starts_with('-')
        {
            return Ok(MoonCommand::OpenShell {
                path: folder.clone(),
            });
        }
        return Ok(MoonCommand::Window { frame, args: rest });
    }

    match command.as_str() {
        "--help" | "-h" | "help" => Ok(MoonCommand::Help),
        "--version" | "-v" => Ok(MoonCommand::Version),
        "open" | "edit" if asks_for_help => Ok(MoonCommand::OpenHelp),
        // `edit` and `open` are one thing said two ways: the tab it lands in is one that
        // edits the file, and both are words a hand reaches for.
        "open" | "edit" => open::parse_open(rest),
        // Ahead of the help every other command answers: what follows `launch` is a program's
        // own command line, and a `--help` in it is the program's to answer.
        "launch" => launch::parse_launch(rest),
        // The wire answers for its own help, which is where its rules are written.
        "wire" => Ok(MoonCommand::Wire(wire::parse(&rest, asks_for_help)?)),
        // So do the agents, for theirs.
        "agent" => Ok(MoonCommand::Agent(agent::parse(&rest, asks_for_help)?)),
        "list" | "serve" | "licenses" | "install-launchers" | "generate-pass-key" | "desktop"
            if asks_for_help =>
        {
            Ok(MoonCommand::Help)
        }
        "list" => match rest.is_empty() {
            true => Ok(MoonCommand::ListWindows),
            false => bail!("`{PROGRAM} list` says which windows are open, so it takes nothing"),
        },
        "serve" => parse_serve(rest),
        "desktop" => match rest.len() {
            0 => Ok(MoonCommand::Desktop { path: None }),
            1 => Ok(MoonCommand::Desktop {
                path: Some(rest[0].clone()),
            }),
            _ => bail!("`{PROGRAM} desktop` takes one folder to open on, or none"),
        },
        "licenses" => match rest.is_empty() {
            true => Ok(MoonCommand::Licenses),
            false => bail!("`{PROGRAM} licenses` takes nothing else"),
        },
        "install-launchers" => match rest.is_empty() {
            true => Ok(MoonCommand::InstallLaunchers),
            false => bail!("`{PROGRAM} install-launchers` takes nothing else"),
        },
        "generate-pass-key" => match rest.is_empty() {
            true => Ok(MoonCommand::GeneratePassKey),
            false => bail!("`{PROGRAM} generate-pass-key` takes nothing else"),
        },
        other => bail!("`{PROGRAM} {other}` is not a command\n\n{}", help_text()),
    }
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

/// `moon shell <folder>`: a shell in that folder, in a window that is already open - and,
/// when no window is open on this machine at all, a window of its own on the folder's
/// project, which is what `moon shell` typed there would open.
fn open_shell(path: &str) -> Result<()> {
    let folder = open::folder_to_open(path)?;
    match instances::open_shell(&folder)? {
        Some(instance) => {
            println!(
                "shell in {} → {} on {}",
                folder.display(),
                instance.program,
                instance.project_path
            );
            Ok(())
        }
        None => open_repo(&folder, Frame::Shell),
    }
}

/// `moon desktop`: the window as the whole session.
///
/// It is the shell window - a session is started from shells - opened on the home directory
/// unless a folder was named, full screen, and told to manage the session. Everything else a
/// desktop needs, it already had: frames to put windows in, a palette to start things from,
/// and the `applications` extension to say what there is to start.
fn open_desktop(path: Option<&str>) -> Result<()> {
    let folder = match path {
        Some(path) => PathBuf::from(path),
        None => dirs_home()?,
    };
    let repo_path = project_root(&folder).unwrap_or(folder);
    let launch = crate::native::launch_local(
        OpenSessionRequest {
            repo_path: repo_path.display().to_string(),
            diff_target: Some(DiffTarget::default()),
            active_commit: None,
        },
        Frame::Shell,
    )?;
    crate::native::run_desktop(launch)
}

/// The home directory of whoever is logged in, which is where a desktop session opens.
fn dirs_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("there is no HOME to open the desktop on"))
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
pub(super) fn folder_when_there_is_no_repo(current_dir: &Path) -> Result<PathBuf> {
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
    if let ReviewSource::Remote {
        target,
        repo_path,
        pass_key,
    } = source
    {
        // The repo lives on the far side, so nothing here is resolved against this machine.
        let pass_key = remote_pass_key(pass_key)?;
        let launch = crate::native::launch_remote(&target, pass_key, repo_path, frame)?;
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

/// The key a remote window is let in with: `--pass-key`, or else the one handed off in the
/// file [`MOON_PASS_KEY_FILE`](crate::pass_keys::handoff::PASS_KEY_FILE_ENV_VAR) names - how
/// a window started through its launcher is given its key, see `crate::native::programs` - or
/// else the environment's `MOON_PASS_KEY`. There is no asking the far side for one - the key is
/// what shows a window may ask it anything.
///
/// A handoff file is deleted as it is read, and one that cannot be read is an error rather
/// than a reason to look further: the window was told a key would be there.
fn remote_pass_key(given: Option<String>) -> Result<String> {
    use crate::{
        backend::remote::PASS_KEY_ENV_VAR,
        pass_keys::handoff::{HandedOver, PASS_KEY_FILE_ENV_VAR, handed_over, take_handed_off},
    };

    if let Some(key) = given {
        return Ok(key);
    }
    match handed_over() {
        Some(HandedOver::KeyFile(path)) => take_handed_off(path)
            .with_context(|| format!("{PASS_KEY_FILE_ENV_VAR} names no pass key to read")),
        Some(HandedOver::Key(key)) => key
            .to_str()
            .map(str::to_owned)
            .with_context(|| format!("{PASS_KEY_ENV_VAR} is not text")),
        None => bail!(
            "--remote needs a pass key for that server: run `{PROGRAM} generate-pass-key` on the \
             machine it runs on, and pass what it prints with --pass-key <key> or in \
             {PASS_KEY_ENV_VAR}"
        ),
    }
}

/// moon's own license.
const MOON_LICENSE: &str = include_str!("../../LICENSE");

/// The licenses and notices of the crates and files moon is built from, written by
/// `scripts/third-party-licenses.py`. Compiled in because an install keeps nothing but the
/// executable, so it is the one place they can travel with it.
pub(super) const THIRD_PARTY_LICENSES: &str = include_str!("../../THIRD_PARTY_LICENSES.txt");

/// Most of a megabyte, so it is read through a pager or `head` as often as not - and one that
/// stops reading is someone who has read enough, not an error.
fn print_licenses() -> Result<()> {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    match write!(stdout, "{MOON_LICENSE}\n\n{THIRD_PARTY_LICENSES}").and_then(|()| stdout.flush()) {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        written => Ok(written?),
    }
}

fn print_version() {
    println!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"));
}

/// What `moon --help` says next to each window's command. Shorter than [`Frame::opens`],
/// which the window's own help and the launchers still use.
const USAGE_LINE_OF_FRAME: &[(Frame, &str)] = &[
    (Frame::Tasks, "show task board"),
    (Frame::Review, "start a review"),
    (Frame::Shell, "start a shell"),
];

/// `moon --help`: every command there is, with what each window opens on.
pub(crate) fn help_text() -> String {
    let windows: Vec<String> = FRAMES
        .iter()
        .map(|frame| {
            format!(
                "  {command:<30} {opens}",
                command = format!("{} [<target>]", frame.command()),
                opens = USAGE_LINE_OF_FRAME
                    .iter()
                    .find(|(listed, _)| *listed == *frame)
                    .expect("every frame has a usage line")
                    .1
            )
        })
        .collect();

    format!(
        "{PROGRAM}

lunar local dev tools.

Usage:
{windows}
  {PROGRAM} tasks new <title>
  {PROGRAM} tasks list                show the board's cards
  {PROGRAM} tasks move <column>       move this shell's task to a column
  {PROGRAM} wire post <one line>      tell the board's other agents
  {PROGRAM} agent <command>           list, start, view or tell the board's agents
  {PROGRAM} edit <path>[:<line>]      Open a file for edition
  {PROGRAM} edit <folder>             pick a file of a folder to open
  {PROGRAM} open <path>[:<line>]      same as `{PROGRAM} edit`
  {PROGRAM} launch <command>          start a program with windows, from a moon's shell
  {PROGRAM} list                      show open windows
  {PROGRAM} serve [--logs]
  {PROGRAM} desktop [<folder>]        start as x11 desktop env
  {PROGRAM} install-launchers         add mac os launchers
  {PROGRAM} generate-pass-key
  {PROGRAM} licenses                  show licenses
  {PROGRAM} --version
  {PROGRAM} --help

Examples:
  {PROGRAM} tasks
  {PROGRAM} tasks new \"fix the races\"
  {PROGRAM} wire post \"rewriting src/cli\"
  {PROGRAM} agent tell fix-the-races \"start with the tests\"
  {PROGRAM} review src/main.rs
  {PROGRAM} shell
  {PROGRAM} shell .
  {PROGRAM} open src/main.rs:42",
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

    // The shell is the one window a folder opens as a tab of rather than a window on, so its
    // one path is a folder, and none of what a path narrows a review to is true of it.
    let (path_usage, path_example) = if frame == Frame::Shell {
        ("<folder>", "src")
    } else {
        ("<path>", "src/main.rs")
    };
    let shell_in_a_folder = if frame == Frame::Shell {
        "`{command} .` and `{command} <folder>` open a shell in that folder as a tab of a window
that is already open - the one on the folder's project, or the one last in front when no
window is open on it - the way `{PROGRAM} edit` puts a file in one. Only when no window is
open at all do they open a window on the folder's project, as `{command}` alone would.\n"
            .replace("{command}", &command)
            .replace("{PROGRAM}", PROGRAM)
    } else {
        String::new()
    };
    let narrows_the_review = if frame == Frame::Shell {
        String::new()
    } else {
        "Run `{command} .` to limit the review to the current directory.
Pass one path to review only that file or directory's working-tree changes.
Pass two paths to review a read-only comparison of those files.\n"
            .replace("{command}", &command)
    };

    // The board is the one window with commands that touch it without opening it - see
    // [`super::tasks`].
    let card_commands_usage = if frame == Frame::Tasks {
        format!("\n  {command} new <title>\n  {command} list\n  {command} move <column>")
    } else {
        String::new()
    };
    let card_commands_examples = if frame == Frame::Tasks {
        format!(
            "\n  {command} new \"fix the races\"\n  {command} list\n  {command} move IN PROGRESS"
        )
    } else {
        String::new()
    };
    let card_commands = if frame == Frame::Tasks {
        "\n`{command} new <title>` writes a card on this repo's board and prints the folder it was
given, without opening a window. That folder is the task's: its notes, its brief, and whatever
an agent working on it leaves behind.
`{command} list` prints that board without opening a window either: each column by its name,
left to right, and under it its cards from the top, each as the task's folder and its title.
`{command} move <column>` moves the card of the task this shell belongs to into the column
the board shows under that name, written as the board writes it and with no quotes needed:
`{command} move IN PROGRESS`. It is run from a task's shell, where {TASK_DIR_ENV_VAR}
says which task that is, and the card arrives the way one dragged there does.\n"
            .replace("{command}", &command)
            .replace("{TASK_DIR_ENV_VAR}", crate::moontasks::TASK_DIR_ENV_VAR)
    } else {
        String::new()
    };

    format!(
        "{command}

Opens a window on {opens}.

Usage:
  {command}{card_commands_usage}
  {command} .
  {command} {path_usage}
  {command} <before-path> <after-path>
  {command} <commit>
  {command} diff <target>
  {command} --pick
  {command} --repo <path>
  {command} --remote <host> [--pass-key <key>] [--repo <path>]

Examples:
  {command}{card_commands_examples}
  {command} .
  {command} {path_example}
  {command} before.json after.json
  {command} 4542abe
  {command} diff dev
  {command} --remote dev-box --repo /home/you/project

Run it inside any git repository you want to work in.
{opens_without_a_repo}{card_commands}{shell_in_a_folder}`--pick` opens the window on its launch screen instead, which is where recent projects and
the folder picker are; it is what the Window menu's New Window items open.
`--repo <path>` opens the window on that repo rather than on the one this shell is in; it is
what the Window menu's Restart hands the instance it starts.
{narrows_the_review}
`{command} <commit>` opens a read-only review of a single commit.
`{command} diff <target>` opens a read-only diff review against a git target.
Use `branch:pathspec` to limit the diff to part of the repo, for example `dev:./`.

Reviewing another machine's repo:
  The window carries the review server inside it, so a window elsewhere can be pointed at
  this repo - `{PROGRAM} serve` is the same server without a window.
  `--remote <host>` opens the window against a `serve` on another machine, where the repo
  lives; `--repo <path>` then names the path there, and without it the window asks.
  `--remote` accepts `host`, `host:port` or a URL, and defaults to port 42000.
  The server lets in only a window that shows it a pass key: run `{PROGRAM} generate-pass-key`
  on that machine, and pass what it prints with `--pass-key <key>` - or in {pass_key_env},
  which keeps it out of the process list other users can read.
Changed submodules are offered inside the review, as extra reviews you can open from the
command palette.

Every other command - the other two windows, `open`, `list`, `serve` and the desktop
launchers - is in `{PROGRAM} --help`.",
        pass_key_env = crate::backend::remote::PASS_KEY_ENV_VAR,
    )
}
