//! Hooks, driven as scripts against a real board.

use serde_json::{Value, json};

use super::{HOOK_POINTS, install, reference, script};
use crate::moontasks::{
    store,
    tool_tests::{Fixture, TASK},
    tools,
};

/// Run a script against the fixture's board, the way a firing hook does.
fn run(fixture: &Fixture, source: &str) -> Result<(), String> {
    script::run(
        &fixture.state,
        &fixture.session_id,
        source,
        "test",
        Value::Null,
    )
    .map_err(|error| format!("{error}"))
}

fn run_with_event(fixture: &Fixture, source: &str, event: Value) -> Result<(), String> {
    script::run(&fixture.state, &fixture.session_id, source, "test", event)
        .map_err(|error| format!("{error}"))
}

/// The point of the whole design: a tool is callable from a script because it is in the table,
/// not because anybody wrote glue for it.
#[test]
fn every_tool_in_the_table_is_callable_from_a_script() {
    let fixture = Fixture::new("bindings");

    for tool in tools::TOOLS {
        // Called with empty arguments on purpose — what is being checked is that the name and
        // the count resolve to a function at all, which is a different failure from the tool
        // reading those arguments and refusing.
        let given = vec!["\"\""; tools::required_count(tool)].join(", ");
        let call = format!("{}({given});", tool.name);
        let Err(error) = run(&fixture, &call) else {
            continue;
        };
        assert!(
            !error.contains("Function not found"),
            "{} is a tool but no script can call it: {error}",
            tool.name
        );
    }
}

/// A tool with optional arguments is one function at each length, so leaving the end off is a
/// call rather than an error.
#[test]
fn an_optional_argument_may_simply_be_left_off() {
    let fixture = Fixture::new("optional-arguments");

    run(&fixture, &format!(r#"create_worktree("{TASK}");"#)).expect("expected a checkout");
    run(&fixture, &format!(r#"discard_worktree("{TASK}");"#)).expect("expected the short call");
    run(&fixture, &format!(r#"create_worktree("{TASK}");"#)).expect("expected another checkout");
    run(&fixture, &format!(r#"discard_worktree("{TASK}", true);"#))
        .expect("expected the long call");
}

/// A script gets the board as data, not as text to parse.
#[test]
fn a_script_reads_the_board_as_values() {
    let fixture = Fixture::new("board-as-values");

    run(
        &fixture,
        &format!(
            r#"
            let tasks = list_tasks();
            if tasks.len() != 1 {{ throw "expected one card"; }}
            if tasks[0].title != "Fix the thing" {{ throw "expected the title"; }}
            if type_of(tasks[0].worktree) != "()" {{ throw "expected no checkout yet"; }}
            append_task_notes("{TASK}", `read ${{tasks.len()}} card(s)`);
            "#
        ),
    )
    .expect("expected the script to run");

    assert_eq!(store::read_notes(&fixture.root, TASK), "read 1 card(s)\n");
}

/// What a hook is handed about what just happened.
#[test]
fn a_script_is_given_the_event_that_fired_it() {
    let fixture = Fixture::new("event");

    run_with_event(
        &fixture,
        &format!(r#"append_task_notes("{TASK}", `${{event.agent}} made ${{event.commits}}`);"#),
        json!({ "agent": "claude", "commits": 2 }),
    )
    .expect("expected the script to run");

    assert_eq!(store::read_notes(&fixture.root, TASK), "claude made 2\n");
}

/// A refusal reaches the script as an error it can catch, rather than ending the firing.
#[test]
fn a_refusal_is_an_error_a_script_can_catch() {
    let fixture = Fixture::new("catchable");

    run(
        &fixture,
        &format!(
            r#"
            try {{
                add_task_tag("{TASK}", "autopilot");
                throw "the refusal did not happen";
            }} catch (error) {{
                append_task_notes("{TASK}", "refused");
            }}
            "#
        ),
    )
    .expect("expected the script to run");

    assert_eq!(store::read_notes(&fixture.root, TASK), "refused\n");
}

/// A hook fires unattended, so a script that never finishes must not be a board that never
/// ticks again.
#[test]
fn a_script_that_runs_away_is_stopped() {
    let fixture = Fixture::new("runaway");

    let error = run(&fixture, "let n = 0; loop { n += 1; }").expect_err("expected to be stopped");

    assert!(error.contains("operations"), "got {error}");
}

/// A script is a file in the repo. Building more script out of strings at runtime is not
/// something a hook needs, and is something a hook should not be able to do.
#[test]
fn a_script_cannot_build_more_script() {
    let fixture = Fixture::new("no-eval");

    let error = run(&fixture, r#"eval("1 + 1");"#).expect_err("expected eval to be gone");

    assert!(!error.is_empty(), "got {error}");
}

/// The shipped hooks are the first Rhai most people will read, and a board whose hooks do not
/// parse is a board that silently does nothing.
#[test]
fn every_shipped_hook_parses() {
    let engine = rhai::Engine::new();

    for point in HOOK_POINTS {
        engine
            .compile(point.default)
            .unwrap_or_else(|error| panic!("{}.rhai does not parse: {error}", point.name));
    }
}

/// The shipped hooks may only call tools that exist. A name that drifts is a board that stops
/// mid-loop, at runtime, with nothing said until someone reads the log.
#[test]
fn every_shipped_hook_only_calls_tools_that_exist() {
    let fixture = Fixture::new("shipped-hooks-call-real-tools");

    for point in HOOK_POINTS {
        // Sent an event with everything any of them reads, so the run gets past its first line.
        let event = json!({
            "task_id": TASK,
            "title": "Fix the thing",
            "run_id": "none",
            "agent": "claude",
            "commits": 0,
            "column": "todo",
            "from": "todo",
        });
        if let Err(error) = run_with_event(&fixture, point.default, event) {
            assert!(
                !error.contains("Function not found"),
                "{}.rhai calls something that is not a tool: {error}",
                point.name
            );
        }
    }
}

/// The reference is what a person writing a hook reads instead of this source, so nothing may
/// be missing from it.
#[test]
fn the_reference_names_every_tool_and_every_hook_point() {
    let text = reference();

    for tool in tools::TOOLS {
        assert!(
            text.contains(&format!("`{}(", tool.name)),
            "{} is missing from the reference",
            tool.name
        );
    }
    for point in HOOK_POINTS {
        assert!(
            text.contains(&format!("`{}.rhai`", point.name)),
            "{} is missing from the reference",
            point.name
        );
    }
}

/// A board gets the shipped hooks once. After that they are the person's, and an edited hook is
/// never written back over — the whole design rests on that being true.
#[test]
fn shipped_hooks_are_written_once_and_then_left_alone() {
    let fixture = Fixture::new("install");

    install(&fixture.root).expect("expected the hooks installed");
    let path = store::hooks_dir(&fixture.root).join("tick.rhai");
    std::fs::write(&path, "// mine now\n").expect("expected to edit a hook");

    install(&fixture.root).expect("expected the second install to be fine");

    assert_eq!(
        std::fs::read_to_string(&path).expect("expected the hook"),
        "// mine now\n",
        "an edited hook must not be written back over"
    );
}

/// The reference describes this build, so unlike the hooks it is rewritten every time.
#[test]
fn the_reference_is_rewritten() {
    let fixture = Fixture::new("reference-rewritten");

    install(&fixture.root).expect("expected the hooks installed");
    let path = store::hooks_dir(&fixture.root).join("REFERENCE.md");
    std::fs::write(&path, "stale\n").expect("expected to stale the reference");

    install(&fixture.root).expect("expected the second install");

    assert_ne!(
        std::fs::read_to_string(&path).expect("expected the reference"),
        "stale\n"
    );
}

/// What a hook prints is the only account of its reasoning, since it fires with nobody in
/// front of it. It goes in the board's log.
#[test]
fn what_a_script_prints_is_written_down_in_the_board_s_log() {
    let fixture = Fixture::new("hook-log");

    run(&fixture, r#"print("waiting on a run");"#).expect("expected the script to run");

    let log =
        std::fs::read_to_string(store::hooks_log_path(&fixture.root)).expect("expected a log");
    assert!(
        log.contains("test: waiting on a run"),
        "the log should name the hook and what it said, got: {log}"
    );
}

/// The tick fires every couple of seconds, so a hook that says the same thing while it waits
/// would bury everything else. The log keeps where the reasoning changed.
#[test]
fn a_line_the_same_as_the_one_before_it_is_not_written_twice() {
    let fixture = Fixture::new("hook-log-repeats");

    for _ in 0..3 {
        run(&fixture, r#"print("waiting on a run");"#).expect("expected the script to run");
    }
    run(&fixture, r#"print("picked one up");"#).expect("expected the script to run");
    run(&fixture, r#"print("waiting on a run");"#).expect("expected the script to run");

    let log =
        std::fs::read_to_string(store::hooks_log_path(&fixture.root)).expect("expected a log");
    let said: Vec<&str> = log
        .lines()
        .filter_map(|line| line.split_once("  "))
        .map(|(_, said)| said)
        .collect();
    assert_eq!(
        said,
        [
            "test: waiting on a run",
            "test: picked one up",
            "test: waiting on a run"
        ],
        "a repeat is one line, and the same thing said again after something else is news"
    );
}

/// A hook that fails has nowhere else to say so when it is the tick, which has no card.
#[test]
fn a_failing_hook_says_why_in_the_log() {
    let fixture = Fixture::new("hook-log-failure");

    install(&fixture.root).expect("expected the hooks installed");
    std::fs::write(
        store::hooks_dir(&fixture.root).join("tick.rhai"),
        r#"throw "this board has no BACKLOG column";"#,
    )
    .expect("expected the hook written");
    super::report(
        &fixture.state,
        &fixture.session_id,
        "tick",
        None,
        Value::Null,
    );

    let log =
        std::fs::read_to_string(store::hooks_log_path(&fixture.root)).expect("expected a log");
    assert!(
        log.contains("failed — tick: Runtime error: this board has no BACKLOG column"),
        "got: {log}"
    );
}

/// The shipped autopilot, run against a board with nothing for it to do. What it decided is
/// the point: a tick that picks nothing up has to say why, or the board is silent about it.
#[test]
fn the_shipped_autopilot_says_why_it_picked_nothing_up() {
    let fixture = Fixture::new("autopilot-quiet");
    let tick = HOOK_POINTS
        .iter()
        .find(|point| point.name == crate::moontasks::AUTOPILOT_HOOK)
        .expect("expected the tick hook");

    // The fixture's one card is in TODO with no `autopilot` tag, so the run reaches the end.
    run(&fixture, tick.default).expect("expected the shipped tick to run");

    let log =
        std::fs::read_to_string(store::hooks_log_path(&fixture.root)).expect("expected a log");
    assert!(
        log.contains("nothing to pick up — no card in TODO is tagged autopilot"),
        "got: {log}"
    );
}
