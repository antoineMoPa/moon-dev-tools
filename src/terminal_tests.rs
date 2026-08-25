//! The pty layer's own tests: what a shell is started with, and what it is told.

use std::time::Duration;

use super::*;

/// A commit is signed, gpg asks for the passphrase on a terminal, and this is the one it
/// is told to ask on. Without it the commit fails rather than prompting.
#[test]
fn a_command_run_is_told_which_terminal_to_ask_for_a_passphrase_on() {
    let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
    let terminal_id = registry
        .spawn(TerminalSpec {
            cwd: std::env::temp_dir(),
            program: TerminalProgram::Command("sh".to_string()),
            args: vec!["-c".to_string(), "printf %s \"$GPG_TTY\"".to_string()],
            env: Vec::new(),
            owner: Some("commit:test".to_string()),
            type_ahead: None,
        })
        .expect("expected the command to start");
    let session = registry.get(&terminal_id).expect("expected the session");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut printed = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        printed = String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay())
            .trim()
            .to_string();
        if !printed.is_empty() {
            break;
        }
    }

    assert!(
        printed.starts_with("/dev/"),
        "the command should have been told its own tty, printed {printed:?}"
    );
    // A run moonreview started on the user's behalf is answered for once it ends, and is
    // none of the workspace's shells.
    let ending = Instant::now() + Duration::from_secs(10);
    while Instant::now() < ending && registry.is_live(&terminal_id) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        registry.take_outcome(&terminal_id),
        Some(0),
        "the command ended well and should have said so"
    );
    assert!(
        !registry.terminal_ids().contains(&terminal_id),
        "a commit's own pty is not one of the workspace's shells"
    );
}

/// Type-ahead reaches the program as keystrokes, and reaches it whole.
///
/// A login shell stands in for an agent here: it echoes what is typed at it, which is the
/// same evidence an agent's input box gives — and unlike an agent it is on every machine
/// this runs on. What it must not do is run anything, so nothing here sends a newline.
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
    // than on the deadline — which is the difference between it being there when you look
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
/// answers travel the same way keystrokes do — but they are not somebody typing, and a
/// tab being open must not be what stops the title going in.
#[cfg(feature = "native")]
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
/// without the real agent — the spec's own env wins over the PATH the spawn sets.
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
            type_ahead: None,
        })
        .expect("expected a shell")
}

/// An agent that falls over — `claude --resume` on a session id that no longer exists,
/// say — has printed the only account of what went wrong. Its shell is kept, unexited, so
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
/// is everyday use — reaped, never kept.
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
