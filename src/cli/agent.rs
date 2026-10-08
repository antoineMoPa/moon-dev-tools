//! `moon agent`: the agents of a board listed, and one started, looked at and told
//! something, from a command line rather than from the board's window - see [`help_text`] for what somebody running it
//! is told.
//!
//! It is run in the repo whose board the task is on, the way `moon tasks new` is, and a task
//! is named by its handle, the way the wire names one - see [`handles`]. None of it is done
//! here: an agent runs in a shell a moon window holds, so each command finds the window and
//! asks it - see [`crate::instances`].

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::PROGRAM;
use crate::{
    api::AgentKind,
    git::project_root,
    instances::{self, Instance},
    moontasks::{
        AGENT_LAUNCHES,
        store::{self, TaskMetadata},
        wire::{self, direct, handles},
    },
    terminal::Shown,
};

/// What `moon agent …` asked for. A task is carried as the handle that was typed, without
/// its `@`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AgentCommand {
    Help,
    /// `moon agent list`.
    List,
    /// `moon agent start <task> <agent>`.
    Start {
        task: String,
        agent: AgentKind,
    },
    /// `moon agent view <task> [--lines <n>]`.
    View {
        task: String,
        wanted: Shown,
    },
    /// `moon agent tell <task> <one line>`.
    Tell {
        task: String,
        line: String,
    },
}

/// What a handle may be written with in front of it, as it is on the wire: `@fix-the-races`.
const HANDLE_MARK: char = '@';

/// The option of `view` that asks for rows from the bottom of everything the agent has
/// printed, rather than for its screen.
const LINES_OPTION: &str = "--lines";

/// Which of the commands a command line names, from the words after `agent`.
///
/// `moon agent` alone is the help, as `moon wire` alone is.
pub(super) fn parse(args: &[String], asks_for_help: bool) -> Result<AgentCommand> {
    let Some((command, words)) = args.split_first() else {
        return Ok(AgentCommand::Help);
    };
    if asks_for_help {
        return Ok(AgentCommand::Help);
    }
    match command.as_str() {
        "list" if words.is_empty() => Ok(AgentCommand::List),
        "list" => bail!(
            "`{PROGRAM} agent list` lists the agents running on this repo's board, so it takes \
             nothing"
        ),
        "start" => parse_start(words),
        "view" => parse_view(words),
        "tell" => parse_tell(words),
        other => bail!(
            "`{PROGRAM} agent {other}` is not a command\n\n{}",
            help_text()
        ),
    }
}

/// The handle a task was named by, without the `@` it may have been typed with.
fn handle(word: &str) -> Result<String> {
    let handle = word.strip_prefix(HANDLE_MARK).unwrap_or(word);
    if handle.is_empty() || handle.starts_with('-') {
        bail!("{word:?} names no task: a task is named by its handle, as in `fix-the-races`");
    }
    Ok(handle.to_string())
}

fn parse_start(words: &[String]) -> Result<AgentCommand> {
    let [task, agent] = words else {
        bail!(
            "`{PROGRAM} agent start` takes the task and the agent to start on it, e.g. \
             `{PROGRAM} agent start fix-the-races {}`",
            agent_names()[0]
        );
    };
    Ok(AgentCommand::Start {
        task: handle(task)?,
        agent: agent_named(agent)?,
    })
}

fn parse_view(words: &[String]) -> Result<AgentCommand> {
    let usage = || {
        format!(
            "`{PROGRAM} agent view` takes the task, and `{LINES_OPTION} <n>` for its last n \
             lines rather than its screen, e.g. `{PROGRAM} agent view fix-the-races \
             {LINES_OPTION} 40`"
        )
    };
    let (task, wanted) = match words {
        [task] => (task, Shown::Screen),
        [task, option, count] if option == LINES_OPTION => (task, Shown::LastRows(rows(count)?)),
        [task, option] => {
            let count = option
                .strip_prefix(LINES_OPTION)
                .and_then(|count| count.strip_prefix('='))
                .with_context(usage)?;
            (task, Shown::LastRows(rows(count)?))
        }
        _ => bail!(usage()),
    };
    Ok(AgentCommand::View {
        task: handle(task)?,
        wanted,
    })
}

/// How many lines `--lines` asked for, which is at least one.
fn rows(count: &str) -> Result<usize> {
    match count.parse::<usize>() {
        Ok(count) if count > 0 => Ok(count),
        _ => bail!("`{LINES_OPTION}` takes how many lines to show, and {count:?} is not a count"),
    }
}

/// `moon agent tell <task> <words…>`. The line is the whole of the rest, joined, so it needs
/// no quoting - though it usually gets some.
fn parse_tell(words: &[String]) -> Result<AgentCommand> {
    let needs_both = || {
        format!(
            "`{PROGRAM} agent tell` takes the task and the line to tell its agent, e.g. \
             `{PROGRAM} agent tell fix-the-races \"start with the tests\"`"
        )
    };
    let (task, line) = words.split_first().with_context(needs_both)?;
    let line = line.join(" ");
    if line.trim().is_empty() {
        bail!(needs_both());
    }
    Ok(AgentCommand::Tell {
        task: handle(task)?,
        line: line.trim().to_string(),
    })
}

/// What an agent is called on the command line: `claude`, `opencode`.
fn name_of(agent: AgentKind) -> String {
    agent.label().to_lowercase()
}

/// Every agent moon starts, by that name.
fn agent_names() -> Vec<String> {
    AGENT_LAUNCHES
        .iter()
        .map(|launch| name_of(launch.kind))
        .collect()
}

fn agent_named(name: &str) -> Result<AgentKind> {
    AGENT_LAUNCHES
        .iter()
        .map(|launch| launch.kind)
        .find(|agent| name_of(*agent) == name)
        .with_context(|| {
            format!(
                "{name} is no agent {PROGRAM} starts: it starts {}",
                agent_names().join(", ")
            )
        })
}

pub(super) fn run(command: AgentCommand) -> Result<()> {
    if command == AgentCommand::Help {
        println!("{}", help_text());
        return Ok(());
    }
    let repo_path =
        project_root(&std::env::current_dir().context("failed to read the current directory")?)?;
    let said = match command {
        AgentCommand::Help => unreachable!("the help was printed above"),
        AgentCommand::List => list(&repo_path)?,
        AgentCommand::Start { task, agent } => start(&repo_path, &task, agent)?,
        AgentCommand::View { task, wanted } => view(&repo_path, &task, wanted)?,
        AgentCommand::Tell { task, line } => tell(&repo_path, &task, &line)?,
    };
    // What a shell is showing ends in the line break of its last row.
    print!("{said}");
    Ok(())
}

/// A task of the board in a repo, as one of these commands named it.
struct Task {
    id: String,
    /// Its handle on this board, which is what is said back - the one that was typed may be
    /// a longer start of the folder's name.
    handle: String,
    metadata: TaskMetadata,
}

impl Task {
    fn named(repo_path: &Path, typed: &str) -> Result<Self> {
        let folders = store::list_task_ids(repo_path)?;
        let id = match handles::resolve(typed, &folders) {
            Ok(id) => id,
            Err(unknown @ handles::Unresolved::Unknown { .. }) => {
                bail!("{unknown}, {}", store::tasks_root(repo_path).display())
            }
            Err(ambiguous) => return Err(ambiguous.into()),
        };
        Self::in_folder(repo_path, id, &folders)
    }

    /// The task in one of the board's folders, which are all of `folders`.
    fn in_folder(repo_path: &Path, id: &str, folders: &[String]) -> Result<Self> {
        Ok(Self {
            id: id.to_string(),
            handle: handles::handle_of(id, folders),
            metadata: store::read_task(repo_path, id)?,
        })
    }

    /// The task's agents that are running, each with the window holding its shell - or why
    /// there is nothing to tell or to look at.
    fn running_agents(&self) -> Result<Vec<Running>> {
        let handle = &self.handle;
        let agents = direct::running_agents(&self.metadata);
        if agents.is_empty() {
            bail!(
                "{HANDLE_MARK}{handle} has no running agent: start one with `{PROGRAM} agent \
                 start {handle} <agent>`"
            );
        }
        agents
            .into_iter()
            .map(|agent| {
                // Only a window answers this. A `moon serve` holds shells too, and answers
                // `moon launch` alone.
                let window = instances::window_of(agent.held_by).with_context(|| {
                    format!(
                        "{HANDLE_MARK}{handle}'s agent runs in a moon with no window (process \
                         {}), such as `{PROGRAM} serve`, and only a window can be asked about \
                         a shell it holds",
                        agent.held_by
                    )
                })?;
                Ok(Running {
                    run: self.run_in(&agent.terminal_id),
                    terminal_id: agent.terminal_id,
                    window,
                })
            })
            .collect()
    }

    /// What the run in a shell is called, and the shell's own id for a run written down
    /// before runs were named.
    fn run_in(&self, terminal_id: &str) -> String {
        self.metadata
            .resources
            .iter()
            .find(|resource| resource.terminal_id.as_deref() == Some(terminal_id))
            .and_then(|resource| resource.name.clone())
            .unwrap_or_else(|| terminal_id.to_string())
    }
}

/// One running agent of a task, as it is reached.
struct Running {
    /// What its run is called - `fix the races claude - 1`.
    run: String,
    terminal_id: String,
    window: Instance,
}

/// Every agent running on the board in a repo, a line each: the handle of its task, which is
/// what `view` and `tell` take, and what its run is called. In the order the board reads its
/// tasks in, and a task's agents in the order they were started.
///
/// An agent held by a moon with no window is listed with the rest: it is running, though
/// nothing here can tell it anything.
fn list(repo_path: &Path) -> Result<String> {
    let folders = store::list_task_ids(repo_path)?;
    let mut running: Vec<(String, String)> = Vec::new();
    for folder in &folders {
        let task = Task::in_folder(repo_path, folder, &folders)?;
        for agent in direct::running_agents(&task.metadata) {
            running.push((
                format!("{HANDLE_MARK}{}", task.handle),
                task.run_in(&agent.terminal_id),
            ));
        }
    }
    if running.is_empty() {
        return Ok(format!(
            "no agent is running on the board in {}\n",
            store::tasks_root(repo_path).display()
        ));
    }
    let widest = running
        .iter()
        .map(|(handle, _)| handle.chars().count())
        .max()
        .expect("an agent is running");
    Ok(running
        .iter()
        .map(|(handle, run)| format!("{handle:<widest$}  {run}\n"))
        .collect())
}

/// Start an agent on a task, in the window open on the board's repo, and say what its run is
/// called.
fn start(repo_path: &Path, task: &str, agent: AgentKind) -> Result<String> {
    let task = Task::named(repo_path, task)?;
    let window = instances::window_on_board(repo_path).with_context(|| {
        format!(
            "no {PROGRAM} window is open on {}: an agent runs in a shell a window holds, so \
             open one there with `{PROGRAM} tasks`",
            repo_path.display()
        )
    })?;
    let run = instances::start_agent(&window, &task.id, agent)?;
    Ok(format!("started {run} on {HANDLE_MARK}{}\n", task.handle))
}

/// What a task's agent is showing. A task with several running shows each under the name of
/// its run.
fn view(repo_path: &Path, task: &str, wanted: Shown) -> Result<String> {
    let running = Task::named(repo_path, task)?.running_agents()?;
    let mut shown = Vec::new();
    for agent in &running {
        shown.push(instances::shown(&agent.window, &agent.terminal_id, wanted)?);
    }
    if let [only] = shown.as_slice() {
        return Ok(only.clone());
    }
    Ok(running
        .iter()
        .zip(shown)
        .map(|(agent, shown)| format!("== {} ==\n{shown}", agent.run))
        .collect())
}

/// Type a line into the shell of each of a task's running agents, and send it.
fn tell(repo_path: &Path, task: &str, line: &str) -> Result<String> {
    let line = wire::one_line(line)?;
    let running = Task::named(repo_path, task)?.running_agents()?;
    let mut told = String::new();
    for agent in running {
        instances::tell(&agent.window, &agent.terminal_id, line)
            .with_context(|| format!("{} was not told the line", agent.run))?;
        told.push_str(&format!("told {}\n", agent.run));
    }
    Ok(told)
}

/// `moon agent --help`.
pub(crate) fn help_text() -> String {
    let agents = agent_names().join("|");
    format!(
        "{PROGRAM} agent

The agents of this repo's board, listed, and one started, looked at and told something from
here.

Usage:
  {PROGRAM} agent list
  {PROGRAM} agent start <task> <{agents}>
  {PROGRAM} agent view <task> [{LINES_OPTION} <n>]
  {PROGRAM} agent tell <task> <one line>
  {PROGRAM} agent --help

Examples:
  {PROGRAM} agent list
  {PROGRAM} agent start fix-the-races claude
  {PROGRAM} agent tell fix-the-races \"start with the tests in src/terminal\"
  {PROGRAM} agent view fix-the-races
  {PROGRAM} agent view fix-the-races {LINES_OPTION} 80"
    )
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{
        instances::window::{AgentAsks, ShellAsks},
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
            "moonreview-agent-cli-{}-{name}-{}",
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

    /// Write an agent run on a task, in a shell of this id held by this process.
    fn run_an_agent(repo: &Path, task_id: &str, terminal_id: &str, name: &str, held_by: u32) {
        let mut metadata = store::read_task(repo, task_id).expect("expected the task");
        metadata.resources.push(TaskResource {
            id: store::new_uuid(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: Some(terminal_id.to_string()),
            terminal_owner: Some(held_by),
            agent_session_id: None,
            name: Some(name.to_string()),
            started_at_unix: 0,
        });
        store::write_task(repo, task_id, &metadata).expect("expected the run to be written");
    }

    /// A window's answers about agents, written down as they are asked for.
    #[derive(Default)]
    struct Asked {
        started: Mutex<Vec<(String, String, AgentKind)>>,
        told: Mutex<Vec<(String, String)>>,
    }

    impl AgentAsks for Asked {
        fn start(&self, repo_path: &str, task_id: &str, agent: AgentKind) -> Result<String> {
            if agent == AgentKind::Codex {
                bail!("Codex is not installed here");
            }
            self.started
                .lock()
                .unwrap()
                .push((repo_path.to_string(), task_id.to_string(), agent));
            Ok("fix the races claude - 1".to_string())
        }

        fn tell(&self, terminal_id: &str, line: &str) -> Result<()> {
            self.told
                .lock()
                .unwrap()
                .push((terminal_id.to_string(), line.to_string()));
            Ok(())
        }

        fn shown(&self, terminal_id: &str, wanted: Shown) -> Result<String> {
            Ok(format!("{wanted:?} of {terminal_id}\n"))
        }
    }

    /// A window open on this repo, answering about agents, and what it is asked.
    fn window_on(repo: &Path) -> (ShellAsks, Arc<Asked>) {
        let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
            .expect("expected a socket");
        asks.on_project(&repo.display().to_string())
            .expect("expected the record to be written");
        let asked = Arc::new(Asked::default());
        asks.agents_answered_by(Arc::clone(&asked) as Arc<dyn AgentAsks>);
        (asks, asked)
    }

    /// The whole of a start, over the real socket: the window open on the board's repo is
    /// asked for the agent on the task the handle names, and what it called the run is said.
    #[test]
    fn an_agent_is_started_by_the_window_open_on_the_boards_repo() {
        let (repo, races, _) = board("start");
        let (_asks, asked) = window_on(&repo);

        let said = start(&repo, "fix-the-races", AgentKind::Claude).expect("expected a start");

        assert_eq!(said, "started fix the races claude - 1 on @fix-the-races\n");
        assert_eq!(
            *asked.started.lock().unwrap(),
            [(repo.display().to_string(), races, AgentKind::Claude)]
        );
    }

    #[test]
    fn a_start_with_no_window_on_the_repo_is_refused_with_how_to_open_one() {
        let (repo, _, _) = board("no-window");
        let (elsewhere, _, _) = board("elsewhere");
        // A window on another project is not one to start this board's agents in.
        let (_asks, asked) = window_on(&elsewhere);

        let error =
            start(&repo, "fix-the-races", AgentKind::Claude).expect_err("expected a refusal");

        assert!(
            error
                .to_string()
                .contains(&format!("no moon window is open on {}", repo.display()))
                && error.to_string().contains("`moon tasks`"),
            "{error}"
        );
        assert!(asked.started.lock().unwrap().is_empty());
    }

    /// What the window could not do is what the command says, in the window's words.
    #[test]
    fn a_start_the_window_refuses_says_why() {
        let (repo, _, _) = board("refused");
        let (_asks, _asked) = window_on(&repo);

        let error =
            start(&repo, "fix-the-races", AgentKind::Codex).expect_err("expected a refusal");

        assert!(
            error.to_string().contains("Codex is not installed here"),
            "{error}"
        );
    }

    #[test]
    fn a_handle_that_names_no_task_is_refused_with_the_board_it_was_looked_for_on() {
        let (repo, _, _) = board("unknown");

        let error = start(&repo, "nobody", AgentKind::Claude).expect_err("expected a refusal");

        assert_eq!(
            error.to_string(),
            format!(
                "@nobody is no task of this board, {}",
                store::tasks_root(&repo).display()
            )
        );
    }

    /// The whole of a tell, over the real socket: the line reaches the window holding the
    /// shell of each of the task's running agents, as it was written.
    #[test]
    fn a_line_is_told_to_each_running_agent_of_the_task_as_it_was_written() {
        let (repo, races, bing_bong) = board("tell");
        let (_asks, asked) = window_on(&repo);
        let this_process = std::process::id();
        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            this_process,
        );
        run_an_agent(
            &repo,
            &races,
            "terminal-b",
            "fix the races codex - 2",
            this_process,
        );
        // A run whose moon has gone is not running, and neither is another task's told.
        run_an_agent(
            &repo,
            &races,
            "terminal-c",
            "fix the races pi - 3",
            PID_OF_NOTHING,
        );
        run_an_agent(
            &repo,
            &bing_bong,
            "terminal-d",
            "bing bong claude - 4",
            this_process,
        );

        let said = tell(&repo, "fix-the-races", "start with the tests").expect("expected a tell");

        assert_eq!(
            said,
            "told fix the races claude - 1\ntold fix the races codex - 2\n"
        );
        assert_eq!(
            *asked.told.lock().unwrap(),
            [
                ("terminal-a".to_string(), "start with the tests".to_string()),
                ("terminal-b".to_string(), "start with the tests".to_string()),
            ]
        );
    }

    /// Every running agent of the board, under the handle `view` and `tell` take - whoever
    /// holds its shell - and none of the runs that are over.
    #[test]
    fn the_list_is_every_running_agent_of_the_board_by_its_tasks_handle() {
        let (repo, races, bing_bong) = board("list");
        let this_process = std::process::id();

        assert_eq!(
            list(&repo).expect("expected a list"),
            format!(
                "no agent is running on the board in {}\n",
                store::tasks_root(&repo).display()
            )
        );

        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            this_process,
        );
        // A run whose moon has gone is not running.
        run_an_agent(
            &repo,
            &races,
            "terminal-b",
            "fix the races pi - 2",
            PID_OF_NOTHING,
        );
        run_an_agent(
            &repo,
            &races,
            "terminal-c",
            "fix the races codex - 3",
            this_process,
        );
        run_an_agent(
            &repo,
            &bing_bong,
            "terminal-d",
            "bing bong claude - 4",
            this_process,
        );

        let listed = list(&repo).expect("expected a list");

        let mut lines: Vec<&str> = listed.lines().collect();
        lines.sort();
        assert_eq!(
            lines,
            [
                "@bing-bong      bing bong claude - 4",
                "@fix-the-races  fix the races claude - 1",
                "@fix-the-races  fix the races codex - 3",
            ],
            "got {listed}"
        );
    }

    #[test]
    fn a_task_with_no_running_agent_is_refused_with_how_to_start_one() {
        let (repo, races, _) = board("nobody-running");
        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            PID_OF_NOTHING,
        );

        for error in [
            tell(&repo, "fix-the-races", "hello").expect_err("expected a refusal"),
            view(&repo, "fix-the-races", Shown::Screen).expect_err("expected a refusal"),
        ] {
            assert_eq!(
                error.to_string(),
                "@fix-the-races has no running agent: start one with `moon agent start \
                 fix-the-races <agent>`"
            );
        }
    }

    /// An agent whose shell is held by a moon with no window - this test process, which
    /// listens on nothing - is one nobody can be asked about.
    #[test]
    fn an_agent_held_by_a_moon_with_no_window_is_refused_with_the_reason() {
        let (repo, races, _) = board("serve");
        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            std::process::id(),
        );

        let error = tell(&repo, "fix-the-races", "hello").expect_err("expected a refusal");

        assert!(
            error.to_string().contains("runs in a moon with no window")
                && error
                    .to_string()
                    .contains(&format!("process {}", std::process::id())),
            "{error}"
        );
    }

    #[test]
    fn a_line_break_in_the_line_is_refused_before_anybody_is_told() {
        let (repo, races, _) = board("line-break");
        let (_asks, asked) = window_on(&repo);
        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            std::process::id(),
        );

        let error = tell(&repo, "fix-the-races", "one\ntwo").expect_err("expected a refusal");

        assert!(error.to_string().contains("line break"), "{error}");
        assert!(asked.told.lock().unwrap().is_empty());
    }

    /// One agent's screen is printed as it is, so it can be piped; several are told apart by
    /// the names of their runs.
    #[test]
    fn a_view_is_what_the_window_says_each_running_agent_is_showing() {
        let (repo, races, bing_bong) = board("view");
        let (_asks, _asked) = window_on(&repo);
        let this_process = std::process::id();
        run_an_agent(
            &repo,
            &races,
            "terminal-a",
            "fix the races claude - 1",
            this_process,
        );
        run_an_agent(
            &repo,
            &bing_bong,
            "terminal-b",
            "bing bong claude - 2",
            this_process,
        );
        run_an_agent(
            &repo,
            &bing_bong,
            "terminal-c",
            "bing bong codex - 3",
            this_process,
        );

        assert_eq!(
            view(&repo, "fix-the-races", Shown::LastRows(40)).expect("expected a view"),
            "LastRows(40) of terminal-a\n"
        );
        assert_eq!(
            view(&repo, "bing-bong", Shown::Screen).expect("expected a view"),
            "== bing bong claude - 2 ==\nScreen of terminal-b\n\
             == bing bong codex - 3 ==\nScreen of terminal-c\n"
        );
    }

    /// The three commands against a real agent, with nothing standing in for anything: a
    /// window's socket, its moon's shells, and a Claude that is started, told something and
    /// read back.
    ///
    /// Ignored because it starts a real Claude - run it with `--ignored` on a machine that
    /// has one. Everything about it that can be checked without one is in the tests above
    /// and in `terminal::told` and `terminal::shown`.
    ///
    /// The board is in a repo of its own under this checkout's `target`, which Claude has
    /// not been run in the first time this is: it comes up asking whether it trusts the
    /// folder, and the start answers that. Claude writes the answer down, so only the first
    /// run on a machine goes that way.
    #[test]
    #[ignore]
    fn a_real_agent_is_started_told_and_viewed() {
        use std::time::{Duration, Instant};

        use crate::{
            api::OpenSessionRequest,
            backend::{Backend, local::LocalBackend},
            moontasks::CreateTaskRequest,
            native::agent_asks::WindowAgents,
        };

        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("agent-from-the-command-line");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).expect("failed to create the fixture directory");
        let repo = repo.canonicalize().expect("expected the repo to resolve");
        crate::git::run_git_no_output(&repo, &["init"]).expect("failed to init the fixture repo");

        let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
        let backend: Arc<dyn Backend> = Arc::new(LocalBackend::new(state));
        let opened = backend
            .open_session(OpenSessionRequest {
                repo_path: repo.display().to_string(),
                diff_target: None,
                active_commit: None,
            })
            .expect("expected the session to open");
        let task = backend
            .create_task(
                &opened.session_id,
                &CreateTaskRequest {
                    title: "Sums".to_string(),
                    status: ColumnId::new("todo"),
                    joins: ColumnEnd::Top,
                },
            )
            .expect("expected the task to be created");
        let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
            .expect("expected a socket");
        asks.on_project(&repo.display().to_string())
            .expect("expected the record to be written");
        asks.agents_answered_by(Arc::new(WindowAgents {
            backend: Arc::clone(&backend),
        }));

        let started = start(&repo, "sums", AgentKind::Claude).expect("expected a start");
        assert_eq!(started, "started Sums claude - 1 on @sums\n");
        let screen_showing = |wanted: &[&str]| {
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                let screen = view(&repo, "sums", Shown::Screen).expect("expected the screen");
                if wanted.iter().any(|wanted| screen.contains(wanted)) {
                    return screen;
                }
                assert!(
                    Instant::now() < deadline,
                    "never showed {wanted:?}:\n{screen}"
                );
                std::thread::sleep(Duration::from_millis(500));
            }
        };
        // Told the moment it was started, which is before it has a box to be told in.
        let told = tell(
            &repo,
            "sums",
            "What is 123 plus 456? Answer with the number and nothing else.",
        )
        .expect("expected a tell");
        assert_eq!(told, "told Sums claude - 1\n");

        let screen = screen_showing(&["579"]);
        let last_rows = view(&repo, "sums", Shown::LastRows(200)).expect("expected the rows");
        let run = store::read_task(&repo, &task.id)
            .expect("expected the task")
            .resources
            .remove(0);
        backend
            .stop_task_resource(&opened.session_id, &task.id, &run.id)
            .expect("expected the agent to stop");

        println!("the screen:\n{screen}\nthe last rows:\n{last_rows}");
        assert!(last_rows.contains("What is 123 plus 456?"), "{last_rows}");
    }
}
