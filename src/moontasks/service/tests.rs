use super::resources::{Fillings, folder_a_run_went_in, write_task_files};
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
        (
            AgentKind::Pi,
            &["--session", session, "--append-system-prompt", "the brief"],
        ),
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
fn pi_starts_with_a_task_session_and_brief() {
    let launch = agent_launch(AgentKind::Pi).expect("Pi is launchable");
    assert_eq!(
        fillings()
            .with_session(Some("task-session"))
            .fill_all(launch.start.iter()),
        [
            "--session-id",
            "task-session",
            "--append-system-prompt",
            "the brief"
        ]
    );
    assert_eq!(
        fillings().fill_all(launch.resume.iter()),
        ["--continue", "--append-system-prompt", "the brief"]
    );
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

/// An agent started from the board itself has no card to be told about: it is told where the
/// board is and where its own folder is.
#[test]
fn the_board_tasks_brief_names_the_board_and_no_task() {
    let brief =
        crate::moontasks::board_task_brief_for("/repo/.moontasks", "/repo/.moontasks/board-1234");

    assert!(brief.contains("Board folder: /repo/.moontasks\n"));
    assert!(brief.contains("/repo/.moontasks/board-1234"));
    assert!(!brief.contains("Task:"), "got {brief}");
}

/// The brief only points at the attachments format, so an agent that attaches nothing does not
/// carry it.
#[test]
fn the_brief_points_at_the_attachments_format_without_spelling_it_out() {
    let brief = crate::moontasks::brief_for("Fix the login page", "/repo/.moontasks/task");

    assert!(brief.contains(crate::moontasks::ATTACHMENTS_BRIEF_FILE_NAME));
    assert!(!brief.contains(crate::moontasks::ATTACHMENTS_FILE_NAME));
}

/// The same for the wire: the brief says there is something to check before working and
/// where, and the one sentence about it is in that file.
#[test]
fn the_brief_points_at_the_coordination_file_without_spelling_it_out() {
    let brief = crate::moontasks::brief_for("Fix the login page", "/repo/.moontasks/task");
    let board_brief =
        crate::moontasks::board_task_brief_for("/repo/.moontasks", "/repo/.moontasks/board-task");

    assert!(brief.contains("\n\nBefore working, check Coordination.md\n\n"));
    assert!(board_brief.ends_with("\n\nBefore working, check Coordination.md in it"));
    for brief in [brief, board_brief] {
        assert!(
            !brief.contains(crate::moontasks::WIRE_FILE_NAME),
            "got {brief}"
        );
        assert!(!brief.contains("wire"), "got {brief}");
    }
}

/// The whole of what an agent is told about the wire as it starts.
#[test]
fn the_coordination_file_is_two_sentences() {
    assert_eq!(
        crate::moontasks::coordination_brief(".moontasks/messageboard.txt"),
        "Before working, read .moontasks/messageboard.txt, then post the areas you will touch: \
         `moon wire post \"<one line>\"`. Start it with @handle to message one agent instead.\n"
    );
}

/// A repo with nothing in it but what the test puts there.
fn temp_repo(name: &str) -> PathBuf {
    let repo = std::env::temp_dir().join(format!(
        "moonreview-task-files-{}-{name}-{}",
        std::process::id(),
        store::new_uuid()
    ));
    std::fs::create_dir_all(&repo).expect("failed to create the test repo");
    repo
}

/// Every file the brief points at is in the task's folder by the time an agent is started
/// there, and the file the wire's broadcasts are kept in is in the board's.
#[test]
fn a_task_started_in_is_given_the_files_its_brief_points_at() {
    let repo = temp_repo("card");
    let task_id = store::create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("todo"),
        ColumnEnd::Top,
    )
    .expect("expected the task to be made");
    let metadata = store::read_task(&repo, &task_id).expect("expected the task");

    write_task_files(&task_id, &repo, &metadata).expect("expected the files to be written");

    let dir = store::task_dir(&repo, &task_id).expect("expected the task's folder");
    let brief = std::fs::read_to_string(dir.join(crate::moontasks::BRIEF_FILE_NAME))
        .expect("expected the brief");
    for pointed_at in [
        crate::moontasks::REVIEW_REQUEST_BRIEF_FILE_NAME,
        crate::moontasks::ATTACHMENTS_BRIEF_FILE_NAME,
        crate::moontasks::COORDINATION_BRIEF_FILE_NAME,
    ] {
        assert!(
            brief.contains(pointed_at),
            "the brief should name {pointed_at}"
        );
        assert!(
            dir.join(pointed_at).is_file(),
            "{pointed_at} should be there"
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("Coordination.md")).expect("expected the file"),
        crate::moontasks::coordination_brief(&crate::moontasks::wire_repo_path())
    );
    assert_eq!(
        std::fs::read_to_string(repo.join(".moontasks/messageboard.txt"))
            .expect("expected the wire's file"),
        "",
        "the wire's file is there to be read, with nothing posted yet"
    );
}

/// The board task's folder is given the same files, and starting an agent leaves what has
/// been posted to the wire as it is.
#[test]
fn the_board_task_is_given_the_same_files_and_the_wire_keeps_its_lines() {
    let repo = temp_repo("board-task");
    store::create_board_task(&repo).expect("expected the board task to be made");
    let metadata = store::read_task(&repo, store::BOARD_TASK_ID).expect("expected the task");
    let posted = "2026-10-06 09:14 @fix-the-login-page: in src/login\n";
    std::fs::write(repo.join(".moontasks/messageboard.txt"), posted)
        .expect("failed to write the wire's file");

    write_task_files(store::BOARD_TASK_ID, &repo, &metadata)
        .expect("expected the files to be written");

    let dir = store::task_dir(&repo, store::BOARD_TASK_ID).expect("expected the folder");
    assert_eq!(
        std::fs::read_to_string(dir.join("Coordination.md")).expect("expected the file"),
        crate::moontasks::coordination_brief(&crate::moontasks::wire_repo_path())
    );
    assert!(
        std::fs::read_to_string(dir.join(crate::moontasks::BRIEF_FILE_NAME))
            .expect("expected the brief")
            .contains("check Coordination.md in it")
    );
    assert_eq!(
        std::fs::read_to_string(repo.join(".moontasks/messageboard.txt"))
            .expect("expected the wire's file"),
        posted
    );
}

#[test]
fn a_finished_agent_is_cleared_without_moving_its_task() {
    let mut metadata = TaskMetadata {
        title: "Fix the login page".to_string(),
        status: Some(ColumnId::new("in_progress")),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        remote_task_tracker_url: String::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            // No server in this test, so no shell of this name is live.
            terminal_id: Some("terminal-gone".to_string()),
            terminal_owner: None,
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }],
        made_by: None,
    };
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));

    assert!(reconcile(&state, &mut metadata));

    assert_eq!(metadata.status, Some(ColumnId::new("in_progress")));
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
        status: Some(ColumnId::new("done")),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        remote_task_tracker_url: String::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: Some("terminal-gone".to_string()),
            terminal_owner: None,
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }],
        made_by: None,
    };
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));

    assert!(
        reconcile(&state, &mut metadata),
        "the shell is still recorded"
    );

    assert_eq!(metadata.status, Some(ColumnId::new("done")));
}

/// A run on this board whose shell another moon holds - the window beside a `moon serve` - is
/// that moon's to end, not this one's: reading the board here leaves it running. Once that moon
/// has exited, the run is ended like any other.
#[test]
fn a_run_held_by_another_running_moon_is_left_alone_until_that_moon_exits() {
    let run_owned_by = |owner: u32| TaskMetadata {
        title: "Fix the login page".to_string(),
        status: Some(ColumnId::new("in_progress")),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        remote_task_tracker_url: String::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: Some("terminal-elsewhere-1".to_string()),
            terminal_owner: Some(owner),
            agent_session_id: None,
            name: None,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }],
        made_by: None,
    };
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));

    // The process that started this test is running, and is not this one.
    let mut held = run_owned_by(std::os::unix::process::parent_id());
    assert!(!reconcile(&state, &mut held));
    assert_eq!(
        held.resources[0].terminal_id.as_deref(),
        Some("terminal-elsewhere-1")
    );

    let mut exited = std::process::Command::new("true")
        .spawn()
        .expect("expected to start `true`");
    let gone = exited.id();
    exited.wait().expect("expected `true` to exit");
    let mut orphaned = run_owned_by(gone);
    assert!(reconcile(&state, &mut orphaned));
    assert_eq!(orphaned.resources[0].terminal_id, None);
    assert_eq!(orphaned.resources[0].terminal_owner, None);
}

/// A card moved to the finishing column by a moon that holds none of its shells - a command
/// line - keeps the run another running moon holds as it was recorded, and forgets the one
/// whose moon has exited.
#[test]
fn a_task_finished_from_another_moon_keeps_the_run_that_moon_holds() {
    let repo = temp_repo("finished-elsewhere");
    let task_id = store::create_task(
        &repo,
        "Fix the login page",
        &ColumnId::new("in_progress"),
        ColumnEnd::Top,
    )
    .expect("expected the task to be made");
    let run_in = |terminal_id: &str, owner: u32| TaskResource {
        id: terminal_id.to_string(),
        kind: TaskResourceKind::Agent,
        agent: AgentKind::Claude,
        file_path: None,
        terminal_id: Some(terminal_id.to_string()),
        terminal_owner: Some(owner),
        agent_session_id: None,
        name: None,
        started_at_unix: 0,
        started_by: None,
        work_tree: None,
    };
    // The process that started this test is running, and is not this one.
    let another_moon = std::os::unix::process::parent_id();
    let mut metadata = store::read_task(&repo, &task_id).expect("expected the task");
    metadata.resources = vec![
        run_in("terminal-held", another_moon),
        run_in("terminal-orphaned", exited_process()),
    ];
    store::write_task(&repo, &task_id, &metadata).expect("expected the runs to be written");
    let shells_held_here = crate::terminal::TerminalRegistry::new(std::sync::Arc::new(
        std::sync::Mutex::new(std::time::Instant::now()),
    ));

    place_tasks_in_repo(
        &shells_held_here,
        &repo,
        std::slice::from_ref(&task_id),
        ColumnId::new(store::RELEASES_SHELLS_IN),
        0,
    )
    .expect("expected the move");

    let finished = store::read_task(&repo, &task_id).expect("expected the task");
    assert_eq!(
        finished.status,
        Some(ColumnId::new(store::RELEASES_SHELLS_IN))
    );
    let [held, orphaned] = finished.resources.as_slice() else {
        panic!("expected both runs to be kept");
    };
    assert_eq!(held.terminal_id.as_deref(), Some("terminal-held"));
    assert_eq!(held.terminal_owner, Some(another_moon));
    assert_eq!(orphaned.terminal_id, None);
    assert_eq!(orphaned.terminal_owner, None);
}

/// A run recorded with its session and no shell: put on the task by hand, or by its own agent.
fn run_of_session(session_id: &str) -> TaskMetadata {
    TaskMetadata {
        title: "Fix the login page".to_string(),
        status: Some(ColumnId::new("in_progress")),
        created_at_unix: 0,
        entered_column_at_unix: None,
        position: 0,
        tags: Vec::new(),
        remote_task_tracker_url: String::new(),
        resources: vec![TaskResource {
            id: "resource".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            file_path: None,
            terminal_id: None,
            terminal_owner: None,
            agent_session_id: Some(session_id.to_string()),
            name: None,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }],
        made_by: None,
    }
}

/// A process that has exited, by the id it had.
fn exited_process() -> u32 {
    let mut exited = std::process::Command::new("true")
        .spawn()
        .expect("expected to start `true`");
    let gone = exited.id();
    exited.wait().expect("expected `true` to exit");
    gone
}

/// A run whose shell another running moon holds is going, and is not offered to be resumed:
/// resuming it here would start a second agent on the one session.
#[test]
fn a_run_another_moon_holds_is_going_and_not_offered_to_be_resumed() {
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));
    let mut held = run_of_session("a-session");
    held.resources[0].terminal_id = Some("terminal-elsewhere-1".to_string());
    // The process that started this test is running, and is not this one.
    let another_moon = std::os::unix::process::parent_id();
    held.resources[0].terminal_owner = Some(another_moon);

    let runs = resources_of(
        &state,
        Path::new("/repo"),
        Path::new("/repo"),
        "task",
        &held,
        &OpenSessions::default(),
    );

    assert_eq!(runs[0].going_elsewhere_in, Some(another_moon));
    assert!(!runs[0].running, "it is not going in a shell of this moon");
    assert!(!runs[0].resumable);
}

/// A session its agent says it has open is going, whatever put it on the task - and one the
/// agent only said it had open before it exited is a run that ended, to be resumed.
#[test]
fn a_session_its_agent_has_open_is_going_and_not_offered_to_be_resumed() {
    let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
        std::time::Instant::now(),
    )));
    let on_the_task = run_of_session("a-session");
    let agent = std::os::unix::process::parent_id();
    let view = |open: &OpenSessions| {
        let repo = Path::new("/repo");
        resources_of(&state, repo, repo, "task", &on_the_task, open).remove(0)
    };

    let open = view(&OpenSessions::said_open(&[(
        AgentKind::Claude,
        "a-session",
        agent,
    )]));
    assert_eq!(open.going_elsewhere_in, Some(agent));
    assert!(!open.resumable);

    let another_session = view(&OpenSessions::said_open(&[(
        AgentKind::Claude,
        "another",
        agent,
    )]));
    assert_eq!(another_session.going_elsewhere_in, None);
    assert!(another_session.resumable);

    let exited = view(&OpenSessions::said_open(&[(
        AgentKind::Claude,
        "a-session",
        exited_process(),
    )]));
    assert_eq!(exited.going_elsewhere_in, None);
    assert!(exited.resumable);
}

/// A run is resumed in the folder it went in: the board's checkout, or the work tree it was
/// started in - and a work tree that has gone since is refused, not swapped for the checkout.
#[test]
fn a_run_is_resumed_in_the_work_tree_it_went_in_or_not_at_all() {
    let checkout = temp_repo("resumed-in");
    let mut run = run_of_session("a-session").resources.remove(0);

    assert_eq!(
        folder_a_run_went_in(&checkout, &run, None).expect("expected the checkout"),
        checkout
    );

    let work_tree = temp_repo("resumed-in-a-work-tree");
    run.work_tree = Some(work_tree.display().to_string());
    assert_eq!(
        folder_a_run_went_in(&checkout, &run, None).expect("expected the work tree"),
        work_tree
    );

    std::fs::remove_dir_all(&work_tree).expect("failed to remove the work tree");
    let error = folder_a_run_went_in(&checkout, &run, None).expect_err("expected a refusal");
    assert!(
        error.to_string().contains(&work_tree.display().to_string()),
        "{error}"
    );
}
