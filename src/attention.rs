//! Attention requests - detects what a program prints to get a person's attention: the bell,
//! and the desktop-notification sequences an agent sends when it is waiting on a question or a
//! permission, or has finished.
//!
//! A terminal like Ghostty turns these into notifications on the desktop. moon reads the same
//! bytes off the pty and keeps them: the run's dot on its card turns red, and the text goes
//! to the window's messages, where it can be read back whenever - and never to the desktop,
//! which nobody asked to be interrupted by.
//!
//! Three sequences carry a message, and every terminal understands at least one of them, so
//! an agent picks by what it thinks it is talking to - see the launches in
//! [`crate::moontasks`], which tell each agent which to send:
//!
//! - OSC 9, iTerm2's: `ESC ] 9 ; message ST`. Claude Code's `iterm2` channel, and what
//!   OpenCode is told to use.
//! - OSC 777, rxvt's, which Ghostty and WezTerm honour: `ESC ] 777 ; notify ; title ; body ST`.
//! - OSC 99, kitty's: `ESC ] 99 ; key=value:… ; payload ST`, the payload base64 when `e=1`.
//!
//! `ST` is `BEL` or `ESC \`. The bell on its own is the fourth signal, and the one Claude Code
//! falls back to in a terminal it does not know.

// Reading a shell's output is the server's: the window in a browser is only told what it asked.
#[cfg(not(target_arch = "wasm32"))]
mod scanner;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use scanner::{Attention, Scanner};

use serde::{Deserialize, Serialize};

/// One request for attention, as the shell printed it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", content = "message", rename_all = "lowercase")]
pub(crate) enum Asked {
    /// A bare BEL: something wants a look, and said nothing about what.
    Bell,
    /// A desktop notification, with what it said.
    Notification(String),
}

impl Asked {
    /// The line the window shows for it.
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Bell => "rang its bell",
            Self::Notification(message) => message,
        }
    }

    /// Whether this is `agent` saying it has finished and is waiting to be typed at, rather
    /// than waiting on an answer - see [`WAITING_TO_BE_TYPED_AT`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn is_waiting_to_be_typed_at(&self, agent: crate::api::AgentKind) -> bool {
        let Self::Notification(message) = self else {
            return false;
        };
        WAITING_TO_BE_TYPED_AT
            .iter()
            .any(|(says_it, ending)| *says_it == agent && message.ends_with(ending))
    }
}

/// What each agent's notification ends with when it has only finished its turn and is waiting
/// at its box to be typed at - as opposed to waiting on a permission or a question.
///
/// The two are told apart for the lines moon types into a shell on somebody else's behalf -
/// see `told` in [`crate::terminal`]. Text and an Enter are what an agent waiting at its box
/// is there for; at a permission, the Enter takes whichever choice is highlighted. So an
/// agent with no entry here, a bell, and any notification that is not one of these are all
/// taken to be a question.
///
/// Matched against the end of the message and for that agent alone: some sequences carry a
/// title in front of the text, and another agent's notification may quote anything at all -
/// a command it wants approved, say.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const WAITING_TO_BE_TYPED_AT: &[(crate::api::AgentKind, &str)] = &[
    // After a minute at its box with nothing typed into it.
    (
        crate::api::AgentKind::Claude,
        "Claude is waiting for your input",
    ),
    // As a session ends its turn.
    (crate::api::AgentKind::OpenCode, "Session done"),
];

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::api::AgentKind;

    fn said(message: &str) -> Asked {
        Asked::Notification(message.to_string())
    }

    #[test]
    fn an_agent_that_finished_is_waiting_to_be_typed_at_and_one_asking_is_not() {
        assert!(
            said("Claude is waiting for your input").is_waiting_to_be_typed_at(AgentKind::Claude)
        );
        // With the title some sequences carry in front.
        assert!(
            said("Claude Code: Claude is waiting for your input")
                .is_waiting_to_be_typed_at(AgentKind::Claude)
        );
        assert!(said("opencode: Session done").is_waiting_to_be_typed_at(AgentKind::OpenCode));

        assert!(
            !said("Claude needs your permission to use Bash")
                .is_waiting_to_be_typed_at(AgentKind::Claude)
        );
        assert!(!said("Permission needs input").is_waiting_to_be_typed_at(AgentKind::OpenCode));
        assert!(!Asked::Bell.is_waiting_to_be_typed_at(AgentKind::Claude));
    }

    /// Another agent's notification may quote anything, so one agent's words are not read
    /// out of another's.
    #[test]
    fn one_agents_words_in_another_agents_notification_are_a_question() {
        assert!(
            !said("Approval requested: echo Session done")
                .is_waiting_to_be_typed_at(AgentKind::Codex)
        );
        assert!(
            !said("Claude is waiting for your input").is_waiting_to_be_typed_at(AgentKind::Codex)
        );
    }
}
