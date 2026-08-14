//! What a script may do to the board, and what it may not.
//!
//! One table. A tool is a name, a sentence saying when to reach for it, the arguments it takes,
//! and a function — and that function is a thin call into [`super::service`] or
//! [`super::worktrees`], the same one the board's own buttons make. Nothing here reimplements
//! anything, which is what stops a tool and a button drifting apart.
//!
//! This table is the single definition of the board's API. The Rhai bindings a hook script is
//! given are registered from it ([`super::hooks::script`]), and the reference written into the
//! hooks folder is generated from it, so a tool cannot exist without being callable and
//! documented.
//!
//! Four things are missing from the table on purpose. There is no tool that creates a task,
//! none that writes the `autopilot` tag, none that edits a file, and none that starts a review.
//! See [`REFUSED_TAGS`] and the module docs of [`super::hooks`].

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    api::{AgentKind, AppState},
    moontasks::{
        AUTOPILOT_TAG, ColumnName, StartResourceRequest, TaskResourceKind, service, store,
        worktrees,
    },
};

/// How much of a run's output [`read_task_run`] answers with, in bytes from the end.
///
/// The end rather than the start: a run says what it did last, and a run that failed says why
/// on its way out.
const RUN_LOG_TAIL: usize = 16_000;

/// Tags no tool may add or remove, whatever it is asked.
///
/// `autopilot` decides which cards the board picks up. A script that could write it could widen
/// its own remit, and one that could clear it could quietly drop a card off the board — so the
/// tag stays the person's, and a card's progress is said with its column and its notes instead.
const REFUSED_TAGS: &[&str] = &[AUTOPILOT_TAG];

/// Which board a call is working, and where.
pub(crate) struct Board<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) session_id: String,
}

/// What an argument holds, which is all a caller needs to be told and all a binding needs to
/// convert.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ParamKind {
    Text,
    Flag,
    Count,
}

/// One argument of one tool. Order is the order they are passed in a script.
pub(crate) struct Param {
    pub(crate) name: &'static str,
    pub(crate) kind: ParamKind,
    /// An argument that may be left off. Every optional argument comes after every required
    /// one, so a call can simply stop early — checked by a test.
    pub(crate) optional: bool,
    pub(crate) about: &'static str,
}

/// One tool: what it is called, when to reach for it, what it takes, and what it does.
pub(crate) struct Tool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) params: &'static [Param],
    /// What it answers with, for the generated reference. Free text: it is read, not parsed.
    pub(crate) answers: &'static str,
    pub(crate) call: fn(&Board<'_>, &Value) -> Result<Value>,
}

const fn text(name: &'static str, about: &'static str) -> Param {
    Param {
        name,
        kind: ParamKind::Text,
        optional: false,
        about,
    }
}

const fn optional(kind: ParamKind, name: &'static str, about: &'static str) -> Param {
    Param {
        name,
        kind,
        optional: true,
        about,
    }
}

const TASK: &[Param] = &[text("task_id", "The card, as list_tasks gives its id.")];

pub(crate) const TOOLS: &[Tool] = &[
    Tool {
        name: "list_tasks",
        description: "Every card on the board: its id, title, column, place in that column, \
                      tags, notes, the checkout it works in, and the runs it has going.",
        params: &[],
        answers: "An array of cards, in board order.",
        call: list_tasks,
    },
    Tool {
        name: "list_columns",
        description: "The board's columns, left to right, by the names their headings read. \
                      A column is named by that name everywhere, in any case.",
        params: &[],
        answers: "An array of `#{ name }`.",
        call: list_columns,
    },
    Tool {
        name: "move_task",
        description: "Put a card in a column, at a place among the cards already there.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text(
                "column",
                "A column, by the name its heading reads. Case does not matter.",
            ),
            optional(
                ParamKind::Count,
                "position",
                "Where among that column's cards. Left off, it joins the bottom.",
            ),
        ],
        answers: "Nothing.",
        call: move_task,
    },
    Tool {
        name: "add_task_tag",
        description: "Mark a card. Every tag but `autopilot`, which is the person's.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text("tag", "The tag to add."),
        ],
        answers: "The card's tags, as they now stand.",
        call: add_task_tag,
    },
    Tool {
        name: "remove_task_tag",
        description: "Take a mark off a card. Every tag but `autopilot`.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text("tag", "The tag to take off."),
        ],
        answers: "The card's tags, as they now stand.",
        call: remove_task_tag,
    },
    Tool {
        name: "read_task_notes",
        description: "A card's notes — its description, and everything written about it since. \
                      This is the brief a run is given.",
        params: TASK,
        answers: "The notes, as text.",
        call: read_task_notes,
    },
    Tool {
        name: "append_task_notes",
        description: "Write at the end of a card's notes. This is how a hook leaves a trail on \
                      a card it worked: what it did, and what happened.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text("text", "What to write. A blank line is added before it."),
        ],
        answers: "Nothing.",
        call: append_task_notes,
    },
    Tool {
        name: "create_worktree",
        description: "Give a card a checkout of its own, on a branch named after it. \
                      Everything the card starts from then on runs there rather than in the \
                      repo, which is what lets a run work without touching what the person has \
                      open.",
        params: TASK,
        answers: "`#{ path, branch }`.",
        call: create_worktree,
    },
    Tool {
        name: "discard_worktree",
        description: "Give that checkout back. The branch stays; only the working copy goes.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            optional(
                ParamKind::Flag,
                "force",
                "Take uncommitted work in the checkout with it. Refused without this.",
            ),
        ],
        answers: "Nothing.",
        call: discard_worktree,
    },
    Tool {
        name: "run_task_agent",
        description: "Start a one-shot agent on a card: it is given the whole of its work up \
                      front, runs on its own, and ends. It runs in the card's checkout if it \
                      has one. Comes back as soon as the run has started, not when it \
                      finishes — the run_finished hook is how you hear about the end.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text("agent", "One of claude, codex, opencode."),
            text("prompt", "The whole of the work. Usually the card's notes."),
        ],
        answers: "The run's id.",
        call: run_task_agent,
    },
    Tool {
        name: "read_task_run",
        description: "What a run printed, from the end. This is the only account of what it \
                      did.",
        params: &[
            text("task_id", "The card, as list_tasks gives its id."),
            text(
                "run_id",
                "From run_task_agent, or from the run_finished hook.",
            ),
        ],
        answers: "What it printed, as text.",
        call: read_task_run,
    },
    Tool {
        name: "stop_task_agents",
        description: "End every run a card has going. For a run that has gone wrong or is no \
                      longer wanted.",
        params: TASK,
        answers: "How many were stopped.",
        call: stop_task_agents,
    },
    Tool {
        name: "repo_status",
        description: "The repo the board belongs to: which branch it is on, whether it has \
                      uncommitted changes, and what its default branch is.",
        params: &[],
        answers: "`#{ repo_path, branch, is_clean, default_branch }`.",
        call: repo_status,
    },
];

pub(crate) fn tool_named(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// How many of a tool's arguments a call must give.
pub(crate) fn required_count(tool: &Tool) -> usize {
    tool.params.iter().filter(|param| !param.optional).count()
}

pub(crate) fn call(board: &Board<'_>, name: &str, arguments: &Value) -> Result<Value> {
    let Some(tool) = tool_named(name) else {
        bail!("{name} is not a tool of this board");
    };
    (tool.call)(board, arguments)
}

/// One required argument, refused by name rather than defaulted: a tool called without what it
/// needs is a mistake worth reporting, not one worth guessing about.
fn text_of(arguments: &Value, name: &str) -> Result<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("{name} is required"))
}

fn task_of(arguments: &Value) -> Result<String> {
    text_of(arguments, "task_id")
}

fn nothing() -> Result<Value> {
    Ok(Value::Null)
}

fn list_tasks(board: &Board<'_>, _arguments: &Value) -> Result<Value> {
    let tasks = service::list_tasks(board.state, &board.session_id)?;
    Ok(serde_json::to_value(tasks)?)
}

fn list_columns(board: &Board<'_>, _arguments: &Value) -> Result<Value> {
    let columns = service::list_columns(board.state, &board.session_id)?;
    Ok(serde_json::to_value(columns)?)
}

fn move_task(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let column = ColumnName::new(text_of(arguments, "column")?);
    // No position means the bottom, which is where work joins a queue. `usize::MAX` is how
    // `place_task` is already told "past the end".
    let position = arguments
        .get("position")
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |position| position as usize);

    service::place_task(board.state, &board.session_id, &task_id, column, position)?;
    nothing()
}

/// The tags a card should end up with, having added or removed one — or a refusal.
fn tags_after(board: &Board<'_>, task_id: &str, tag: &str, holding: bool) -> Result<Vec<String>> {
    let Some(tag) = store::tag_of(tag) else {
        bail!("that is not a tag");
    };
    if REFUSED_TAGS.contains(&tag.as_str()) {
        bail!(
            "the {tag} tag is the person's — say what happened to this card with its column or \
             its notes instead"
        );
    }

    let repo_path = service::repo_of(board.state, &board.session_id)?;
    let mut tags = store::read_task(&repo_path, task_id)?.tags;
    tags.retain(|held| *held != tag);
    if holding {
        tags.push(tag);
    }
    Ok(tags)
}

fn set_tag(board: &Board<'_>, arguments: &Value, holding: bool) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let tag = text_of(arguments, "tag")?;
    let tags = tags_after(board, &task_id, &tag, holding)?;
    service::set_tags(board.state, &board.session_id, &task_id, &tags)?;
    Ok(serde_json::to_value(tags)?)
}

fn add_task_tag(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    set_tag(board, arguments, true)
}

fn remove_task_tag(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    set_tag(board, arguments, false)
}

fn read_task_notes(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    // Reading the task first, so a task id that names nothing is said plainly rather than
    // coming back as empty notes.
    store::read_task(&repo_path, &task_id)?;
    Ok(Value::String(store::read_notes(&repo_path, &task_id)))
}

fn append_task_notes(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let text = text_of(arguments, "text")?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    store::read_task(&repo_path, &task_id)?;

    let mut notes = store::read_notes(&repo_path, &task_id);
    if !notes.is_empty() && !notes.ends_with('\n') {
        notes.push('\n');
    }
    if !notes.is_empty() {
        notes.push('\n');
    }
    notes.push_str(text.trim_end());
    notes.push('\n');
    store::write_notes(&repo_path, &task_id, &notes)?;
    nothing()
}

fn create_worktree(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let worktree = worktrees::create(board.state, &board.session_id, &task_id)?;
    Ok(json!({ "path": worktree.path, "branch": worktree.branch }))
}

fn discard_worktree(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let force = arguments
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    worktrees::discard(board.state, &board.session_id, &task_id, force)?;
    nothing()
}

fn run_task_agent(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let prompt = text_of(arguments, "prompt")?;
    let named = text_of(arguments, "agent")?;
    let agent = match named.as_str() {
        "claude" => AgentKind::Claude,
        "codex" => AgentKind::Codex,
        "opencode" => AgentKind::OpenCode,
        other => bail!("{other} is not an agent this board can run"),
    };

    let terminal_id = service::start_resource(
        board.state,
        &board.session_id,
        &task_id,
        StartResourceRequest {
            kind: TaskResourceKind::Agent,
            agent,
            prompt: Some(prompt),
        },
        store::RunStarter::Hook,
    )?;

    // The run's own id, which is what reading its output later is asked for. It is found by
    // the shell it was just given rather than by being newest, so two runs started in the same
    // moment cannot be mixed up.
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    let run_id = store::read_task(&repo_path, &task_id)?
        .resources
        .into_iter()
        .find(|resource| resource.terminal_id.as_deref() == Some(&terminal_id))
        .map(|resource| resource.id)
        .context("the run was started but is not on the card")?;

    Ok(Value::String(run_id))
}

fn read_task_run(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let run_id = text_of(arguments, "run_id")?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;

    let path = store::run_log_path(&repo_path, &task_id, &run_id);
    let Ok(printed) = std::fs::read(&path) else {
        bail!(
            "nothing was recorded for that run — a run started as a conversation rather than \
             with a prompt keeps no transcript"
        );
    };
    let printed = String::from_utf8_lossy(&printed);
    Ok(Value::String(
        match printed.char_indices().nth_back(RUN_LOG_TAIL) {
            // Cut on a character rather than a byte, and say that it was cut: an answer that
            // silently starts mid-sentence reads as the run having started there.
            Some((at, _)) => format!("[the last {RUN_LOG_TAIL} characters]\n{}", &printed[at..]),
            None => printed.into_owned(),
        },
    ))
}

fn stop_task_agents(board: &Board<'_>, arguments: &Value) -> Result<Value> {
    let task_id = task_of(arguments)?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    let running: Vec<String> = store::read_task(&repo_path, &task_id)?
        .resources
        .into_iter()
        .filter(|resource| {
            resource
                .terminal_id
                .as_ref()
                .is_some_and(|terminal_id| board.state.terminals.is_live(terminal_id))
        })
        .map(|resource| resource.id)
        .collect();

    for resource_id in &running {
        service::stop_resource(board.state, &board.session_id, &task_id, resource_id)?;
    }
    Ok(json!(running.len()))
}

fn repo_status(board: &Board<'_>, _arguments: &Value) -> Result<Value> {
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    Ok(json!({
        "repo_path": repo_path.display().to_string(),
        "branch": crate::git::current_branch_name(&repo_path)?,
        "is_clean": crate::git::is_clean(&repo_path)?,
        "default_branch": crate::git::default_branch_ref(&repo_path)?,
    }))
}
