//! `moon wire`: a line from one agent of a board to the others - see
//! [`crate::moontasks::wire`] for what the wire is, and [`help_text`] for what an agent is
//! told about it.
//!
//! It is run from a task's shell, and reads who is posting out of the environment that
//! shell was started with rather than out of the folder it is in: an agent may have moved
//! into a submodule, which is another repo with another board, or none.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use super::PROGRAM;
use crate::{
    instances::{self, Ask, Instance},
    moontasks::{
        TASK_DIR_ENV_VAR,
        store::{self, TASKS_DIR_NAME},
        wire::{self, broadcasts, direct, handles},
    },
};

/// What `moon wire …` asked for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WireCommand {
    Help,
    /// `moon wire post …`: a line for the tasks tagged in front of it, or for every agent of
    /// the board when none is.
    Post {
        /// The tags the line started with, without their `@`.
        tags: Vec<String>,
        message: String,
    },
}

/// What marks a word in front of a line as a tag: `@fix-the-races`.
const TAG_MARK: char = '@';

/// Which of the wire's commands a command line names, from the words after `wire`.
///
/// `moon wire` alone is the help, as `moon` alone is: there is nothing else it could mean.
pub(super) fn parse(args: &[String], asks_for_help: bool) -> Result<WireCommand> {
    let Some((command, words)) = args.split_first() else {
        return Ok(WireCommand::Help);
    };
    if asks_for_help {
        return Ok(WireCommand::Help);
    }
    match command.as_str() {
        "post" => parse_post(words),
        other => bail!(
            "`{PROGRAM} wire {other}` is not a command\n\n{}",
            help_text()
        ),
    }
}

/// `moon wire post <words…>`. The line is the whole of the rest, joined, so it needs no
/// quoting - though it usually gets some, and a quoted line arrives as one word with its
/// tags inside it. So the tags are read off the front of the joined line: every word that
/// starts with `@`, up to the first that does not.
fn parse_post(words: &[String]) -> Result<WireCommand> {
    let line = words.join(" ");
    let mut message = line.trim();
    let mut tags = Vec::new();
    while let Some(tagged) = message.strip_prefix(TAG_MARK) {
        let (tag, rest) = tagged
            .split_once(char::is_whitespace)
            .unwrap_or((tagged, ""));
        if tag.is_empty() {
            bail!("`{TAG_MARK}` alone tags nobody: a tag is `{TAG_MARK}handle`, with no space");
        }
        tags.push(tag.to_string());
        message = rest.trim_start();
    }
    if message.is_empty() {
        match tags.as_slice() {
            [] => bail!(
                "`{PROGRAM} wire post` needs the line to post, e.g. `{PROGRAM} wire post \
                 \"rewriting src/cli\"`"
            ),
            _ => bail!(
                "`{PROGRAM} wire post` needs a message after the tags, e.g. `{PROGRAM} wire \
                 post \"{TAG_MARK}{} are you in src/cli?\"`",
                tags[0]
            ),
        }
    }
    Ok(WireCommand::Post {
        tags,
        message: message.to_string(),
    })
}

pub(super) fn run(command: WireCommand) -> Result<()> {
    match command {
        WireCommand::Help => {
            println!("{}", help_text());
            Ok(())
        }
        WireCommand::Post { tags, message } => {
            post(&tags, &message, std::env::var_os(TASK_DIR_ENV_VAR))
        }
    }
}

/// The task a line is posted from, read off the folder its shell was told it is in.
#[derive(Debug, PartialEq, Eq)]
struct Sender {
    /// The repo whose board the task is on.
    repo_path: PathBuf,
    task_id: String,
}

impl Sender {
    /// The task whose folder this is: `<repo>/.moontasks/<task id>`, as every process moon
    /// starts for a task is given it. Anything else is not a task's shell, and is refused
    /// rather than read as the nearest thing to one.
    fn of(task_dir: Option<&OsStr>) -> Result<Self> {
        let Some(task_dir) = task_dir.filter(|task_dir| !task_dir.is_empty()) else {
            bail!(
                "`{}` is run from a task's shell, and this is not one: {TASK_DIR_ENV_VAR} is \
                 not set",
                wire::post_command()
            );
        };
        let task_dir = Path::new(task_dir);
        let not_a_task_folder = || {
            format!(
                "{TASK_DIR_ENV_VAR} is {}, which is not a task's folder in a board's \
                 {TASKS_DIR_NAME}",
                task_dir.display()
            )
        };
        let task_id = task_dir
            .file_name()
            .and_then(OsStr::to_str)
            .with_context(not_a_task_folder)?;
        let board_dir = task_dir.parent().with_context(not_a_task_folder)?;
        if board_dir.file_name() != Some(OsStr::new(TASKS_DIR_NAME)) {
            bail!(not_a_task_folder());
        }
        let repo_path = board_dir.parent().with_context(not_a_task_folder)?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            task_id: task_id.to_string(),
        })
    }
}

/// One shell a direct message is to be typed into, with everything checked that can be
/// before anything is sent.
struct Delivery {
    /// The tag as it was typed, which is how the poster knows the task.
    tag: String,
    window: Instance,
    line: Ask,
}

/// Post a line: to the board's file when it has no tags, and into the shells of the tagged
/// tasks' agents when it has.
fn post(tags: &[String], message: &str, task_dir: Option<OsString>) -> Result<()> {
    let sender = Sender::of(task_dir.as_deref())?;
    let message = wire::one_line(message)?;
    let folders = store::list_task_ids(&sender.repo_path)?;
    if !folders.contains(&sender.task_id) {
        bail!(
            "{} is not a task of the board in {}",
            sender.task_id,
            store::tasks_root(&sender.repo_path).display()
        );
    }
    let handle = handles::handle_of(&sender.task_id, &folders);

    if tags.is_empty() {
        broadcasts::post(&sender.repo_path, &handle, message)?;
        println!(
            "posted as {TAG_MARK}{handle} to {}",
            broadcasts::file_path(&sender.repo_path).display()
        );
        return Ok(());
    }

    let deliveries = deliveries(&sender.repo_path, &folders, &handle, tags, message)?;
    for delivery in deliveries {
        instances::wire(&delivery.window, &delivery.line)
            .with_context(|| format!("{TAG_MARK}{} was not sent the line", delivery.tag))?;
        println!("sent to {TAG_MARK}{}", delivery.tag);
    }
    Ok(())
}

/// Every shell a direct message goes to, or the first reason it cannot go to one of them.
///
/// Worked out whole before anything is sent, so a tag that names nobody, or a task nobody is
/// running in, leaves every other tagged agent untold rather than half of them told.
fn deliveries(
    repo_path: &Path,
    folders: &[String],
    sender: &str,
    tags: &[String],
    message: &str,
) -> Result<Vec<Delivery>> {
    let mut tagged: Vec<(&String, &str)> = Vec::new();
    for tag in tags {
        let task_id = match handles::resolve(tag, folders) {
            Ok(task_id) => task_id,
            Err(unknown @ handles::Unresolved::Unknown { .. }) => {
                bail!("{unknown}. {}", agents_to_tag(repo_path, folders, sender)?)
            }
            Err(ambiguous) => return Err(ambiguous.into()),
        };
        // A task tagged twice is told once.
        if !tagged.iter().any(|(_, already)| *already == task_id) {
            tagged.push((tag, task_id));
        }
    }

    let mut deliveries = Vec::new();
    for (tag, task_id) in tagged {
        let recipient = handles::handle_of(task_id, folders);
        let agents = direct::running_agents(&store::read_task(repo_path, task_id)?);
        if agents.is_empty() {
            bail!("{TAG_MARK}{tag} has no running agent");
        }
        for agent in agents {
            // Only a window listens for a line. A `moon serve` holds shells too, and has
            // nothing to be asked on.
            let Some(window) = instances::window_of(agent.held_by) else {
                bail!(
                    "{TAG_MARK}{tag}'s agent runs in a moon with no window (process {}), such \
                     as `{PROGRAM} serve`, and only a window can be asked to type a line into \
                     a shell",
                    agent.held_by
                );
            };
            deliveries.push(Delivery {
                tag: tag.clone(),
                window,
                line: Ask::Wire {
                    terminal_id: agent.terminal_id,
                    sender: sender.to_string(),
                    recipient: recipient.clone(),
                    message: message.to_string(),
                },
            });
        }
    }
    Ok(deliveries)
}

/// Who there is to tag, said to a poster whose tag named nobody: the board's other tasks
/// with an agent running. Not every task of the board - a board keeps its finished cards,
/// and theirs are handles nobody answers to.
fn agents_to_tag(repo_path: &Path, folders: &[String], sender: &str) -> Result<String> {
    let mut running = Vec::new();
    for folder in folders {
        if direct::running_agents(&store::read_task(repo_path, folder)?).is_empty() {
            continue;
        }
        let handle = handles::handle_of(folder, folders);
        if handle != sender {
            running.push(format!("{TAG_MARK}{handle}"));
        }
    }
    Ok(match running.as_slice() {
        [] => "No other task of it has an agent running".to_string(),
        _ => format!("Its tasks with an agent running: {}", running.join(", ")),
    })
}

/// `moon wire --help`: the whole of the wire's rules. An agent is told one sentence about
/// the wire as it starts - see `coordination_brief` in [`crate::moontasks`] - and comes here
/// for the rest, which is the case when a post was refused.
pub(crate) fn help_text() -> String {
    let wire_file = crate::moontasks::wire_repo_path();
    let kept = broadcasts::KEPT_LINES;
    format!(
        "{PROGRAM} wire

How the agents working on one board tell each other what they are doing.

Usage:
  {PROGRAM} wire post <one line>              to every agent of the board
  {PROGRAM} wire post @<handle> <one line>    to one agent, typed into its shell
  {PROGRAM} wire --help

Examples:
  {PROGRAM} wire post \"rewriting src/cli/args.rs and its tests\"
  {PROGRAM} wire post \"@fix-the-races are you still in src/terminal.rs?\"

Run it from a task's shell. Who is posting is read from {TASK_DIR_ENV_VAR}, which every
shell and agent started from the board is given, so it works from any folder.

A line with no tag in front is appended to {wire_file}, dated and signed
with your handle, and the file is cut back to its latest {kept} lines. Read that file; never
write to it. This command is its only writer, and it refuses to post while the file holds a
line it did not write, naming the line.

A line that starts with one or more @handle words goes to those agents instead, and not
to the file. The moon window holding each agent's shell types
`agent @<your handle> sent this message: <line>` into it and presses Enter - once the agent
is not waiting on an answer to something it asked, and nobody has typed in that shell for a
few seconds. Until then the line waits, and the lines sent after it wait behind it.

A handle is the front of a task's folder name: the words of its title, as in
@fix-the-races. Two tasks with the same words each take the first characters of their id
as well, as in @fix-the-races-3f2; any longer start of the folder name works too. The
board's own task is @{board_task}.

Refused, with nothing posted and nothing sent:
  - an empty line, and a line with a line break or any other control character in it
  - tags with no message after them
  - a tag that is no task of the board, which lists the tasks with an agent running, or
    that is several tasks, which lists those
  - a tagged task with no running agent, or whose agent runs in `{PROGRAM} serve` rather
    than in a window",
        board_task = store::BOARD_TASK_ID,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::AgentKind,
        instances::window::ShellAsks,
        moontasks::{
            ColumnEnd, ColumnId,
            store::{TaskResource, TaskResourceKind},
        },
    };

    /// A pid nothing is running under: higher than any pid a system hands out.
    const PID_OF_NOTHING: u32 = 4_194_303;

    /// A repo with a board of two cards, `Fix the races` and `Bing bong`, and their ids.
    fn board(name: &str) -> (PathBuf, String, String) {
        let repo = std::env::temp_dir().join(format!(
            "moonreview-wire-cli-{}-{name}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        std::fs::create_dir_all(&repo).expect("failed to create the test repo");
        let card = |title: &str| {
            store::create_task(&repo, title, &ColumnId::new("todo"), ColumnEnd::Top)
                .expect("expected the card to be made")
        };
        let (races, bing_bong) = (card("Fix the races"), card("Bing bong"));
        (repo, races, bing_bong)
    }

    /// The folder a shell of this task is told it is in.
    fn task_dir(repo: &Path, task_id: &str) -> Option<OsString> {
        Some(
            store::task_dir(repo, task_id)
                .expect("expected the task's folder")
                .into_os_string(),
        )
    }

    /// Write an agent run on a task, in a shell of this id held by this process.
    fn run_an_agent(repo: &Path, task_id: &str, terminal_id: &str, held_by: u32) {
        let mut metadata = store::read_task(repo, task_id).expect("expected the task");
        metadata.resources.push(TaskResource {
            id: store::new_uuid(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: Some(terminal_id.to_string()),
            terminal_owner: Some(held_by),
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
        });
        store::write_task(repo, task_id, &metadata).expect("expected the run to be written");
    }

    fn kept(repo: &Path) -> String {
        std::fs::read_to_string(broadcasts::file_path(repo)).unwrap_or_default()
    }

    fn tags(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|tag| tag.to_string()).collect()
    }

    #[test]
    fn the_sender_is_the_task_whose_folder_the_shell_was_told() {
        let sender = Sender::of(Some(OsStr::new(
            "/repos/project/.moontasks/fix-the-races-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a",
        )))
        .expect("expected a sender");

        assert_eq!(
            sender,
            Sender {
                repo_path: PathBuf::from("/repos/project"),
                task_id: "fix-the-races-6f9c1e2a-0b1c-4d2e-8f3a-9b8c7d6e5f4a".to_string(),
            }
        );
    }

    /// Outside a task's shell there is nobody for the line to be from, and the folder the
    /// command was typed in is not asked instead.
    #[test]
    fn a_post_from_outside_a_tasks_shell_is_refused() {
        for unset in [None, Some(OsString::new())] {
            let error = post(&[], "rewriting src/cli", unset).expect_err("expected a refusal");
            assert!(
                error.to_string().contains("is run from a task's shell")
                    && error.to_string().contains(TASK_DIR_ENV_VAR),
                "{error}"
            );
        }

        for not_a_task_folder in ["/repos/project", "/repos/project/src/task", "/"] {
            let error =
                Sender::of(Some(OsStr::new(not_a_task_folder))).expect_err("expected a refusal");
            assert!(
                error.to_string().contains("is not a task's folder"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_post_from_a_folder_that_is_no_task_of_the_board_is_refused() {
        let (repo, _, _) = board("no-task");
        let stray = store::tasks_root(&repo).join("no-such-task");

        let error = post(&[], "rewriting src/cli", Some(stray.into_os_string()))
            .expect_err("expected a refusal");

        assert!(
            error
                .to_string()
                .contains("no-such-task is not a task of the board"),
            "{error}"
        );
        assert_eq!(kept(&repo), "");
    }

    #[test]
    fn a_line_with_no_tags_is_posted_to_the_boards_file_signed_with_the_handle() {
        let (repo, races, bing_bong) = board("broadcast");

        post(&[], "rewriting src/terminal.rs", task_dir(&repo, &races))
            .expect("expected the first post");
        post(&[], "in src/cli", task_dir(&repo, &bing_bong)).expect("expected the second post");

        let kept = kept(&repo);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines.len(), 2, "got {kept}");
        assert!(
            lines[0].ends_with(" @fix-the-races: rewriting src/terminal.rs"),
            "got {kept}"
        );
        assert!(lines[1].ends_with(" @bing-bong: in src/cli"), "got {kept}");
    }

    #[test]
    fn a_line_break_in_the_line_is_refused() {
        let (repo, races, _) = board("line-break");

        let error = post(&[], "one\ntwo", task_dir(&repo, &races)).expect_err("expected a refusal");

        assert!(error.to_string().contains("line break"), "{error}");
        assert_eq!(kept(&repo), "");
    }

    /// A tag that names nobody stops the whole post, saying who there is to tag: the other
    /// tasks with an agent running, and not the cards nobody is working on. A tagged line is
    /// never written to the file.
    #[test]
    fn a_tag_that_names_no_task_sends_nothing_and_lists_the_agents_running() {
        let (repo, races, bing_bong) = board("unknown-tag");
        let idle =
            store::create_task(&repo, "Nobody home", &ColumnId::new("done"), ColumnEnd::Top)
                .expect("expected the card to be made");
        let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
            .expect("expected a socket");
        asks.on_project(&repo.display().to_string())
            .expect("expected the record to be written");
        run_an_agent(&repo, &races, "terminal-a", std::process::id());
        run_an_agent(&repo, &bing_bong, "terminal-b", std::process::id());
        // A run whose moon has gone is not running.
        run_an_agent(&repo, &idle, "terminal-c", PID_OF_NOTHING);

        let error = post(
            &tags(&["bing-bong", "nobody"]),
            "hello",
            task_dir(&repo, &races),
        )
        .expect_err("expected a refusal");

        assert_eq!(
            error.to_string(),
            "@nobody is no task of this board. Its tasks with an agent running: @bing-bong"
        );
        assert!(
            asks.drain_wired().is_empty(),
            "nothing is sent on a bad tag"
        );
        assert_eq!(kept(&repo), "");
    }

    #[test]
    fn a_tagged_task_with_no_running_agent_is_refused() {
        let (repo, races, bing_bong) = board("nobody-running");
        // A run whose moon is gone is not running.
        run_an_agent(&repo, &bing_bong, "terminal-a", PID_OF_NOTHING);

        let error = post(&tags(&["bing-bong"]), "hello", task_dir(&repo, &races))
            .expect_err("expected a refusal");

        assert_eq!(error.to_string(), "@bing-bong has no running agent");
    }

    /// An agent whose shell is held by a moon with no window - this test process, which
    /// listens on nothing - is one nobody can be asked to type into.
    #[test]
    fn an_agent_held_by_a_moon_with_no_window_is_refused_with_the_reason() {
        let (repo, races, bing_bong) = board("no-window");
        run_an_agent(&repo, &bing_bong, "terminal-a", std::process::id());

        let error = post(&tags(&["bing-bong"]), "hello", task_dir(&repo, &races))
            .expect_err("expected a refusal");

        assert!(
            error.to_string().contains("runs in a moon with no window")
                && error
                    .to_string()
                    .contains(&format!("process {}", std::process::id())),
            "{error}"
        );
    }

    /// The whole of a direct message, over the real socket: every running agent of every
    /// tagged task is sent the line once, by the window holding its shell, in its parts.
    #[test]
    fn a_tagged_line_reaches_the_window_holding_each_agents_shell() {
        let (repo, races, bing_bong) = board("direct");
        let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
            .expect("expected a socket");
        asks.on_project(&repo.display().to_string())
            .expect("expected the record to be written");
        run_an_agent(&repo, &bing_bong, "terminal-a", std::process::id());
        run_an_agent(&repo, &bing_bong, "terminal-b", std::process::id());

        post(
            &tags(&["bing-bong", bing_bong.as_str()]),
            "are you in src/cli?",
            task_dir(&repo, &races),
        )
        .expect("expected the line to be sent");

        let wired = asks.drain_wired();
        assert_eq!(
            wired
                .iter()
                .map(|line| line.terminal_id.as_str())
                .collect::<Vec<_>>(),
            ["terminal-a", "terminal-b"],
            "a task tagged twice is told once, in each of its agents' shells"
        );
        for line in &wired {
            assert_eq!(line.sender, "fix-the-races");
            assert_eq!(line.recipient, "bing-bong");
            assert_eq!(line.message, "are you in src/cli?");
        }
        assert_eq!(kept(&repo), "", "a tagged line is not posted to the file");
    }
}
