//! Lines a shell is told: text moon types into it and sends on somebody else's behalf, which
//! is how a direct message of the wire reaches an agent - see `crate::moontasks::wire`.
//!
//! Nobody at the window typed a told line, so it goes round [`TerminalSession::write_input`]:
//! it answers nothing the shell was asking, and it is not a person having typed. It is sent,
//! though - an Enter follows it - and that is what makes when it is typed matter. Two things
//! hold a line back:
//!
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

use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::{TerminalRegistry, TerminalSession};
use crate::attention::Asked;

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

/// What the Enter key writes.
const ENTER: &[u8] = b"\r";

/// The lines a shell has been told and has not had typed into it yet, oldest first.
#[derive(Default)]
pub(super) struct Told {
    waiting: VecDeque<String>,
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
        let session = self
            .get(terminal_id)
            .ok_or_else(|| anyhow::anyhow!("unknown terminal {terminal_id}"))?;
        if session.has_ended() {
            anyhow::bail!("the program in {terminal_id} has ended");
        }

        let mut told = session.told.lock().unwrap();
        told.waiting.push_back(line.to_string());
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
    fn has_ended(&self) -> bool {
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

    /// Whether a line this shell is told has to wait - see the module's own words on why.
    fn holds_back_what_it_is_told(&self) -> bool {
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

/// Type the line that has waited longest and send it, unless it is held back.
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
        told.waiting.clear();
    }
    if told.waiting.is_empty() {
        // Said under the lock a line is added under, so one added from here on starts a
        // thread of its own rather than being left to this one.
        told.being_typed = false;
        return Looked::Finished;
    }
    if session.holds_back_what_it_is_told() {
        return Looked::Held;
    }
    let line = told.waiting.pop_front().expect("a line is waiting");
    drop(told);

    // A write that fails is a shell that ended a moment ago, which the next look finds.
    if writer
        .write_all(line.as_bytes())
        .and_then(|()| writer.flush())
        .is_ok()
    {
        std::thread::sleep(ENTER_FOLLOWS_AFTER);
        if writer.write_all(ENTER).and_then(|()| writer.flush()).is_ok() {
            session.a_told_line_was_sent();
        }
    }
    Looked::Typed
}

#[cfg(test)]
#[path = "told_tests.rs"]
mod tests;
