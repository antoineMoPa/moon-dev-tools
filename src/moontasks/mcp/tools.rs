//! The tools themselves: what an agent may do to the board, and what it may not.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::Board;
use crate::{
    api::AgentKind,
    moontasks::{
        AUTOPILOT_TAG, ColumnId, StartResourceRequest, TaskResourceKind, service, store, worktrees,
    },
};

/// How much of a run's output a tool answers with, in bytes from the end.
const RUN_LOG_TAIL: usize = 16_000;

/// Tags no tool may add or remove, whatever it is asked.
const REFUSED_TAGS: &[&str] = &[AUTOPILOT_TAG];

/// One tool: what it is called, when to reach for it, what it takes, and what it does.
pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    /// The JSON Schema of its arguments.
    pub(super) schema: fn() -> Value,
    pub(super) call: fn(&Board<'_>, &Value) -> Result<String>,
}

pub(super) const TOOLS: &[Tool] = &[
    Tool {
        name: "list_tasks",
        description: "Every card on the board: its id, title, column, place in that column, \
                      tags, notes, the checkout it works in, and the runs it has going. Read \
                      this before doing anything else, and read it again to see whether a run \
                      you started has finished.",
        schema: no_arguments,
        call: list_tasks,
    },
    Tool {
        name: "list_columns",
        description: "The board's columns, left to right, as ids and labels. Card moves name \
                      a column by its id.",
        schema: no_arguments,
        call: list_columns,
    },
    Tool {
        name: "move_task",
        description: "Move a card to a column, and to a place among the cards already there. \
                      Position 0 is the top; leaving it out puts the card at the bottom.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "column": { "type": "string", "description": "A column id from list_columns." },
                    "position": { "type": "integer", "minimum": 0 },
                },
                "required": ["task_id", "column"],
            })
        },
        call: move_task,
    },
    Tool {
        name: "add_task_tag",
        description: "Mark a card with a tag. The `autopilot` tag is the person's and is \
                      refused here: say what happened to a card with its column and its notes.",
        schema: tag_arguments,
        call: add_task_tag,
    },
    Tool {
        name: "remove_task_tag",
        description: "Take a tag off a card. The `autopilot` tag is refused, including on a \
                      card you have finished with.",
        schema: tag_arguments,
        call: remove_task_tag,
    },
    Tool {
        name: "read_task_notes",
        description: "The whole of a card's notes.md — the task's description and whatever has \
                      been written about it since. This is where the work is actually \
                      described, so read it before running anything.",
        schema: task_argument,
        call: read_task_notes,
    },
    Tool {
        name: "append_task_notes",
        description: "Add to the end of a card's notes.md. Use it to leave an account of what \
                      was done, what a run reported, or why a card was left alone.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "text": { "type": "string" },
                },
                "required": ["task_id", "text"],
            })
        },
        call: append_task_notes,
    },
    Tool {
        name: "create_worktree",
        description: "Give a card a checkout of its own, on a branch named after it, and \
                      answer with where it is. Everything the card runs from then on happens \
                      there rather than in the repo, which is what keeps a run clear of the \
                      person's own working tree. Do this before starting work on a card.",
        schema: task_argument,
        call: create_worktree,
    },
    Tool {
        name: "discard_worktree",
        description: "Remove a card's checkout. Its branch stays. Refused while there is \
                      uncommitted work in it unless force is true.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "force": { "type": "boolean" },
                },
                "required": ["task_id"],
            })
        },
        call: discard_worktree,
    },
    Tool {
        name: "run_task_agent",
        description: "Start an agent in a card with the whole of its work, and answer at once \
                      with the run's id — it does NOT wait, and the run may take many minutes. \
                      This is how work gets done: you decide what should happen and this does \
                      it. Call await_task_run next, and keep calling it until it says the run \
                      has finished — that is how you wait, and you must not stop while a run of \
                      yours is still going.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "agent": {
                        "type": "string",
                        "enum": ["claude", "codex", "opencode"],
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The whole of the work. The run cannot ask you anything, \
                                        so say what done looks like and what to do about what \
                                        it finds.",
                    },
                },
                "required": ["task_id", "agent", "prompt"],
            })
        },
        call: run_task_agent,
    },
    Tool {
        name: "await_task_run",
        description: "Wait for a run to finish, and answer with what it printed. Comes back \
                      early if the run is still going, saying so — call it again until it is \
                      done. This is how you wait: it holds the line for you, so you do not \
                      have to sleep and cannot lose track of a run by stopping while it works.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "run_id": { "type": "string", "description": "From run_task_agent." },
                },
                "required": ["task_id", "run_id"],
            })
        },
        call: await_task_run,
    },
    Tool {
        name: "read_task_run",
        description: "What a run printed, from the end. This is the only account of what it \
                      did, so read it before deciding a card is finished.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "run_id": { "type": "string", "description": "From run_task_agent, or list_tasks." },
                },
                "required": ["task_id", "run_id"],
            })
        },
        call: read_task_run,
    },
    Tool {
        name: "stop_task_agents",
        description: "End every run a card has going. For a run that has gone wrong or is no \
                      longer wanted.",
        schema: task_argument,
        call: stop_task_agents,
    },
    Tool {
        name: "repo_status",
        description: "The repo the board belongs to: which branch it is on, whether it has \
                      uncommitted changes, and what its default branch is. Reviewing a card \
                      needs the repo to be clean, so check here before asking for one.",
        schema: no_arguments,
        call: repo_status,
    },
    Tool {
        name: "review",
        description: "Put a card's work where the person can look at it: its branch is checked \
                      out in the repo itself and a review opens on it. Refused if the card's \
                      checkout still has uncommitted work — the run has to commit on its \
                      branch first — or if the repo has local changes, which must never be \
                      moved out from under. Read the refusal and act on it rather than \
                      retrying.",
        schema: task_argument,
        call: review,
    },
];

/// Every tool, in the shape `tools/list` answers with.
pub(super) fn definitions() -> Value {
    Value::Array(
        TOOLS
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": (tool.schema)(),
                })
            })
            .collect(),
    )
}

pub(super) fn call(board: &Board<'_>, name: &str, arguments: &Value) -> Result<String> {
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        bail!("{name} is not a tool of this server");
    };
    (tool.call)(board, arguments)
}

fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn task_argument() -> Value {
    json!({
        "type": "object",
        "properties": { "task_id": { "type": "string" } },
        "required": ["task_id"],
    })
}

fn tag_arguments() -> Value {
    json!({
        "type": "object",
        "properties": {
            "task_id": { "type": "string" },
            "tag": { "type": "string" },
        },
        "required": ["task_id", "tag"],
    })
}

/// One required string argument, refused by name rather than defaulted: a tool called without
/// what it needs is a mistake worth reporting, not one worth guessing about.
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

fn list_tasks(board: &Board<'_>, _arguments: &Value) -> Result<String> {
    let tasks = service::list_tasks(board.state, &board.session_id)?;
    Ok(serde_json::to_string_pretty(&tasks)?)
}

fn list_columns(board: &Board<'_>, _arguments: &Value) -> Result<String> {
    let columns = service::list_columns(board.state, &board.session_id)?;
    Ok(serde_json::to_string_pretty(&columns)?)
}

fn move_task(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let column = ColumnId::new(text_of(arguments, "column")?);
    // No position means the bottom, which is where work joins a queue. `usize::MAX` is how
    // `place_task` is already told "past the end".
    let position = arguments
        .get("position")
        .and_then(Value::as_u64)
        .map_or(usize::MAX, |position| position as usize);

    service::place_task(
        board.state,
        &board.session_id,
        &task_id,
        column.clone(),
        position,
    )?;
    Ok(format!("{task_id} is now in {column}"))
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

fn add_task_tag(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let tag = text_of(arguments, "tag")?;
    let tags = tags_after(board, &task_id, &tag, true)?;
    service::set_tags(board.state, &board.session_id, &task_id, &tags)?;
    Ok(format!("{task_id} is marked {}", tags.join(", ")))
}

fn remove_task_tag(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let tag = text_of(arguments, "tag")?;
    let tags = tags_after(board, &task_id, &tag, false)?;
    service::set_tags(board.state, &board.session_id, &task_id, &tags)?;
    Ok(match tags.is_empty() {
        true => format!("{task_id} is marked with nothing"),
        false => format!("{task_id} is marked {}", tags.join(", ")),
    })
}

fn read_task_notes(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    // Reading the task first, so a task id that names nothing is said plainly rather than
    // coming back as empty notes.
    store::read_task(&repo_path, &task_id)?;
    Ok(match store::read_notes(&repo_path, &task_id) {
        notes if notes.trim().is_empty() => "this card has no notes yet".to_string(),
        notes => notes,
    })
}

fn append_task_notes(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let text = text_of(arguments, "text")?;
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    store::read_task(&repo_path, &task_id)?;

    let mut notes = store::read_notes(&repo_path, &task_id);
    if !notes.is_empty() && !notes.ends_with('\n') {
        notes.push('\n');
    }
    notes.push_str(text.trim_end());
    notes.push('\n');
    store::write_notes(&repo_path, &task_id, &notes)?;
    Ok(format!("written to the notes of {task_id}"))
}

fn create_worktree(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let worktree = worktrees::create(board.state, &board.session_id, &task_id)?;
    Ok(format!(
        "{task_id} now works in {} on branch {}",
        worktree.path, worktree.branch
    ))
}

fn discard_worktree(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let force = arguments
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    worktrees::discard(board.state, &board.session_id, &task_id, force)?;
    Ok(format!(
        "the checkout of {task_id} is gone; its branch stays"
    ))
}

fn run_task_agent(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let prompt = text_of(arguments, "prompt")?;
    let agent = match text_of(arguments, "agent")?.as_str() {
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

    Ok(format!(
        "started; run_id {run_id}. It is working now — read list_tasks until it stops running, \
         then read_task_run to see what it said."
    ))
}

/// How long one `await_task_run` holds the line before answering "still going".
const AWAIT_RUN_LIMIT: std::time::Duration = std::time::Duration::from_secs(50);
const AWAIT_RUN_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether that run still has a shell of its own that the server believes in.
fn run_is_going(board: &Board<'_>, task_id: &str, run_id: &str) -> Result<bool> {
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    let Some(resource) = store::read_task(&repo_path, task_id)?
        .resources
        .into_iter()
        .find(|resource| resource.id == run_id)
    else {
        bail!("{run_id} is not a run of that card");
    };
    Ok(resource
        .terminal_id
        .is_some_and(|terminal_id| board.state.terminals.is_live(&terminal_id)))
}

/// Hold the line until the run stops, or until it is time to say it has not.
fn await_task_run(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let run_id = text_of(arguments, "run_id")?;

    let until = std::time::Instant::now() + AWAIT_RUN_LIMIT;
    while std::time::Instant::now() < until {
        if !run_is_going(board, &task_id, &run_id)? {
            return read_task_run(board, arguments)
                .map(|printed| format!("the run has finished. What it printed:\n\n{printed}"));
        }
        std::thread::sleep(AWAIT_RUN_POLL);
    }
    Ok("still going — call await_task_run again".to_string())
}

fn read_task_run(board: &Board<'_>, arguments: &Value) -> Result<String> {
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
    Ok(match printed.char_indices().nth_back(RUN_LOG_TAIL) {
        // Cut on a character rather than a byte, and say that it was cut: an answer that
        // silently starts mid-sentence reads as the run having started there.
        Some((at, _)) => format!("[the last {RUN_LOG_TAIL} characters]\n{}", &printed[at..]),
        None => printed.into_owned(),
    })
}

fn stop_task_agents(board: &Board<'_>, arguments: &Value) -> Result<String> {
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
    Ok(match running.len() {
        0 => format!("{task_id} had nothing running"),
        stopped => format!("stopped {stopped} on {task_id}"),
    })
}

fn repo_status(board: &Board<'_>, _arguments: &Value) -> Result<String> {
    let repo_path = service::repo_of(board.state, &board.session_id)?;
    Ok(serde_json::to_string_pretty(&json!({
        "repo_path": repo_path.display().to_string(),
        "branch": crate::git::current_branch_name(&repo_path)?,
        "is_clean": crate::git::is_clean(&repo_path)?,
        "default_branch": crate::git::default_branch_ref(&repo_path)?,
    }))?)
}

fn review(board: &Board<'_>, arguments: &Value) -> Result<String> {
    let task_id = task_of(arguments)?;
    let review = worktrees::review(board.state, &board.session_id, &task_id)?;
    let opened = review.repo_path.clone();
    // The pane belongs to the window, so what is left here is the asking. The board's next
    // poll picks it up.
    crate::api::ask_the_window(
        board.state,
        &board.session_id,
        crate::api::WindowRequest::Review(review),
    )?;
    Ok(format!(
        "the branch is checked out in {opened} and a review is open on it. The person QAs and \
         commits from here — leave the card alone until they say otherwise."
    ))
}
