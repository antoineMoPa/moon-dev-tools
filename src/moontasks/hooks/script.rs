//! The Rhai engine a hook runs in: the board's tools, and nothing else.
//!
//! Every binding is registered from [`tools::TOOLS`], in a loop. There is no list of function
//! names here and no per-tool glue, so a tool cannot be added to the board without being
//! callable from a script, and one cannot be taken away and left callable.
//!
//! A hook fires unattended, on a worker thread, so a script that never finishes would be a
//! board that never ticks again. Two limits stop that: a cap on how much work one firing may
//! do, and a wall-clock deadline. Both end the run with an error, which is reported like any
//! other failing hook.

use std::{
    any::TypeId,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use rhai::{Dynamic, Engine, EvalAltResult, NativeCallContext, Position, Scope};
use serde_json::{Map, Value};

use crate::{
    api::AppState,
    moontasks::tools::{self, Board, ParamKind, Tool},
};

/// How much work one firing of one hook may do.
///
/// Rhai counts operations rather than time, so this is the same limit on a fast machine and a
/// slow one. High enough to walk a board of any size several times over; low enough that a
/// runaway loop is caught in well under a second.
const MAX_OPERATIONS: u64 = 2_000_000;

/// How long one firing may take in the real world.
///
/// Separate from the operation cap because the expensive things a hook does are not
/// operations: a tool call shells out to git or starts a process, and a script that made a
/// thousand of them would sit inside the cap while holding the worker for minutes.
const DEADLINE: Duration = Duration::from_secs(30);

/// Run one script with the board's tools bound, and `event` in scope.
pub(crate) fn run(
    state: &AppState,
    session_id: &str,
    source: &str,
    name: &str,
    event: Value,
) -> Result<()> {
    let engine = engine_for(state.clone(), session_id.to_string(), name.to_string());
    let mut scope = Scope::new();
    scope.push_dynamic("event", to_dynamic(&event)?);

    engine
        .run_with_scope(&mut scope, source)
        .map_err(|error| anyhow!("{name}: {error}"))
}

fn engine_for(state: AppState, session_id: String, name: String) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);

    // `print` is where a script says why it did what it did, and the board's log is where
    // someone reads that back — a hook fires unattended, so a line on stdout is a line nobody
    // is standing in front of. `debug` goes the same way rather than nowhere.
    let printing = (state.clone(), session_id.clone(), name.clone());
    engine.on_print(move |said| {
        let (state, session_id, name) = &printing;
        log_line(state, session_id, &format!("{name}: {said}"));
    });
    let debugging = (state.clone(), session_id.clone(), name);
    engine.on_debug(move |said, _source, position| {
        let (state, session_id, name) = &debugging;
        log_line(state, session_id, &format!("{name} ({position}): {said}"));
    });
    // A script is read from a file in the repo, so there is no call for building more script
    // out of strings at runtime — and every reason not to.
    engine.disable_symbol("eval");

    let until = Instant::now() + DEADLINE;
    engine.on_progress(move |_| {
        (Instant::now() > until).then(|| Dynamic::from("this hook took too long and was stopped"))
    });

    for tool in tools::TOOLS {
        register(&mut engine, tool, &state, &session_id);
    }
    engine
}

/// Bind one tool under its own name, at every number of arguments it accepts.
///
/// Optional arguments come last and may simply be left off, so one tool is several Rhai
/// functions of the same name — `discard_worktree(id)` and `discard_worktree(id, true)` are
/// both calls of the tool.
fn register(engine: &mut Engine, tool: &'static Tool, state: &AppState, session_id: &str) {
    for arity in tools::required_count(tool)..=tool.params.len() {
        let state = state.clone();
        let session_id = session_id.to_string();
        engine.register_raw_fn(
            tool.name,
            // `Dynamic` in a signature means "any type", so the arguments arrive as written and
            // are checked against what the tool asked for rather than by Rhai's overloading.
            &vec![TypeId::of::<Dynamic>(); arity],
            move |_context: NativeCallContext, args: &mut [&mut Dynamic]| {
                let board = Board {
                    state: &state,
                    session_id: session_id.clone(),
                };
                let arguments = arguments_of(tool, args).map_err(runtime_error)?;
                // By name rather than through the function pointer in hand, so that every call
                // of every tool — from a script or from anywhere else — goes the one way.
                let answer = tools::call(&board, tool.name, &Value::Object(arguments))
                    .map_err(runtime_error)?;
                to_dynamic(&answer).map_err(runtime_error)
            },
        );
    }
}

/// Name the arguments a script passed by position, and check each one holds what the tool's
/// table says it holds.
fn arguments_of(tool: &Tool, args: &[&mut Dynamic]) -> Result<Map<String, Value>> {
    let mut arguments = Map::new();
    for (param, argument) in tool.params.iter().zip(args) {
        let given =
            match param.kind {
                ParamKind::Text => {
                    Value::String((*argument).clone().into_string().map_err(|held| {
                        anyhow!("{} takes text, and was given {held}", param.name)
                    })?)
                }
                ParamKind::Flag => Value::Bool(argument.as_bool().map_err(|held| {
                    anyhow!("{} takes true or false, and was given {held}", param.name)
                })?),
                ParamKind::Count => Value::from(argument.as_int().map_err(|held| {
                    anyhow!("{} takes a number, and was given {held}", param.name)
                })?),
            };
        arguments.insert(param.name.to_string(), given);
    }
    Ok(arguments)
}

fn to_dynamic(value: &Value) -> Result<Dynamic> {
    rhai::serde::to_dynamic(value).map_err(|error| anyhow!("{error}"))
}

/// Put a line in the board's log, which is a file in the board's own folder.
pub(crate) fn log_line(state: &AppState, session_id: &str, line: &str) {
    let Ok(repo_path) = crate::moontasks::service::repo_of(state, session_id) else {
        return;
    };
    crate::moontasks::store::append_hooks_log(&repo_path, line);
}

fn runtime_error(error: anyhow::Error) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(format!("{error}")),
        Position::NONE,
    ))
}
