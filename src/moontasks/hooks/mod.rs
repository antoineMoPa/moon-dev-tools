//! Hooks: the board's own behaviour, written as scripts in the repo.
//!
//! Three things happen on a board that something might want to act on — a card lands in a
//! column, a run ends, and time passes. Each is a [`HookPoint`], and each is a file in
//! `.moontasks/hooks/`. When one fires, the file for it is run with the board's tools bound as
//! functions and the event in a variable called `event`.
//!
//! Autopilot is not a feature of the app. It is the three scripts shipped in
//! [`HOOK_POINTS`], installed once into a board that has none and then the person's: they pick
//! up a card tagged `autopilot`, give it a checkout, run a one-shot agent in it, and say on the
//! card what came of it. Changing what autopilot does is editing those files, which is the
//! point — the shipped behaviour is written in the same language as anything you would write,
//! with no privileged path into the app.
//!
//! Two things a hook cannot do, and neither is refused in the script:
//!
//! - **Edit the repo.** No tool writes a file. A hook's hands are the one-shot runs it starts,
//!   which work in the card's own checkout.
//! - **Start a review.** Reviewing puts a card's branch in the repo itself, over whatever the
//!   person has open, so it stays a button a person presses. A hook says a card is ready; it
//!   does not decide when someone looks at it.
//!
//! Nothing fires while the board is paused — see [`store::BoardConfig::hooks_running`] and the
//! play/pause control the board draws.

pub(crate) mod script;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    api::AppState,
    moontasks::{service, store},
};

/// How often the board looks at itself: fires the tick hook, and notices what has changed
/// since the last look.
///
/// A person dragging a card and a run ending are both things a hook may want to act on within
/// a moment or two, and the look itself is a directory read — so this is short. It is not a
/// poll of anything expensive; the tick hook is what decides whether there is work to do.
const LOOK: Duration = Duration::from_secs(2);

/// One moment a script can be attached to.
pub(crate) struct HookPoint {
    /// Its name, which is also its file's name in `.moontasks/hooks/`.
    pub(crate) name: &'static str,
    pub(crate) about: &'static str,
    /// What `event` holds when this one fires, for the generated reference.
    pub(crate) event: &'static str,
    /// The script installed into a board that has none.
    pub(crate) default: &'static str,
}

pub(crate) const HOOK_POINTS: &[HookPoint] = &[
    HookPoint {
        name: "tick",
        about: "Every couple of seconds, while the board is running. This is what picks work \
                up.",
        event: "Nothing — read the board with list_tasks().",
        default: include_str!("defaults/tick.rhai"),
    },
    HookPoint {
        name: "run_finished",
        about: "A one-shot run has ended, however it ended.",
        event: "#{ task_id, title, run_id, agent, commits } — `commits` is how many commits the \
                card's branch has that it did not when its checkout was made, which is the \
                plainest answer to whether the run did anything.",
        default: include_str!("defaults/run_finished.rhai"),
    },
    HookPoint {
        name: "task_entered_column",
        about: "A card has arrived in a column, whether a person dragged it there or a script \
                moved it.",
        event: "#{ task_id, title, column, from }",
        default: include_str!("defaults/task_entered_column.rhai"),
    },
];

fn point_named(name: &str) -> Option<&'static HookPoint> {
    HOOK_POINTS.iter().find(|point| point.name == name)
}

/// Write the shipped hooks into a board that has none, and the reference beside them.
///
/// A script that is already there is never written over — it is the person's from the moment
/// they first open it, the same way `brief.md` is. The reference is generated from the tool
/// table and rewritten every time, because it is a description of this build rather than
/// something anyone edits.
pub(crate) fn install(repo_path: &Path) -> Result<()> {
    let dir = store::hooks_dir(repo_path);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    for point in HOOK_POINTS {
        let path = dir.join(format!("{}.rhai", point.name));
        if path.exists() {
            continue;
        }
        std::fs::write(&path, point.default)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    let path = dir.join("REFERENCE.md");
    std::fs::write(&path, reference())
        .with_context(|| format!("failed to write {}", path.display()))
}

/// The whole script API, written out of the tool table.
pub(crate) fn reference() -> String {
    let mut text = String::from(
        "# Hooks\n\n\
         Generated — this file is rewritten on every start. Edit the `.rhai` files beside it, \
         not this one.\n\n\
         A hook is a [Rhai](https://rhai.rs) script run when something happens on the board. \
         The functions below are the whole of what a script can do; there is no file access, \
         no network, and no way to run anything but the runs you start with `run_task_agent`. \
         A script that fails says so on the card it was working, and moves nothing.\n\n\
         Nothing fires while the board is paused.\n\n         `print` puts a line in the board's log — `.moontasks/hooks.log`, and *View Autopilot          Logs* in the menu or `autopilot log` in the command palette. A hook fires with nobody          in front of it, so that log is the only account of why it did what it did; say there          what a script would have said out loud. A line the same as the one before it is not          written twice, so a tick may say why it is waiting every time it waits.\n\n         ## Hook points\n\n",
    );
    for point in HOOK_POINTS {
        text.push_str(&format!(
            "### `{}.rhai`\n\n{}\n\n`event`: {}\n\n",
            point.name, point.about, point.event
        ));
    }

    text.push_str("## Functions\n\n");
    for tool in crate::moontasks::tools::TOOLS {
        let signature: Vec<String> = tool
            .params
            .iter()
            .map(|param| match param.optional {
                true => format!("[{}]", param.name),
                false => param.name.to_string(),
            })
            .collect();
        text.push_str(&format!(
            "### `{}({})`\n\n{}\n\n",
            tool.name,
            signature.join(", "),
            tool.description
        ));
        for param in tool.params {
            text.push_str(&format!(
                "- `{}`{} — {}\n",
                param.name,
                match param.optional {
                    true => " (optional)",
                    false => "",
                },
                param.about
            ));
        }
        if !tool.params.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("Answers: {}\n\n", tool.answers));
    }
    text
}

/// Run the script attached to one hook point. A point with no file is not an error — it is a
/// board that does not want anything to happen there.
pub(crate) fn fire(
    state: &AppState,
    session_id: &str,
    point: &HookPoint,
    event: Value,
) -> Result<()> {
    let repo_path = service::repo_of(state, session_id)?;
    let path = store::hooks_dir(&repo_path).join(format!("{}.rhai", point.name));
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    script::run(state, session_id, &source, point.name, event)
}

/// What one board looked like the last time it was looked at, which is what turns a reading of
/// the board into the events that happened since.
#[derive(Default)]
struct Seen {
    /// Which column each card was in.
    columns: HashMap<String, String>,
    /// The runs that were going, as `(task id, run id)`.
    running: HashSet<(String, String)>,
}

/// Watch every open board and fire its hooks, for as long as the process lives.
///
/// One thread for all boards rather than one each: hooks are the board acting on itself, and
/// two firings at once on the same board would race over the same cards. Being a single thread
/// also means a slow hook delays the next look rather than piling up behind it.
pub(crate) fn watch(state: AppState) {
    std::thread::Builder::new()
        .name("moontasks-hooks".to_string())
        .spawn(move || {
            let mut seen: HashMap<String, Seen> = HashMap::new();
            loop {
                std::thread::sleep(LOOK);
                look(&state, &mut seen);
            }
        })
        .expect("failed to start the hooks thread");
}

/// One board per repo, whichever session found it first: two windows open on the same repo are
/// two views of one board, and its hooks belong to the board.
fn boards_of(state: &AppState) -> Vec<(String, String)> {
    let Ok(guard) = state.inner.lock() else {
        return Vec::new();
    };
    let mut boards: HashMap<String, String> = HashMap::new();
    for (session_id, session) in guard.sessions.iter() {
        boards
            .entry(session.repo_path.display().to_string())
            .or_insert_with(|| session_id.clone());
    }
    boards.into_iter().collect()
}

fn look(state: &AppState, seen: &mut HashMap<String, Seen>) {
    for (repo_path, session_id) in boards_of(state) {
        let Ok(tasks) = service::list_tasks(state, &session_id) else {
            continue;
        };

        let mut now = Seen::default();
        for task in &tasks {
            now.columns
                .insert(task.id.clone(), task.status.as_str().to_string());
            for run in &task.resources {
                if run.running {
                    now.running.insert((task.id.clone(), run.id.clone()));
                }
            }
        }

        // A board seen for the first time is only recorded, and given the shipped hooks if it
        // has none. Everything already on it is older than this process, so firing for all of
        // it would be reporting history as news.
        let Some(before) = seen.get(&repo_path) else {
            if let Err(error) = install(Path::new(&repo_path)) {
                eprintln!("[moontasks] {error}");
            }
            seen.insert(repo_path, now);
            continue;
        };
        // A paused board is still followed, so that starting it again acts on what happens
        // next rather than on everything that happened while it was stopped.
        let running = store::read_board(Path::new(&repo_path)).hooks_running;
        if running {
            fire_what_changed(state, &session_id, &tasks, before, &now);
        }
        seen.insert(repo_path, now);
    }
}

fn fire_what_changed(
    state: &AppState,
    session_id: &str,
    tasks: &[crate::moontasks::TaskView],
    before: &Seen,
    now: &Seen,
) {
    // Runs first, then arrivals, then the tick: each is told about a board the one before it
    // has finished acting on.
    for (task_id, run_id) in before.running.difference(&now.running) {
        let Some(task) = tasks.iter().find(|task| task.id == *task_id) else {
            continue;
        };
        // The same spelling `run_task_agent` takes, so a hook can start another run of the
        // same agent with what it was handed.
        let agent = task
            .resources
            .iter()
            .find(|run| run.id == *run_id)
            .map_or(Value::Null, |run| json!(run.agent));
        let event = json!({
            "task_id": task_id,
            "title": task.title,
            "run_id": run_id,
            "agent": agent,
            "commits": commits_of(state, session_id, task_id).unwrap_or_default(),
        });
        script::log_line(
            state,
            session_id,
            &format!("run ended on \"{}\" — firing run_finished", task.title),
        );
        report(state, session_id, "run_finished", Some(task_id), event);
    }

    for (task_id, column) in &now.columns {
        let Some(was) = before.columns.get(task_id) else {
            continue;
        };
        if was == column {
            continue;
        }
        let Some(task) = tasks.iter().find(|task| task.id == *task_id) else {
            continue;
        };
        let event = json!({
            "task_id": task_id,
            "title": task.title,
            "column": column,
            "from": was,
        });
        report(
            state,
            session_id,
            "task_entered_column",
            Some(task_id),
            event,
        );
    }

    report(state, session_id, "tick", None, Value::Null);
}

/// How many commits a card's branch has that it did not when its checkout was made.
fn commits_of(state: &AppState, session_id: &str, task_id: &str) -> Result<usize> {
    let repo_path = service::repo_of(state, session_id)?;
    let metadata = store::read_task(&repo_path, task_id)?;
    let Some(worktree) = metadata.worktree else {
        return Ok(0);
    };
    let Some(base) = worktree.base_commit else {
        return Ok(0);
    };
    crate::git::commits_ahead(&repo_path, &base, &worktree.branch)
}

/// Fire one hook, and put whatever went wrong where the person will see it.
///
/// A hook that fails writes on the card it was working, because that is where someone is
/// already looking when they wonder why nothing happened to it — and in the board's log
/// either way, which is the one place a tick's failure can be read at all.
fn report(state: &AppState, session_id: &str, point: &str, task_id: Option<&str>, event: Value) {
    let Some(point) = point_named(point) else {
        return;
    };
    let Err(error) = fire(state, session_id, point, event) else {
        return;
    };

    eprintln!("[moontasks] {error}");
    script::log_line(state, session_id, &format!("failed — {error}"));
    let Some(task_id) = task_id else {
        return;
    };
    let Ok(repo_path) = service::repo_of(state, session_id) else {
        return;
    };
    let mut notes = store::read_notes(&repo_path, task_id);
    if !notes.is_empty() {
        notes.push_str("\n\n");
    }
    notes.push_str(&format!("[hooks] {error}\n"));
    let _ = store::write_notes(&repo_path, task_id, &notes);
}
