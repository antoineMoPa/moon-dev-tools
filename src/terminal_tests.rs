//! The pty layer's own tests: what a shell is started with, and what it is told.

use std::time::Duration;

use super::*;

/// A commit is signed, gpg asks for the passphrase on a terminal, and this is the one it
/// is told to ask on. Without it the commit fails rather than prompting.
#[test]
fn a_commit_run_is_told_which_terminal_to_ask_for_a_passphrase_on() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::LoginShell,
            args: Vec::new(),
            env: Vec::new(),
            owner: Some("commit:test".to_string()),
            name: None,
            type_ahead: Some("printf %s \"[$GPG_TTY]\"\r".to_string()),
        })
        .expect("expected the shell to start");
    let session = registry.get(&terminal_id).expect("expected the session");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string();
        if printed.contains("[/dev/") {
            break;
        }
    }

    assert!(
        printed.contains("[/dev/"),
        "the shell should have been told its own tty, printed {printed:?}"
    );
    // A run moonreview started on the user's behalf belongs to the review that started it,
    // and is none of the workspace's shells.
    assert!(
        !registry.terminal_ids().contains(&terminal_id),
        "a commit's own pty is not one of the workspace's shells"
    );
}

/// Type-ahead reaches the program as keystrokes, and reaches it whole.
///
/// A login shell stands in for an agent here: it echoes what is typed at it, which is the
/// same evidence an agent's input box gives - and unlike an agent it is on every machine
/// this runs on. What it must not do is run anything, so nothing here sends a newline.
/// A project's build and run are ordinary shells with the command typed in and sent, which
/// is what makes the output stay on screen and the tab reusable once the command is over.
#[test]
fn a_project_command_is_typed_into_the_shell_and_sent() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec::running(
            std::env::temp_dir(),
            "printf %s \"[project-command-ran]\"",
        ))
        .expect("expected the shell to start");
    let session = registry.get(&terminal_id).expect("expected the session");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string();
        if printed.contains("[project-command-ran]") {
            break;
        }
    }

    assert!(
        printed.contains("[project-command-ran]"),
        "the command should have been sent, printed {printed:?}"
    );
    // The shell is the workspace's own: it is listed and closed like any other, and it
    // outlives the command it was given.
    assert!(
        registry.terminal_ids().contains(&terminal_id),
        "a project command's shell should be one of the workspace's shells"
    );
}

#[test]
fn type_ahead_is_typed_into_the_shell_and_left_unsent() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::LoginShell,
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            name: None,
            type_ahead: Some("moonreview-typed-this".to_string()),
        })
        .expect("expected a shell");
    let session = registry.get(&terminal_id).expect("expected the shell");

    // The wait before it is typed is the point of it, so this waits out that wait.
    let started = Instant::now();
    let deadline = started + TYPE_AHEAD_DEADLINE + Duration::from_secs(7);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        // The pty is 80 columns wide and a prompt takes some of them, so what was typed
        // comes back wrapped: the line break the shell inserted is dropped rather than
        // matched against.
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay())
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if printed.contains("moonreview-typed-this") {
            break;
        }
    }
    let took = started.elapsed();
    registry.remove(&terminal_id);

    assert!(
        printed.contains("moonreview-typed-this"),
        "the shell never echoed what was typed at it, printed: {printed:?}"
    );
    // A prompt is drawn and then nothing more, so the text goes in on that silence rather
    // than on the deadline - which is the difference between it being there when you look
    // at the tab and it landing over what you had started typing.
    assert!(
        took < TYPE_AHEAD_DEADLINE,
        "type-ahead waited out its deadline rather than the shell going quiet: {took:?}"
    );
    // Left at the prompt, not run: a shell that had been sent the line would have gone
    // looking for a command by that name.
    assert!(
        !printed.contains("notfound"),
        "type-ahead must not be sent, printed: {printed:?}"
    );
}

/// A terminal answers the program's own questions the moment it attaches, and those
/// answers travel the same way keystrokes do - but they are not somebody typing, and a
/// tab being open must not be what stops the title going in.
#[test]
fn a_reply_to_the_program_is_not_somebody_typing() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::LoginShell,
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            name: None,
            type_ahead: Some("moonreview-typed-this".to_string()),
        })
        .expect("expected a shell");
    let session = registry.get(&terminal_id).expect("expected the shell");

    // A device status report, which is what a terminal sends back unasked.
    session
        .write_reply(b"\x1b[0n")
        .expect("failed to answer the program");

    let deadline = Instant::now() + TYPE_AHEAD_DEADLINE + Duration::from_secs(7);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay())
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if printed.contains("moonreview-typed-this") {
            break;
        }
    }
    registry.remove(&terminal_id);

    assert!(
        printed.contains("moonreview-typed-this"),
        "the title should still have been typed, printed: {printed:?}"
    );
}

/// A folder holding a fake `claude` for the PATH, so an agent shell can be spawned
/// without the real agent - the spec's own env wins over the PATH the spawn sets.
#[cfg(unix)]
fn fake_claude(script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!(
        "moonreview-fake-agent-{}",
        crate::moontasks::store::new_uuid()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the fake agent's folder");
    let path = dir.join("claude");
    std::fs::write(&path, script).expect("failed to write the fake agent");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("failed to make the fake agent executable");
    dir
}

#[cfg(unix)]
fn spawn_fake_claude(registry: &Arc<TerminalRegistry>, script: &str) -> String {
    let path = fake_claude(script);
    registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::Agent(AgentKind::Claude),
            args: Vec::new(),
            env: vec![("PATH".to_string(), path.display().to_string())],
            owner: None,
            name: None,
            type_ahead: None,
        })
        .expect("expected a shell")
}

/// An agent that falls over - `claude --resume` on a session id that no longer exists,
/// say - has printed the only account of what went wrong. Its shell is kept, unexited, so
/// the tabs showing the error stay open, and a notice says how it ended.
#[cfg(unix)]
#[test]
fn a_failed_agent_keeps_its_shell_open_with_a_notice() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = spawn_fake_claude(
        &registry,
        "#!/bin/sh\necho 'No conversation found with session ID'\nexit 1\n",
    );
    let session = registry.get(&terminal_id).expect("expected the shell");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut printed = String::new();
    while Instant::now() < deadline {
        printed =
            String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string();
        if printed.contains("[claude exited with code 1]") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        printed.contains("No conversation found with session ID"),
        "the error itself should still be there, printed: {printed:?}"
    );
    assert!(
        printed.contains("[claude exited with code 1]"),
        "the notice should say how it ended, printed: {printed:?}"
    );
    assert!(
        registry.is_live(&terminal_id),
        "the failed shell should be kept rather than reaped"
    );
    assert!(
        !*session.exited.borrow(),
        "kept means not exited, so attached tabs stay open"
    );
    // Nothing reads the pty any more, so typing at the kept shell is discarded rather
    // than left to fill the pty's buffer and block.
    assert!(session.child_ended.load(Ordering::Relaxed));
    session
        .write_to_child(b"typed at a dead shell")
        .expect("input at a kept shell is discarded, not an error");
    registry.remove(&terminal_id);
}

/// An agent that ends cleanly is done with: reaped like any shell, tab and all.
#[cfg(unix)]
#[test]
fn an_agent_that_ends_cleanly_is_reaped() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = spawn_fake_claude(&registry, "#!/bin/sh\nexit 0\n");

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.is_live(&terminal_id) {
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !registry.is_live(&terminal_id),
        "a clean ending has nothing to keep"
    );
}

/// A login shell exits with whatever its last command returned, so a nonzero status there
/// is everyday use - reaped, never kept.
#[cfg(unix)]
#[test]
fn a_plain_shell_that_exits_nonzero_is_still_reaped() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::LoginShell,
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            name: None,
            type_ahead: None,
        })
        .expect("expected a shell");
    let session = registry.get(&terminal_id).expect("expected the shell");

    session
        .writer
        .lock()
        .unwrap()
        .write_all(b"exit 1\n")
        .expect("failed to type into the shell");

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.is_live(&terminal_id) {
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !registry.is_live(&terminal_id),
        "a plain shell's exit status is its last command's, not a failure to keep"
    );
}

/// The one thing type-ahead must never do is write into a sentence someone else started.
#[test]
fn a_shell_someone_has_typed_into_is_left_alone() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::LoginShell,
            args: Vec::new(),
            env: Vec::new(),
            owner: None,
            name: None,
            type_ahead: Some("moonreview-typed-this".to_string()),
        })
        .expect("expected a shell");
    let session = registry.get(&terminal_id).expect("expected the shell");

    // Someone gets there first, before the shell has even finished coming up.
    session.typed_into.store(true, Ordering::Relaxed);

    std::thread::sleep(TYPE_AHEAD_DEADLINE + Duration::from_secs(1));
    let printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay())
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    registry.remove(&terminal_id);

    assert!(
        !printed.contains("moonreview-typed-this"),
        "the title was typed over what was being written, printed: {printed:?}"
    );
}

/// A new agent's shell is numbered one past the highest number in use for that agent,
/// whether the shell carrying it is running or only written down on a task. A name someone
/// retyped counts for nothing, and each agent counts on its own.
#[test]
fn a_new_agent_shell_is_numbered_past_every_number_in_use() {
    let in_use = || {
        [
            "claude - 1",
            "claude - 3",
            "codex - 1",
            "parser",
            "claude - two",
        ]
        .map(str::to_string)
    };

    assert_eq!(numbered_name(AgentKind::Claude, in_use()), "claude - 4");
    assert_eq!(numbered_name(AgentKind::Codex, in_use()), "codex - 2");
    assert_eq!(numbered_name(AgentKind::OpenCode, in_use()), "opencode - 1");
    assert_eq!(numbered_name(AgentKind::Claude, []), "claude - 1");
}

/// A shell starts under the name it was given, and a plain shell under none: its tab reads
/// what the program in it sets.
#[cfg(unix)]
#[test]
fn a_shell_starts_under_the_name_it_was_given() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let path = fake_claude("#!/bin/sh\nsleep 30\n");
    let agent = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::Agent(AgentKind::Claude),
            args: Vec::new(),
            env: vec![("PATH".to_string(), path.display().to_string())],
            owner: Some("write-the-parser".to_string()),
            name: Some("claude - 2".to_string()),
            type_ahead: None,
        })
        .expect("expected a shell");
    let shell = registry
        .spawn(TerminalSpec::shell(std::env::temp_dir(), None, None))
        .expect("expected a shell");

    assert_eq!(registry.name(&agent).as_deref(), Some("claude - 2"));
    assert_eq!(registry.owner(&agent).as_deref(), Some("write-the-parser"));
    assert_eq!(registry.name(&shell), None);
    assert_eq!(registry.owner(&shell), None);
    assert_eq!(registry.live_names(), vec!["claude - 2".to_string()]);

    registry.remove(&agent);
    registry.remove(&shell);
}

/// A shell is renamed from its tab, and the name is the server's to keep, so a tab closed
/// and reopened reads it back. A blank name is refused, and so is a shell the server does
/// not have.
#[test]
fn a_shell_is_renamed_and_a_blank_name_is_refused() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let shell = registry
        .spawn(TerminalSpec::shell(std::env::temp_dir(), None, None))
        .expect("expected a shell");

    registry
        .rename(&shell, "  build ")
        .expect("expected the rename");
    assert_eq!(
        registry.name(&shell).as_deref(),
        Some("build"),
        "trimmed of the spaces around it"
    );

    assert!(
        registry.rename(&shell, "   ").is_err(),
        "a tab has to read as something"
    );
    assert_eq!(
        registry.name(&shell).as_deref(),
        Some("build"),
        "and the refusal changed nothing"
    );
    assert!(registry.rename("terminal-nobody-0", "build").is_err());
    assert_eq!(registry.name("terminal-nobody-0"), None);

    registry.remove(&shell);
}

/// Renaming a task's run writes the name on the task, where it outlives the shell: the card
/// reads it after the shell is gone, and a resumed run takes it back.
#[cfg(unix)]
#[test]
fn renaming_a_tasks_run_writes_the_name_on_the_task() {
    use crate::backend::Backend;

    const TASK: &str = "write-the-parser-1111";
    let fixture = crate::native::ui_tests::seeded_fixture("run-rename");
    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let registry = Arc::clone(&state.terminals);
    let backend = crate::backend::local::LocalBackend::new(state);
    let opened = backend
        .open_session(crate::api::OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the session to open");

    let path = fake_claude("#!/bin/sh\nsleep 30\n");
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: fixture.root.clone(),
            program: TerminalProgram::Agent(AgentKind::Claude),
            args: Vec::new(),
            env: vec![("PATH".to_string(), path.display().to_string())],
            owner: Some(TASK.to_string()),
            name: Some("claude - 1".to_string()),
            type_ahead: None,
        })
        .expect("expected a shell");
    fixture.write(
        &format!(".moontasks/{TASK}/metadata.json"),
        &format!(
            "{{\n  \"title\": \"Write the parser\",\n  \"status\": \"todo\",\n  \
             \"created_at_unix\": 1700000000,\n  \"resources\": [{{\n    \"id\": \"run\",\n    \
             \"kind\": \"agent\",\n    \"agent\": \"claude\",\n    \
             \"terminal_id\": \"{terminal_id}\",\n    \"name\": \"claude - 1\",\n    \
             \"started_at_unix\": 1700000000\n  }}]\n}}\n"
        ),
    );
    let run = |backend: &crate::backend::local::LocalBackend| {
        let tasks = backend
            .list_tasks(&opened.session_id)
            .expect("expected the board");
        let task = tasks
            .into_iter()
            .find(|task| task.id == TASK)
            .expect("expected the task on the board");
        let run = &task.resources[0];
        (run.label.clone(), run.running)
    };
    assert_eq!(run(&backend), ("claude - 1".to_string(), true));

    backend
        .rename_terminal(&opened.session_id, &terminal_id, "parser")
        .expect("expected the rename");

    assert_eq!(run(&backend), ("parser".to_string(), true));
    assert_eq!(
        crate::moontasks::store::read_task(&fixture.root, TASK)
            .expect("expected the task")
            .resources[0]
            .name
            .as_deref(),
        Some("parser"),
        "the name is written on the task"
    );

    registry.remove(&terminal_id);
    assert_eq!(
        run(&backend),
        ("parser".to_string(), false),
        "and the card reads it after the shell is gone"
    );
}

/// A shell waiting at its prompt has nothing running in it, and one with a command going
/// does: this is what the quit warning is about, so it has to tell them apart.
#[test]
fn a_shell_reads_as_running_a_command_only_while_one_runs() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec::shell(std::env::temp_dir(), None, None))
        .expect("expected the shell to start");
    let session = registry.get(&terminal_id).expect("expected the session");

    let settle = |wanted: bool| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if session.is_running_a_command() == wanted {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    };

    assert!(
        settle(false),
        "a shell that was started and left alone should be sitting at its prompt"
    );
    assert!(
        registry.terminals_running_a_command().is_empty(),
        "and so should not be listed as running a command"
    );

    session
        .write_input(b"sleep 300\n")
        .expect("expected the shell to take the command");
    assert!(
        settle(true),
        "a shell running a command should say so while it runs"
    );
    assert_eq!(
        registry.terminals_running_a_command(),
        vec![terminal_id.clone()],
        "and should be the one listed"
    );

    registry.remove(&terminal_id);
}

/// The `<E2><80><94>` bug: a window started from a desktop launcher has no locale, and the
/// pager git hands its output to draws every byte of an em dash as its own hex escape. What
/// a shell is started in is a UTF-8 locale, so the character is drawn as itself.
///
/// `less` is the tool that does the escaping and the one git uses, so it is what is run here.
#[test]
#[cfg(unix)]
fn a_shell_pages_utf8_text_as_characters_rather_than_escapes() {
    // Nothing to fix, and nothing to test, on a machine that has no UTF-8 locale to start a
    // shell in - see `crate::shell_locale::shell_lang`.
    let inherits_utf8 = ["LC_ALL", "LC_CTYPE", "LANG"].iter().any(|name| {
        std::env::var(name).is_ok_and(|value| value.to_uppercase().ends_with("UTF-8"))
    });
    if !inherits_utf8 && crate::shell_locale::shell_lang().is_none() {
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "moonreview-pager-{}",
        crate::moontasks::store::new_uuid()
    ));
    std::fs::create_dir_all(&dir).expect("failed to create the folder to page from");
    std::fs::write(dir.join("dash.txt"), "an em dash \u{2014} here\n")
        .expect("failed to write the file to page");

    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    // Quit at the end of the file rather than waiting to be told to, which is how git runs it.
    let terminal_id = registry
        .spawn(TerminalSpec::running(dir.clone(), "less -F -X dash.txt"))
        .expect("expected the shell to start");
    let session = registry.get(&terminal_id).expect("expected the session");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string();
        if printed.contains("an em dash") {
            break;
        }
    }

    registry.remove(&terminal_id);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        printed.contains('\u{2014}'),
        "the pager should have drawn the em dash itself, printed {printed:?}"
    );
    assert!(
        !printed.contains("<E2>"),
        "the pager should not have drawn the bytes of it, printed {printed:?}"
    );
}
