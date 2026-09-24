//! What each command line does: parsed in [`super::args`] and [`super::open`], run here.

use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::{
    FRAME_ENV, FRAMES, Frame, PROGRAM,
    args::{
        CliCommand, ReviewSource, ReviewTarget, current_dir_pathspec, parse_cli_args,
        review_open_request,
    },
    frame::frame_named,
    open,
};
use crate::{
    api::{DiffTarget, OpenSessionRequest},
    git::{find_repo_root, project_root},
    server,
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
    /// A pass key for this machine's server, printed alone so it can be piped - see
    /// [`crate::pass_keys`].
    GeneratePassKey,
    /// One file, in the window already open on its project - and in the window last in
    /// front when no window is open on it.
    Open {
        path: String,
        line: Option<usize>,
        /// `--wait`: return only once the file's tab has been closed, which is what git
        /// needs of the editor it hands a commit message to.
        wait: bool,
    },
    /// What `open` and `edit` take, printed and nothing opened.
    OpenHelp,
    /// Which windows are open, and what they are open on.
    ListWindows,
    /// moon's license, and the licenses and notices of everything it is built from.
    Licenses,
    /// A card on the board of the repo this shell is in, without opening a window on it. The
    /// task folder it made is printed, which is what the caller wanted it for.
    NewTask {
        title: String,
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
        MoonCommand::Open { path, line, wait } => open::open_file(&path, line, wait),
        MoonCommand::OpenHelp => {
            println!("{}", open::help_text());
            Ok(())
        }
        MoonCommand::ListWindows => open::list_windows(),
        MoonCommand::Licenses => print_licenses(),
        MoonCommand::NewTask { title } => new_task(&title),
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
        // The one word after a window's name that is not something to open it on. The board
        // is a folder of files, so a card can be made without a window - which is what an
        // agent asked to write itself a task needs.
        if frame == Frame::Tasks && rest.first().is_some_and(|word| word == "new") {
            // The window's help is where `new` is written up.
            if asks_for_help {
                return Ok(MoonCommand::Window {
                    frame,
                    args: vec!["--help".to_string()],
                });
            }
            return parse_new_task(&rest[1..]);
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
        "list" | "serve" | "licenses" | "install-launchers" | "generate-pass-key"
            if asks_for_help =>
        {
            Ok(MoonCommand::Help)
        }
        "list" => match rest.is_empty() {
            true => Ok(MoonCommand::ListWindows),
            false => bail!("`{PROGRAM} list` says which windows are open, so it takes nothing"),
        },
        "serve" => parse_serve(rest),
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

/// `moon tasks new <title>`. The title is the whole of the rest, joined, so it needs no
/// quoting - though it usually gets some.
fn parse_new_task(args: &[String]) -> Result<MoonCommand> {
    if let Some(option) = args.iter().find(|arg| arg.starts_with('-')) {
        bail!("`{PROGRAM} tasks new` takes the card's title and no options, not {option}");
    }
    let title = args.join(" ");
    if title.trim().is_empty() {
        bail!(
            "`{PROGRAM} tasks new` needs the card's title, e.g. `{PROGRAM} tasks new \"fix the races\"`"
        );
    }
    Ok(MoonCommand::NewTask { title })
}

/// Make a card on the board of the repo this shell is in and print the folder it was given.
///
/// It joins the top of the board's first column, which is where the board itself puts a card
/// nobody said anything else about: the leftmost column is the one work starts in.
fn new_task(title: &str) -> Result<()> {
    use crate::moontasks::{ColumnEnd, store};

    let repo_path =
        project_root(&env::current_dir().context("failed to read the current directory")?)?;
    let board = store::read_board(&repo_path);
    let column = board
        .columns
        .first()
        .context("the board has no columns to put a card in")?
        .id
        .clone();

    let task_id = store::create_task(&repo_path, title, &column, ColumnEnd::Top)?;
    println!("{}", store::task_dir(&repo_path, &task_id)?.display());
    Ok(())
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
  {PROGRAM} tasks new <title>         a card on this repo's board, with no window opened
  {PROGRAM} open <path>[:<line>]      a file, in the window on its project or the one last in front
  {PROGRAM} list                      which windows are open, and what they are on
  {PROGRAM} serve [--logs]            the review server, for a window on another machine or a
                                 browser; it prints a link that logs a browser in, once
  {PROGRAM} install-launchers         entries the OS offers for the three windows
  {PROGRAM} generate-pass-key         a pass key for this machine's server, printed alone
  {PROGRAM} licenses                  moon's license, and those of what it is built from
  {PROGRAM} --version
  {PROGRAM} --help

Examples:
  {PROGRAM} tasks
  {PROGRAM} tasks new \"fix the races\"
  {PROGRAM} review src/main.rs
  {PROGRAM} shell
  {PROGRAM} open src/main.rs:42

Open a window inside any git repository you want to work in. The board and the shell run just
as well in a folder that is no repository: the review is the part that needs one.

`{PROGRAM} <window> --help` says what that window can be opened on; `--pick` opens it on its
launch screen instead, and `--remote <host>` opens it against a `serve` on another machine.
`{PROGRAM} open --help` says how a file is named. Every command answers `--help` with its help
and does nothing else.

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

    // The board is the one window with a command that touches it without opening it.
    let makes_a_card_usage = if frame == Frame::Tasks {
        format!("\n  {command} new <title>")
    } else {
        String::new()
    };
    let makes_a_card = if frame == Frame::Tasks {
        "\n`{command} new <title>` writes a card on this repo's board and prints the folder it was
given, without opening a window. That folder is the task's: its notes, its brief, and whatever
an agent working on it leaves behind.\n"
            .replace("{command}", &command)
    } else {
        String::new()
    };

    format!(
        "{command}

Opens a window on {opens}.

Usage:
  {command}{makes_a_card_usage}
  {command} .
  {command} <path>
  {command} <before-path> <after-path>
  {command} <commit>
  {command} diff <target>
  {command} --pick
  {command} --repo <path>
  {command} --remote <host> [--pass-key <key>] [--repo <path>]

Examples:
  {command}
  {command} .
  {command} src/main.rs
  {command} before.json after.json
  {command} 4542abe
  {command} diff dev
  {command} --remote dev-box --repo /home/you/project

Run it inside any git repository you want to work in.
{opens_without_a_repo}{makes_a_card}`--pick` opens the window on its launch screen instead, which is where recent projects and
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
