use super::resources::Fillings;
use super::*;

/// A run's fillings, without the brief file a real one writes.
fn fillings() -> Fillings {
    Fillings {
        values: vec![("{brief}", "the brief".to_string())],
    }
}

/// Claude is given the session id and the brief - and no prompt, so it comes up knowing
/// the task and waits to be told what to do about it.
#[test]
fn an_agent_is_told_the_task_but_not_set_to_work() {
    let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

    let args = fillings()
        .with_session(Some("11111111-2222-4333-8444-555555555555"))
        .fill_all(launch.start.iter());

    assert_eq!(
        args,
        [
            "--session-id",
            "11111111-2222-4333-8444-555555555555",
            "--append-system-prompt",
            "the brief",
        ]
    );
}

#[test]
fn a_missing_session_id_takes_the_flag_in_front_of_it_with_it() {
    let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

    let args = fillings().fill_all(launch.attach.iter());

    assert!(
        !args.iter().any(|arg| arg == "--resume"),
        "expected no dangling --resume: {args:?}"
    );
    // What is left is a fresh run that still knows the task, which is the best that can be
    // done with a session that turned out not to be there.
    assert_eq!(args, ["--append-system-prompt", "the brief"]);
}

/// Attaching opens the exact session that was picked, whichever agent it belongs to.
///
/// Claude is handed the brief with it: a session resumed comes back with the system prompt it
/// was opened on, so without this a run started before the board asked for anything would
/// never hear that it can.
#[test]
fn an_attached_session_is_opened_by_its_own_id() {
    let session = "11111111-2222-4333-8444-555555555555";
    let expected: &[(AgentKind, &[&str])] = &[
        (
            AgentKind::Claude,
            &["--resume", session, "--append-system-prompt", "the brief"],
        ),
        (
            AgentKind::Codex,
            &["-c", "developer_instructions=the brief", "resume", session],
        ),
        (AgentKind::OpenCode, &["--session", session]),
    ];

    for (kind, args) in expected {
        let launch = agent_launch(*kind).expect("expected the agent to be launchable");

        assert_eq!(
            fillings()
                .with_session(Some(session))
                .fill_all(launch.attach.iter()),
            *args,
            "{kind:?} did not open the picked session"
        );
    }
}

#[test]
fn an_agent_resumed_by_its_own_reckoning_needs_no_session_id() {
    let launch = agent_launch(AgentKind::Codex).expect("expected codex to be launchable");

    let args = fillings().fill_all(launch.resume.iter());

    assert_eq!(
        args,
        ["-c", "developer_instructions=the brief", "resume", "--last"],
        "the brief goes on a resumed run too - a session resumed comes back with what it \
         was opened on and never hears anything new"
    );
}

/// OpenCode is started with no arguments at all: what it is told comes through the
/// environment, so its command line is bare.
#[test]
fn opencode_starts_with_a_bare_command_line() {
    let launch = agent_launch(AgentKind::OpenCode).expect("expected the agent to be launchable");

    assert!(
        fillings().fill_all(launch.start.iter()).is_empty(),
        "OpenCode takes its instructions from the environment, not its arguments"
    );
}

/// Starting an agent opens a conversation rather than firing a job off, so none of the
/// three is handed the work.
#[test]
fn no_agent_is_given_the_task_as_a_prompt() {
    for launch in crate::moontasks::AGENT_LAUNCHES {
        let args = fillings()
            .with_session(Some("11111111-2222-4333-8444-555555555555"))
            .fill_all(launch.start.iter());

        assert!(
            !args.iter().any(|arg| arg.contains("Fix the login page")),
            "{:?} should not be started on the work: {args:?}",
            launch.kind
        );
    }
}

/// Every agent has to come up knowing which task it is on, and the three take it three
/// different ways - a flag, a config override, an environment variable. What must not
/// happen is an agent that is told nowhere: typing it at them does not work, because the
/// box does not exist yet when a run begins.
#[test]
fn every_agent_is_told_which_task_it_is_on() {
    let fillings = Fillings {
        values: vec![
            ("{brief}", "the brief".to_string()),
            ("{brief_file}", "/repo/.moontasks/task/brief.md".to_string()),
        ],
    }
    .with_session(Some("11111111-2222-4333-8444-555555555555"));

    for launch in crate::moontasks::AGENT_LAUNCHES {
        let args = fillings.fill_all(launch.start.iter()).join(" ");
        let env = fillings
            .fill_env(launch.env)
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(" ");

        assert!(
            args.contains("the brief") || env.contains("/repo/.moontasks/task/brief.md"),
            "{:?} starts knowing nothing about the task: args {args:?}, env {env:?}",
            launch.kind
        );
    }
}

/// OpenCode is the one told through the environment: an inline config naming the brief as
/// a file to load as instructions. It has to be the file's path rather than the text.
#[test]
fn opencode_is_handed_a_config_naming_the_brief_file() {
    let launch = crate::moontasks::agent_launch(AgentKind::OpenCode).expect("a launch");
    let env = Fillings {
        values: vec![
            ("{brief}", "the brief".to_string()),
            ("{brief_file}", "/repo/.moontasks/task/brief.md".to_string()),
        ],
    }
    .fill_env(launch.env);

    assert_eq!(env.len(), 1);
    assert_eq!(env[0].0, "OPENCODE_CONFIG_CONTENT");
    let config: serde_json::Value =
        serde_json::from_str(&env[0].1).expect("the config has to be JSON OpenCode can read");
    assert_eq!(
        config["instructions"],
        serde_json::json!(["/repo/.moontasks/task/brief.md"])
    );
}

/// The brief names the task and says where its notes go, which is all an agent needs to
/// know beyond the work itself.
#[test]
fn the_brief_names_the_task_and_its_folder() {
    let brief = crate::moontasks::brief_for("Fix the login page", "/repo/.moontasks/task");

    assert!(brief.contains("Fix the login page"));
    assert!(brief.contains("/repo/.moontasks/task"));
}

#[test]
fn a_finished_agent_is_cleared_without_moving_its_task() {
    let mut metadata = TaskMetadata {
        title: "Fix the login page".to_string(),
        status: ColumnId::new("in_progress"),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            // No server in this test, so no shell of this name is live.
            terminal_id: Some("terminal-gone".to_string()),
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
        }],
    };
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));

    assert!(reconcile(&state, &mut metadata));

    assert_eq!(metadata.status, ColumnId::new("in_progress"));
    assert_eq!(metadata.resources[0].terminal_id, None);
    assert!(
        !reconcile(&state, &mut metadata),
        "a settled task changes nothing on the next read"
    );
}

#[test]
fn a_finished_agent_leaves_a_task_where_the_user_put_it() {
    let mut metadata = TaskMetadata {
        title: "Fix the login page".to_string(),
        status: ColumnId::new("done"),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: Some("terminal-gone".to_string()),
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
        }],
    };
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));

    assert!(
        reconcile(&state, &mut metadata),
        "the shell is still recorded"
    );

    assert_eq!(metadata.status, ColumnId::new("done"));
}
