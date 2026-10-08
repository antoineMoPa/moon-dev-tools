//! Started application - a program started for its windows. Keeps the end of what it says on
//! its standard error and tells whoever started it how it ended, so that one that fails as it
//! starts is reported with its own words in the window that asked for it.
//!
//! Both places that start applications use it: the server's desktop - see `super::running` -
//! and a window that is its machine's whole session - see
//! `App::start_application_on_this_screen`, and [`OnThisScreen`] for what a shell of that
//! window starts with `moon launch`.

use std::{
    collections::VecDeque,
    io::{Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};

/// The longest whoever started a program waits to hear how its start went: one still running
/// by then is taken to have started. The server's desktop answers the window that asked
/// sooner whenever it can - see `Running::start_application` - and a window that is its
/// machine's session waits this long on a worker, where nothing is kept waiting. Longer than
/// a browser with no sandbox to run in takes to say so and give up, with room for a slower
/// machine than the one that was timed on: a third of a second there, at its slowest.
pub(crate) const SOON: Duration = Duration::from_millis(1500);

/// How much of what a program says on its standard error is kept, in bytes: the end of it. A
/// browser's last words about its sandbox are a thousand.
const SAID_KEPT: usize = 2048;

/// How long a program that ended in failure is given for the rest of what it said to be read.
/// What it wrote is in the pipe already and read at once; the wait has an end for the program
/// that left another running, which holds the pipe open for as long as it likes.
const LAST_WORDS_WITHIN: Duration = Duration::from_millis(200);

/// What a program said as it failed, where what to do about it is the machine's to do and is
/// not plain from the words: what to find in what it said, and the sentence added for it.
const HINTS: &[(&str, &str)] = &[(
    // Chromium, and every browser built on it. The browser's own way out, `--no-sandbox`, is
    // never added here: running a browser unsandboxed is for the person to decide and type.
    "No usable sandbox!",
    "This browser found neither of the two sandboxes it can run in: the machine denies it \
     unprivileged user namespaces, and no setuid `chrome-sandbox` is installed beside it - \
     allow one of the two on that machine (Debian: install `chromium-sandbox`; Ubuntu 23.10 \
     and later: an AppArmor profile granting this browser `userns`).",
)];

/// Where what a program says on its standard error goes, beside the end of it that is kept.
#[derive(Clone, Copy)]
pub(crate) enum Said {
    /// Nowhere: a server's log is not where a browser's chatter belongs.
    KeptOnly,
    /// To this process's own standard error, which is where it went while the program
    /// inherited it.
    PassedOn,
}

/// A program that has been started, and has yet to be waited for.
pub(crate) struct Started {
    program: Child,
    said: Arc<Mutex<LastSaid>>,
    /// Hung up on once nobody holds the program's standard error open any more, which is
    /// when all that was said on it has been read.
    all_said: mpsc::Receiver<()>,
}

impl Started {
    /// Start `program`, reading what it says on its standard error from then on.
    pub(crate) fn start(program: &mut Command, said: Said) -> Result<Self> {
        let mut program = program.stderr(Stdio::piped()).spawn()?;
        let mut saying = program
            .stderr
            .take()
            .expect("the program's standard error is piped");
        let kept = Arc::new(Mutex::new(LastSaid::default()));
        let (still_saying, all_said) = mpsc::channel();

        let keeping = Arc::clone(&kept);
        thread::Builder::new()
            .name(format!("moon application {} said", program.id()))
            .spawn(move || {
                let _still_saying: mpsc::Sender<()> = still_saying;
                let mut heard = [0; 4096];
                // Until nobody holds the other end: the program has ended, and so has
                // whatever it started that could still say something here.
                while let Ok(length @ 1..) = saying.read(&mut heard) {
                    let heard = &heard[..length];
                    keeping
                        .lock()
                        .expect("the lock on what the application said is poisoned")
                        .hear(heard);
                    if let Said::PassedOn = said {
                        let _ = std::io::stderr().write_all(heard);
                    }
                }
            })
            .context("the thread reading what the application says would not start")?;
        Ok(Self {
            program,
            said: kept,
            all_said,
        })
    }

    /// The program's process id - and its process group's, when it was started as its own.
    pub(crate) fn id(&self) -> u32 {
        self.program.id()
    }

    /// Wait for the program on a thread of its own - so that one that has ended is not kept
    /// by the system as one that has yet to be asked how it went - and do `then` once it has
    /// ended, with how it did.
    pub(crate) fn wait(self, then: impl FnOnce(Ended) + Send + 'static) -> Result<()> {
        let Self {
            mut program,
            said,
            all_said,
        } = self;
        thread::Builder::new()
            .name(format!("moon application {}", program.id()))
            .spawn(move || {
                let status = program
                    .wait()
                    .expect("a program this process started is its to wait for");
                if !status.success() {
                    // Hung up on or timed out: either way, what is kept by now is its last
                    // words.
                    let _ = all_said.recv_timeout(LAST_WORDS_WITHIN);
                }
                let said = said
                    .lock()
                    .expect("the lock on what the application said is poisoned")
                    .text();
                then(Ended { status, said });
            })
            .context("the application's thread would not start")?;
        Ok(())
    }
}

/// How a program ended.
pub(crate) struct Ended {
    status: ExitStatus,
    /// The end of what it said on its standard error.
    said: String,
}

impl Ended {
    /// What to tell whoever started `command` when this is how its start went: nothing when
    /// it ended well.
    pub(crate) fn failure_of(&self, command: &str) -> Option<String> {
        (!self.status.success()).then(|| failure(command, &self.status.to_string(), &self.said))
    }
}

/// A failure as the window says it: the command, how it ended, what it said - and what that
/// means, where [`HINTS`] knows.
fn failure(command: &str, status: &str, said: &str) -> String {
    let mut failure = match said.is_empty() {
        true => format!("`{command}` ended ({status}), saying nothing"),
        false => format!("`{command}` ended ({status}): {said}"),
    };
    for (_, hint) in HINTS.iter().filter(|(found, _)| said.contains(found)) {
        failure.push('\n');
        failure.push_str(hint);
    }
    failure
}

/// The screen this process is on, as what starts the programs a shell asks for with
/// `moon launch` in a window that is its machine's session - see
/// `App::listen_for_shell_asks`. The program's window is the session's to find and put in a
/// pane, as it is for one started from the menu.
pub(crate) struct OnThisScreen;

impl crate::instances::StartsApplications for OnThisScreen {
    /// Answered when the program ends or [`SOON`] after it was started, whichever is first,
    /// on the thread the window's socket is answered on - which draws nothing.
    fn start(&self, command: &str, folder: &std::path::Path) -> Result<String> {
        let (ending, ended) = mpsc::channel();
        Started::start(
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(folder)
                .stdin(Stdio::null()),
            // The session's log, where what an application started from the menu says goes.
            Said::PassedOn,
        )
        .and_then(|program| {
            // Nobody left to hear is the wait below being over: it ended later than soon.
            program.wait(move |ended| {
                let _ = ending.send(ended);
            })
        })
        .with_context(|| format!("`{command}` would not start"))?;
        match ended
            .recv_timeout(SOON)
            .ok()
            .and_then(|ended| ended.failure_of(command))
        {
            Some(failure) => anyhow::bail!(failure),
            None => Ok("this screen".to_string()),
        }
    }
}

/// The end of what a program said on its standard error: the last [`SAID_KEPT`] bytes.
#[derive(Default)]
struct LastSaid {
    bytes: VecDeque<u8>,
    /// Whether more was said than is kept, so that what is kept starts partway through.
    cut: bool,
}

impl LastSaid {
    fn hear(&mut self, said: &[u8]) {
        self.bytes.extend(said);
        let over = self.bytes.len().saturating_sub(SAID_KEPT);
        if over > 0 {
            self.bytes.drain(..over);
            self.cut = true;
        }
    }

    /// What is kept, as whole lines: one cut partway through is left out.
    fn text(&self) -> String {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        let text = String::from_utf8_lossy(&bytes);
        let whole_lines = match text.split_once('\n') {
            Some((_cut_line, rest)) if self.cut => rest,
            _ => &text,
        };
        whole_lines.trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(line: &str) -> Command {
        let mut shell = Command::new("sh");
        shell.args(["-c", line]).stdin(Stdio::null());
        shell
    }

    /// How `program` ended, as whoever waits for it hears.
    fn ended(program: Started) -> Ended {
        let (ending, ended) = mpsc::channel();
        program
            .wait(move |ended| ending.send(ended).expect("expected the test to be waiting"))
            .expect("expected a thread");
        ended.recv().expect("expected the program to end")
    }

    #[test]
    fn a_program_that_fails_as_it_starts_is_reported_with_what_it_said() {
        let program = Started::start(
            &mut shell("echo \"$$ has no display\" >&2; exit 3"),
            Said::KeptOnly,
        )
        .expect("expected a shell");
        let id = program.id();
        assert_eq!(
            ended(program).failure_of("xlogo"),
            Some(format!("`xlogo` ended (exit status: 3): {id} has no display"))
        );

        // One that ended well is nothing to report.
        let program =
            Started::start(&mut shell("exit 0"), Said::PassedOn).expect("expected a shell");
        assert_eq!(ended(program).failure_of("true"), None);
    }

    #[test]
    fn the_end_of_what_was_said_is_kept_and_a_browser_without_a_sandbox_is_explained() {
        let mut said = LastSaid::default();
        said.hear("first line\n".repeat(SAID_KEPT / 4).as_bytes());
        said.hear(b"second to last\nlast\n");
        let kept = said.text();
        assert!(kept.len() < SAID_KEPT);
        assert!(kept.starts_with("first line\n"), "{kept}");
        assert!(kept.ends_with("second to last\nlast"), "{kept}");

        let sandbox = "[1:1:1008/023659.034249:FATAL:zygote_host_impl_linux.cc:130] No usable \
                       sandbox! If you are running on Ubuntu 23.10+";
        let explained = failure("chromium", "signal: 5 (SIGTRAP)", sandbox);
        assert!(explained.starts_with("`chromium` ended (signal: 5 (SIGTRAP)): [1:1:"));
        assert!(explained.ends_with(HINTS[0].1), "{explained}");
        assert!(!failure("chromium", "exit status: 1", "cannot open display").contains('\n'));
        assert_eq!(
            failure("true", "exit status: 1", ""),
            "`true` ended (exit status: 1), saying nothing"
        );
    }
}
