//! What a program prints when it wants a person: the bell, and the desktop-notification
//! sequences an agent sends when it is waiting on a question, a permission, or has finished.
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
}
