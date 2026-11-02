//! Lines a shell is told: text moon types into it and sends on somebody else's behalf, which
//! is how a direct message of the wire reaches an agent - see `crate::moontasks::wire`.
//!
//! Nobody at the window typed a told line, so it goes round [`TerminalSession::write_input`]:
//! it answers nothing the shell was asking, and it is not a person having typed. It is sent,
//! though - an Enter follows it - and that is what makes when it is typed matter. Three
//! things hold a line back:
//!
//! - The shell's agent has only just been started. While it is still drawing its interface
//!   it has no box to take a line, and keys written at it then are dropped - which is what a
//!   line told to an agent the moment it was started, as `moon agent start` and then
//!   `moon agent tell` do, would otherwise be. And started that way it may come up asking
//!   whether it trusts the folder, which the Enter would answer - see [`super::trust`].
//! - The shell is asking a person something - see [`crate::attention`]. An agent waiting on
//!   a permission has a choice highlighted, and an Enter there takes it. An agent that has
//!   only finished and is waiting at its box to be typed at is not held: a line is what it is
//!   waiting for, and typing one answers that ask - see
//!   [`crate::attention::WAITING_TO_BE_TYPED_AT`].
//! - Somebody typed into the shell a moment ago. The line would land in the sentence they are
//!   in the middle of, and the Enter would send both.
//!
//! A held line is not dropped: it waits, and the lines told after it wait behind it, so a
//! shell reads what it is told in the order it was told. What is still waiting when the shell
//! ends goes with it.
//!
//! A line cannot carry a line break, so what runs to several lines is told as a paste - see
//! [`TerminalRegistry::tell_paste`]. It waits in the same queue, held back by the same three
//! things, and its sender hears how it ended, since it may wait for as long as a person takes
//! to answer a permission.

use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::{TYPE_AHEAD_DEADLINE, TYPE_AHEAD_QUIET, TerminalRegistry, TerminalSession};
use crate::{api::CancelToken, attention::Asked};

/// How long after the last keystroke in a shell a line it is told still waits. Long enough to
/// outlast a pause between two words, and short next to how long an agent takes over anything.
const WAITS_AFTER_TYPING: Duration = Duration::from_secs(5);

/// How often a line that is held looks at whether it still is, and how long passes between
/// one line being sent and the next being typed.
const LOOKS_AGAIN_AFTER: Duration = Duration::from_millis(200);

/// How long after a line's text its Enter is written.
///
/// An agent's interface reads a burst of characters that arrives at once as a paste, and in
/// a paste a return is a line break in the box rather than the key that sends it - Claude
/// Code's does. Written apart, the text is read as typing and the return as Enter.
const ENTER_FOLLOWS_AFTER: Duration = Duration::from_millis(100);

/// How long a line waits at most for an agent to start reading its keys - see
/// [`TerminalSession::is_still_coming_up`]. One that has not by then never will, and the line
/// is typed for whatever is there.
const READS_KEYS_WITHIN: Duration = Duration::from_secs(60);

/// How long the Enter waits at most for the line to show on the shell's screen - see
/// [`wait_until_shown`]. A shell that has not drawn it by then is sent the Enter anyway.
const ENTER_WAITS_FOR_THE_LINE_AT_MOST: Duration = Duration::from_secs(10);

/// What the Enter key writes.
const ENTER: &[u8] = b"\r";

/// What a program that has turned bracketed paste on reads around a paste.
const PASTE_STARTS: &[u8] = b"\x1b[200~";
const PASTE_ENDS: &[u8] = b"\x1b[201~";

/// How long after a paste its Enter is written. There is no waiting to see it drawn as there
/// is for a line: an agent folds a long paste to a mark such as `[Pasted text +12 lines]`.
const ENTER_FOLLOWS_A_PASTE_AFTER: Duration = Duration::from_millis(500);

/// How a paste a shell was told ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PasteEnded {
    Sent,
    /// The sender took it back while it was waiting its turn.
    Canceled,
    Failed(String),
}

/// What hears how a paste ended.
pub(crate) type OnPasteEnded = Box<dyn FnOnce(PasteEnded) + Send>;

/// Text of several lines waiting to be pasted into a shell and sent.
struct WaitingPaste {
    text: String,
    canceled: CancelToken,
    on_ended: OnPasteEnded,
}

/// One thing a shell has been told and has not had typed into it yet.
enum Waiting {
    Line(String),
    Paste(WaitingPaste),
}

/// What a shell has been told and has not had typed into it yet, oldest first.
#[derive(Default)]
pub(super) struct Told {
    waiting: VecDeque<Waiting>,
    /// Whether a thread is typing them. One at most, which is what keeps them in order.
    being_typed: bool,
}

/// What became of the look a typing thread took at its shell's lines.
enum Looked {
    /// One was typed and sent, and there may be more.
    Typed,
    /// The next one is held back - see the module's own words on why.
    Held,
    /// There are none left, or no shell left to type them into.
    Finished,
}

impl TerminalRegistry {
    /// Type a line into a shell and send it, once nothing holds it back - see [this
    /// module](self).
    /// Answers as soon as the line is waiting its turn, which is before it is typed.
    ///
    /// A line is one line of text: a line break in it would be an Enter in the middle of it.
    pub(crate) fn tell(&self, terminal_id: &str, line: &str) -> anyhow::Result<()> {
        if line.is_empty() || line.chars().any(char::is_control) {
            anyhow::bail!("a shell is told one line of text, and {line:?} is not one");
        }
        self.told_to_wait(terminal_id, Waiting::Line(line.to_string()))
    }

    /// Paste text of any number of lines into a shell and send it, once nothing holds it back.
    /// Answers as soon as the paste is waiting its turn; `on_ended` hears how it ended, unless
    /// this answers an error, in which case the paste never joined the queue.
    pub(crate) fn tell_paste(
        &self,
        terminal_id: &str,
        text: &str,
        canceled: CancelToken,
        on_ended: OnPasteEnded,
    ) -> anyhow::Result<()> {
        // Source code can hold anything. A control character in it must not reach the terminal
        // as one: a paste's own end marker would turn the rest of the text into keystrokes.
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
            .collect();
        if text.trim().is_empty() {
            anyhow::bail!("a shell is told a paste with something in it");
        }
        self.told_to_wait(
            terminal_id,
            Waiting::Paste(WaitingPaste {
                text,
                canceled,
                on_ended,
            }),
        )
    }

    fn told_to_wait(&self, terminal_id: &str, what: Waiting) -> anyhow::Result<()> {
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow::anyhow!("unknown terminal {terminal_id}"))?;
        if session.has_ended() {
            anyhow::bail!("the program in {terminal_id} has ended");
        }

        let mut told = session.told.lock().unwrap();
        told.waiting.push_back(what);
        if !told.being_typed {
            told.being_typed = true;
            let typing_into = Arc::clone(&session);
            // A thread of its own, so being told answers straight away however long the line
            // is held for.
            std::thread::spawn(move || type_what_it_is_told(&typing_into));
        }
        Ok(())
    }
}

impl TerminalSession {
    /// Whether the program is gone: the shell has exited, or it is an agent that fell over
    /// and is only kept for its error to be read.
    pub(super) fn has_ended(&self) -> bool {
        self.has_exited() || self.child_ended.load(Ordering::Relaxed)
    }

    /// Whether an ask of this shell's is its agent having finished and waiting to be typed
    /// at. Never, for a login shell: what a program in one means by a notification is unknown.
    fn is_a_wait_to_be_typed_at(&self, asked: &Asked) -> bool {
        self.program
            .agent()
            .is_some_and(|agent| asked.is_waiting_to_be_typed_at(agent))
    }

    /// A line was typed into the shell and sent. An agent that was waiting to be typed at has
    /// been, so that ask is answered; any other stands - one made since the line was let
    /// through is a question the line was no answer to.
    fn a_told_line_was_sent(&self) {
        let mut attention = self.attention.lock().unwrap();
        if attention
            .as_ref()
            .is_some_and(|standing| self.is_a_wait_to_be_typed_at(&standing.asked))
        {
            *attention = None;
        }
    }

    /// Whether the agent in this shell may still be coming up, and so have no box to take a
    /// line. Until it has put its terminal in raw mode it is not reading keys at all: what is
    /// typed then is echoed by the terminal and handed over with its Enter in one read, which
    /// the agent takes for a paste whose Enter sends nothing. Once it reads keys, it is the
    /// wait the card's title used to make before it was typed - see `type_ahead` in
    /// [`super::spawning`]: over once the agent has printed something and gone quiet, and over
    /// whatever it is printing once it has had as long as any of them takes.
    /// Never, for a login shell: what is typed before its prompt is up is read once it is.
    fn is_still_coming_up(&self) -> bool {
        if self.program.agent().is_none() || self.started.elapsed() >= READS_KEYS_WITHIN {
            return false;
        }
        if !self.reads_keys_as_they_come() {
            return true;
        }
        if self.started.elapsed() >= TYPE_AHEAD_DEADLINE {
            return false;
        }
        let drawn_and_waiting = self
            .last_output
            .lock()
            .unwrap()
            .is_some_and(|last| last.elapsed() >= TYPE_AHEAD_QUIET);
        !drawn_and_waiting
    }

    /// Whether the program has its terminal out of canonical mode, reading each key as it
    /// comes rather than a line at a time - which every agent's interface does once its box
    /// is up. A terminal whose mode cannot be read is taken to be reading.
    #[cfg(unix)]
    fn reads_keys_as_they_come(&self) -> bool {
        let Some(fd) = self.master.lock().unwrap().as_raw_fd() else {
            return true;
        };
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `fd` is the pty's master, held open by `self.master`, and `termios` is
        // written whole by a call that answers 0.
        if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
            return true;
        }
        let termios = unsafe { termios.assume_init() };
        termios.c_lflag & libc::ICANON == 0
    }

    #[cfg(not(unix))]
    fn reads_keys_as_they_come(&self) -> bool {
        true
    }

    /// Whether a line this shell is told has to wait - see the module's own words on why.
    fn holds_back_what_it_is_told(&self) -> bool {
        if self.is_still_coming_up() || self.trust_unanswered.load(Ordering::Relaxed) {
            return true;
        }
        let asking = self
            .attention
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|standing| !self.is_a_wait_to_be_typed_at(&standing.asked));
        let typed_just_now = self
            .last_typed_into
            .lock()
            .unwrap()
            .is_some_and(|last| last.elapsed() < WAITS_AFTER_TYPING);
        asking || typed_just_now
    }
}

/// Type every line the shell has been told, in order, each as it stops being held back.
fn type_what_it_is_told(session: &TerminalSession) {
    loop {
        if let Looked::Finished = type_the_next_line(session) {
            return;
        }
        std::thread::sleep(LOOKS_AGAIN_AFTER);
    }
}

/// Type what has waited longest and send it, unless it is held back.
///
/// The writer is held from before the shell is looked at until the Enter is written. It is
/// the lock a keystroke takes, so a person who typed just before is seen, and one who types
/// from here on lands after the Enter rather than between the line and it - at the price of
/// that one keystroke waiting out [`ENTER_FOLLOWS_AFTER`].
fn type_the_next_line(session: &TerminalSession) -> Looked {
    let mut writer = session.writer.lock().unwrap();
    let mut told = session.told.lock().unwrap();
    // A shell that has ended reads nothing, and its pty refuses writes or blocks on them.
    if session.has_ended() {
        for waiting in told.waiting.drain(..) {
            if let Waiting::Paste(paste) = waiting {
                (paste.on_ended)(PasteEnded::Failed(
                    "the agent ended before it could be told".to_string(),
                ));
            }
        }
    }
    if told.waiting.is_empty() {
        // Said under the lock a line is added under, so one added from here on starts a
        // thread of its own rather than being left to this one.
        told.being_typed = false;
        return Looked::Finished;
    }
    // Before what holds the queue back: a paste being taken back is why someone would rather
    // not wait for that.
    let taken_back = matches!(
        told.waiting.front(),
        Some(Waiting::Paste(paste)) if paste.canceled.load(Ordering::SeqCst)
    );
    if !taken_back && session.holds_back_what_it_is_told() {
        return Looked::Held;
    }
    let waiting = told.waiting.pop_front().expect("something is waiting");
    drop(told);

    match waiting {
        Waiting::Line(line) => type_a_line(session, &mut *writer, &line),
        Waiting::Paste(paste) if taken_back => (paste.on_ended)(PasteEnded::Canceled),
        Waiting::Paste(paste) => {
            let ended = paste_and_send(session, &mut *writer, &paste.text);
            (paste.on_ended)(ended);
        }
    }
    Looked::Typed
}

fn type_a_line(session: &TerminalSession, writer: &mut dyn Write, line: &str) {
    // A write that fails is a shell that ended a moment ago, which the next look finds.
    if writer
        .write_all(line.as_bytes())
        .and_then(|()| writer.flush())
        .is_ok()
    {
        std::thread::sleep(ENTER_FOLLOWS_AFTER);
        wait_until_shown(session, line);
        if writer.write_all(ENTER).and_then(|()| writer.flush()).is_ok() {
            session.a_told_line_was_sent();
        }
    }
}

/// A program that has not turned bracketed paste on cannot tell a paste's line breaks from
/// Enter, and each would send half of the text: it is refused rather than typed.
fn paste_and_send(session: &TerminalSession, writer: &mut dyn Write, text: &str) -> PasteEnded {
    match session.reads_bracketed_pastes() {
        Ok(true) => {}
        Ok(false) => {
            return PasteEnded::Failed("the agent is not reading pastes".to_string());
        }
        Err(error) => return PasteEnded::Failed(error.to_string()),
    }
    let pasted = [PASTE_STARTS, text.as_bytes(), PASTE_ENDS]
        .iter()
        .try_for_each(|part| writer.write_all(part))
        .and_then(|()| writer.flush());
    if let Err(error) = pasted {
        return PasteEnded::Failed(format!("the paste did not reach the agent: {error}"));
    }
    std::thread::sleep(ENTER_FOLLOWS_A_PASTE_AFTER);
    if let Err(error) = writer.write_all(ENTER).and_then(|()| writer.flush()) {
        return PasteEnded::Failed(format!("the paste was left unsent: {error}"));
    }
    session.a_told_line_was_sent();
    PasteEnded::Sent
}

/// Wait until the shell's screen shows `line`, which is the program having read it.
///
/// An agent can go quiet before its box is reading keys - one loading its servers, in a repo
/// with many. The line and the Enter then wait unread in the pty together and are read as
/// one burst, which the agent takes for a paste: the Enter becomes a line break in its box
/// and nothing is sent. Once the line is drawn it has been read, and the Enter is a key of
/// its own. Looked for with its spaces taken out, since a long line wraps in the box.
fn wait_until_shown(session: &TerminalSession, line: &str) {
    let wanted: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    let deadline = std::time::Instant::now() + ENTER_WAITS_FOR_THE_LINE_AT_MOST;
    while std::time::Instant::now() < deadline && !session.has_ended() {
        // A screen that cannot be read is one the line cannot be seen on: the Enter goes.
        let Ok(screen) = session.shown(super::Shown::Screen) else {
            return;
        };
        let screen: String = screen.chars().filter(|c| !c.is_whitespace()).collect();
        if screen.contains(&wanted) {
            return;
        }
        std::thread::sleep(LOOKS_AGAIN_AFTER);
    }
}

#[cfg(test)]
#[path = "told_tests.rs"]
mod tests;
