//! Starting a shell on a pty: the program it runs, the threads that read and wait on it, and
//! what is typed into an agent once its input box is up.

use std::{
    io::{Read, Write},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Instant,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::{broadcast, watch};

use crate::api::AgentKind;

use super::{
    BROADCAST_CAPACITY, CLAUDE_NOTIFY_THROUGH_THE_TERMINAL, OUTPUT_CHUNK_SIZE, Scrollback,
    TYPE_AHEAD_DEADLINE, TYPE_AHEAD_POLL, TYPE_AHEAD_QUIET, TerminalProgram, TerminalRegistry,
    TerminalSession, TerminalSpec,
};

impl TerminalRegistry {
    pub(crate) fn spawn(self: &Arc<Self>, spec: TerminalSpec) -> anyhow::Result<String> {
        let pty = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut command = match &spec.program {
            TerminalProgram::LoginShell => {
                let mut command = CommandBuilder::new(crate::shell_path::login_shell());
                command.arg("-l");
                command
            }
            TerminalProgram::Agent(AgentKind::None) => {
                unreachable!("no agent picked is the login shell, see TerminalProgram::of_agent")
            }
            TerminalProgram::Agent(AgentKind::Claude) => CommandBuilder::new("claude"),
            TerminalProgram::Agent(AgentKind::Codex) => CommandBuilder::new("codex"),
            TerminalProgram::Agent(AgentKind::OpenCode) => CommandBuilder::new("opencode"),
        };
        // Every agent moon starts is told to say through the terminal when it wants a
        // person, the way it would tell iTerm2 or Ghostty - which is what the window reads
        // off the pty and shows on the run's card, and never sends to the desktop. Claude
        // takes it as a setting on its command line, OpenCode as an environment variable of
        // its terminal library's - see [`crate::attention`].
        match &spec.program {
            TerminalProgram::Agent(AgentKind::Claude) => {
                command.arg("--settings");
                command.arg(CLAUDE_NOTIFY_THROUGH_THE_TERMINAL);
            }
            TerminalProgram::Agent(AgentKind::OpenCode) => {
                command.env("OPENTUI_NOTIFICATION_PROTOCOL", "osc9");
            }
            _ => {}
        }
        // Codex is told how to show a visualization, which moon then shows beside it - see
        // [`crate::visualizations`].
        let arguments = match &spec.program {
            TerminalProgram::Agent(AgentKind::Codex) => {
                crate::visualizations::codex_launch::codex_arguments(
                    &crate::visualizations::codex_home(),
                    &spec.args,
                )?
            }
            _ => spec.args.clone(),
        };
        for argument in &arguments {
            command.arg(argument);
        }
        command.cwd(&spec.cwd);
        command.env("TERM", "xterm-256color");
        // Which terminal a passphrase can be asked on. gpg launches pinentry and then fails
        // with "Inappropriate ioctl for device" when this is unset, which is what a signed
        // commit started from a window rather than a shell would otherwise hit: the window
        // inherits no GPG_TTY, and the pty it just opened is the terminal to name.
        if let Some(tty_name) = pty.master.tty_name() {
            command.env("GPG_TTY", tty_name);
        }
        // Which window a `moon open` typed in this shell should land in: this process's, when
        // this process is a window. A `serve` on another machine runs shells too and is no
        // window, so what this names is checked against the windows written down before it is
        // believed - see `crate::instances`.
        command.env(crate::instances::WINDOW_ENV, std::process::id().to_string());
        // The agent is started by name, so it has to be looked up on the PATH the user's shell
        // has rather than the one a desktop launcher hands this process.
        command.env("PATH", crate::shell_path::installed_tools_path());
        // Which characters the tools in the shell can read and write - see
        // `crate::shell_locale`. A window has already adopted this into its own environment,
        // so the shell would inherit it either way; a test's registry never goes through
        // `run`, and this is what starts its shells in the same locale a window's are.
        if let Some(lang) = crate::shell_locale::shell_lang() {
            command.env("LANG", lang);
        }
        // A test's shell is the developer's own login shell, started with their HOME, so
        // whatever a test types into it would be saved to their bash history on exit. An
        // empty HISTFILE is what bash reads as "keep none", and it survives the profile.
        #[cfg(test)]
        command.env("HISTFILE", "");
        for (name, value) in &spec.env {
            command.env(name, value);
        }
        let child = pty.slave.spawn_command(command)?;
        let child_pid = child.process_id();
        // The slave handle must be dropped so the reader sees EOF once the shell exits.
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader()?;
        let writer = pty.master.take_writer()?;
        let (output, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (exited, _) = watch::channel(false);

        let order = self.next_id.fetch_add(1, Ordering::Relaxed);
        let terminal_id = format!("terminal-{}-{order}", self.run);
        let started_at_unix = crate::moontasks::store::now_unix();
        let session = Arc::new(TerminalSession {
            owner: spec.owner,
            name: Mutex::new(spec.name),
            program: spec.program.clone(),
            started_at_unix,
            order,
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
            child: Mutex::new(child),
            output: output.clone(),
            exited: exited.clone(),
            scrollback: Mutex::new(Scrollback::default()),
            last_output: Mutex::new(None),
            typed_into: std::sync::atomic::AtomicBool::new(false),
            attention: Mutex::new(None),
            attention_scanner: Mutex::new(Default::default()),
            child_ended: std::sync::atomic::AtomicBool::new(false),
            last_activity: Arc::clone(&self.last_activity),
            child_pid,
            visualizations: (spec.program == TerminalProgram::Agent(AgentKind::Codex)).then(|| {
                Mutex::new(crate::visualizations::rollout::CodexRollouts::new(
                    &crate::visualizations::codex_home(),
                    started_at_unix,
                ))
            }),
        });
        self.sessions
            .lock()
            .unwrap()
            .insert(terminal_id.clone(), Arc::clone(&session));

        // Typed from a thread of its own, so starting a shell answers straight away rather
        // than after the wait.
        if let Some(text) = spec.type_ahead {
            let typing_into = Arc::clone(&session);
            std::thread::spawn(move || type_ahead(&typing_into, &text));
        }

        let registry = Arc::clone(self);
        let reaped_id = terminal_id.clone();
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; OUTPUT_CHUNK_SIZE];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let chunk = &buffer[..count];
                        crate::api::mark_activity(&session.last_activity);
                        *session.last_output.lock().unwrap() = Some(Instant::now());
                        for asked in session.attention_scanner.lock().unwrap().feed(chunk) {
                            session.asked_for_attention(asked);
                        }
                        // A window attached now reads this as it comes, and answers any question
                        // in it - which a later replay then leaves out.
                        let seen_live = output.receiver_count() > 0;
                        session.scrollback.lock().unwrap().push(chunk, seen_live);
                        // No attached tab is normal: the shell keeps running regardless.
                        let _ = output.send(chunk.to_vec());
                    }
                }
            }
            // An agent that fell over on its own - `claude --resume` on a session id that no
            // longer exists, say - has printed the only account of what went wrong, and the
            // window closes the tab of a shell marked exited. So its session is kept, with a
            // notice saying how it ended, until the user closes it themselves. A shell taken
            // out of the registry already was ended on purpose, and has nothing to explain.
            match failure_notice(&session) {
                Some(notice) if registry.is_live(&reaped_id) => {
                    session.child_ended.store(true, Ordering::Relaxed);
                    // Moon's own words, which ask nothing.
                    session.scrollback.lock().unwrap().push(notice.as_bytes(), true);
                    let _ = output.send(notice.into_bytes());
                }
                _ => {
                    // `send` is refused when nothing is subscribed, and would leave the flag
                    // false for whoever asks later; the value has to be set whether or not
                    // anyone is listening.
                    exited.send_replace(true);
                    registry.remove(&reaped_id);
                }
            }
        });

        Ok(terminal_id)
    }
}

/// The notice a shell's ending earns, if it is one worth keeping on screen: an agent whose
/// process ended in failure. `None` says the shell is done with and can be reaped.
///
/// A plain login shell is never kept: it exits with whatever its last command returned, so a
/// nonzero status there is everyday use rather than the program falling over.
fn failure_notice(session: &TerminalSession) -> Option<String> {
    let program = match session.program.agent() {
        None => return None,
        Some(agent) => agent.label().to_lowercase(),
    };
    let status = session.child.lock().unwrap().wait().ok()?;
    if status.success() {
        return None;
    }

    let ending = match status.signal() {
        Some(signal) => format!("was ended by {signal}"),
        None => format!("exited with code {}", status.exit_code()),
    };
    Some(format!("\r\n\x1b[31m[{program} {ending}]\x1b[0m\r\n"))
}

/// Type text into a shell once the program in it has a box to take it, as if a person had.
///
/// The wait is what makes this work at all: keys written at an agent that has not drawn its
/// input box yet are dropped by it, so the text has to arrive after that and - this is the
/// point - before the person starts writing over it. So it waits for the program to stop
/// drawing rather than for a fixed span, and if the person got there first it types nothing:
/// a title landing in the middle of a sentence someone is writing is worse than no title.
fn type_ahead(session: &TerminalSession, text: &str) {
    let deadline = Instant::now() + TYPE_AHEAD_DEADLINE;
    while Instant::now() < deadline {
        if *session.exited.borrow()
            || session.child_ended.load(Ordering::Relaxed)
            || session.typed_into.load(Ordering::Relaxed)
        {
            return;
        }
        // Silence before the program has printed anything at all is it still starting up, not
        // a box waiting to be typed into.
        let quiet = session
            .last_output
            .lock()
            .unwrap()
            .is_some_and(|last| last.elapsed() >= TYPE_AHEAD_QUIET);
        if quiet {
            break;
        }
        std::thread::sleep(TYPE_AHEAD_POLL);
    }

    // Checked again holding the writer, which is the lock a keystroke takes: a person who has
    // typed by now is seen, and one who types from here on waits and lands after the whole of
    // the text rather than inside it. The shell may also have ended, and a dead pty refuses
    // writes.
    let mut writer = session.writer.lock().unwrap();
    if *session.exited.borrow()
        || session.child_ended.load(Ordering::Relaxed)
        || session.typed_into.load(Ordering::Relaxed)
    {
        return;
    }
    let _ = writer.write_all(text.as_bytes());
}
