//! The wire: how the agents working on one board tell each other what they are doing, so two
//! of them do not rewrite the same file.
//!
//! There is one way onto it, run from a task's shell - `moon wire post "<one line>"`, see
//! `crate::cli::wire` - and a line goes one of two places:
//!
//! - With no tag in front it is a broadcast, kept in `.moontasks/messageboard.txt` for every
//!   agent to read - see [`broadcasts`].
//! - Starting with `@handle` it is a direct message: the window holding that task's agent
//!   types it into the agent's shell - see [`direct`], and `crate::terminal` for the typing.
//!
//! A [handle](handles) is the front of a task's folder name, which is what an agent is tagged
//! by.

pub(crate) mod broadcasts;
pub(crate) mod direct;
pub(crate) mod handles;

use anyhow::{Result, bail};

/// The command that posts a line, as an agent types it - and the only writer of the file the
/// broadcasts are kept in.
pub(crate) fn post_command() -> String {
    format!("{} wire post", crate::cli::PROGRAM)
}

/// Hold a line to what the wire carries: some text, and nothing that is not text.
///
/// A line break would make two lines of one in the file, and typed into a shell it is the
/// Enter that sends half a message. Every other control character is refused with it, for the
/// second reason: an escape or a `^C` typed into an agent's box is a key pressed there.
pub(crate) fn one_line(text: &str) -> Result<&str> {
    if text.trim().is_empty() {
        bail!("the wire carries a line of text, and this one is empty");
    }
    if text.contains(['\n', '\r']) {
        bail!("the wire carries one line at a time, and this has a line break in it");
    }
    if let Some(character) = text.chars().find(|character| character.is_control()) {
        bail!("the wire carries text, and this has the control character {character:?} in it");
    }
    Ok(text)
}

/// What a window types into an agent's shell for a direct message, before the Enter.
pub(crate) fn typed_line(sender: &str, message: &str) -> String {
    format!("agent @{sender} sent this message: {message}")
}

/// What a window writes in its Messages for a direct message it typed into one of its shells.
pub(crate) fn logged_line(sender: &str, recipient: &str, message: &str) -> String {
    format!("@{sender} → @{recipient}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_text_is_carried_as_it_is() {
        assert_eq!(
            one_line("rewriting src/cli - tests too").expect("expected a line"),
            "rewriting src/cli - tests too"
        );
    }

    #[test]
    fn an_empty_line_and_a_line_break_are_refused() {
        for refused in [
            "",
            "   ",
            "one\ntwo",
            "one\r",
            "bell\u{7}",
            "escape\u{1b}[2J",
        ] {
            assert!(one_line(refused).is_err(), "{refused:?} should be refused");
        }
    }

    #[test]
    fn a_direct_message_is_typed_and_logged_with_who_sent_it() {
        assert_eq!(
            typed_line("bing-bong-313", "are you in src/cli?"),
            "agent @bing-bong-313 sent this message: are you in src/cli?"
        );
        assert_eq!(
            logged_line("bing-bong-313", "fix-the-races", "are you in src/cli?"),
            "@bing-bong-313 → @fix-the-races: are you in src/cli?"
        );
    }
}
