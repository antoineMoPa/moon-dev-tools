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

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The most of a message that is kept. A notification is a line, not a document.
const MESSAGE_LIMIT: usize = 240;
/// The most of a sequence that is read before it is given up on as not a notification: a
/// program painting an image through an OSC would otherwise be buffered whole.
const SEQUENCE_LIMIT: usize = 4096;

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

/// A request for attention a shell has made and nobody has answered: kept on the shell until
/// someone types into it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Attention {
    pub(crate) asked: Asked,
    /// When, in seconds since the epoch: what tells a window it has already posted this one.
    pub(crate) at_unix: u64,
}

impl Attention {
    pub(crate) fn now(asked: Asked) -> Self {
        Self {
            asked,
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Plain,
    /// After an ESC, waiting to see whether `]` follows.
    Escape,
    /// Inside an OSC, collecting it up to its terminator.
    Osc,
    /// After an ESC inside an OSC: `\` closes it, anything else abandons it.
    OscEscape,
}

/// Reads a shell's output as it comes, in whatever pieces it comes in, and picks out the
/// requests for attention. A sequence split across two reads is one sequence.
pub(crate) struct Scanner {
    state: State,
    sequence: Vec<u8>,
    /// Whether the sequence being read has outgrown [`SEQUENCE_LIMIT`]: it is read to its
    /// end so its terminator is not taken for a bell, and then thrown away.
    too_long: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            state: State::Plain,
            sequence: Vec::new(),
            too_long: false,
        }
    }
}

impl Scanner {
    /// Read one piece of output, answering with every request for attention in it, in order.
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Asked> {
        let mut found = Vec::new();
        for &byte in chunk {
            match self.state {
                State::Plain => match byte {
                    0x07 => found.push(Asked::Bell),
                    0x1b => self.state = State::Escape,
                    _ => {}
                },
                State::Escape => {
                    self.state = if byte == b']' {
                        self.sequence.clear();
                        self.too_long = false;
                        State::Osc
                    } else {
                        State::Plain
                    };
                }
                State::Osc => match byte {
                    0x07 => self.finish(&mut found),
                    0x1b => self.state = State::OscEscape,
                    _ => {
                        if self.sequence.len() < SEQUENCE_LIMIT {
                            self.sequence.push(byte);
                        } else {
                            // Too long to be a notification - an image, most likely. Read to
                            // its end all the same, so its terminator is not heard as a bell.
                            self.sequence.clear();
                            self.too_long = true;
                        }
                    }
                },
                State::OscEscape => {
                    if byte == b'\\' {
                        self.finish(&mut found);
                    } else {
                        self.sequence.clear();
                        self.state = State::Plain;
                    }
                }
            }
        }
        found
    }

    fn finish(&mut self, found: &mut Vec<Asked>) {
        if !self.too_long
            && let Some(message) = notification_in(&self.sequence)
        {
            found.push(Asked::Notification(message));
        }
        self.sequence.clear();
        self.state = State::Plain;
    }
}

/// The message a complete OSC carries, for the three that are notifications.
fn notification_in(sequence: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(sequence);
    let (number, rest) = text.split_once(';')?;
    let message = match number {
        "9" => {
            // ConEmu overloads OSC 9 with `9 ; <digit> ; …` for progress bars and the like;
            // a message beginning with a digit and a semicolon is one of those, not a line
            // for a person.
            let is_conemu = rest.split_once(';').is_some_and(|(first, _)| {
                !first.is_empty() && first.bytes().all(|b| b.is_ascii_digit())
            });
            if is_conemu {
                return None;
            }
            rest.to_string()
        }
        "777" => {
            let (kind, rest) = rest.split_once(';')?;
            if kind != "notify" {
                return None;
            }
            match rest.split_once(';') {
                Some((title, body)) if !title.is_empty() && !body.is_empty() => {
                    format!("{title}: {body}")
                }
                Some((title, body)) => format!("{title}{body}"),
                None => rest.to_string(),
            }
        }
        "99" => kitty_notification_in(rest)?,
        _ => return None,
    };
    let message = readable(&message);
    (!message.is_empty()).then_some(message)
}

/// kitty's protocol: `key=value:key=value;payload`. The payload is the body unless `p=` says
/// it is the title or something else, and it is base64 when `e=1`.
fn kitty_notification_in(rest: &str) -> Option<String> {
    let (metadata, payload) = rest.split_once(';').unwrap_or(("", rest));
    let mut payload_type = "body";
    let mut encoded = false;
    for pair in metadata.split(':') {
        match pair.split_once('=') {
            Some(("p", value)) => payload_type = value,
            Some(("e", "1")) => encoded = true,
            _ => {}
        }
    }
    if payload_type != "body" && payload_type != "title" {
        return None;
    }
    if encoded {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        return Some(String::from_utf8_lossy(&bytes).into_owned());
    }
    Some(payload.to_string())
}

/// One line, control characters out, cut to [`MESSAGE_LIMIT`].
fn readable(message: &str) -> String {
    let mut out = String::new();
    let mut last_was_space = true;
    for character in message.chars() {
        let character = if character.is_control() || character.is_whitespace() {
            ' '
        } else {
            character
        };
        if character == ' ' && last_was_space {
            continue;
        }
        last_was_space = character == ' ';
        out.push(character);
        if out.chars().count() >= MESSAGE_LIMIT {
            break;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(bytes: &[u8]) -> Vec<Asked> {
        Scanner::default().feed(bytes)
    }

    #[test]
    fn a_bare_bell_asks_for_a_look() {
        assert_eq!(scan(b"done\x07\n"), vec![Asked::Bell]);
    }

    #[test]
    fn the_bell_that_ends_a_notification_is_not_a_second_ask() {
        assert_eq!(
            scan(b"\x1b]9;Permission needs input\x07"),
            vec![Asked::Notification("Permission needs input".into())]
        );
    }

    #[test]
    fn every_notification_protocol_is_read() {
        assert_eq!(
            scan(b"\x1b]9;claude: Waiting for your input\x1b\\"),
            vec![Asked::Notification("claude: Waiting for your input".into())]
        );
        assert_eq!(
            scan(b"\x1b]777;notify;opencode;Question needs input\x1b\\"),
            vec![Asked::Notification("opencode: Question needs input".into())]
        );
        // kitty: the body in the clear, then base64 with `e=1`.
        assert_eq!(
            scan(b"\x1b]99;i=1:d=1;Session done\x1b\\"),
            vec![Asked::Notification("Session done".into())]
        );
        assert_eq!(
            scan(b"\x1b]99;i=opentui-1:p=body:e=1:d=1;U2Vzc2lvbiBkb25l\x1b\\"),
            vec![Asked::Notification("Session done".into())]
        );
    }

    #[test]
    fn a_sequence_split_across_reads_is_one_sequence() {
        let mut scanner = Scanner::default();
        assert!(scanner.feed(b"\x1b]9;Perm").is_empty());
        assert!(scanner.feed(b"ission needs in").is_empty());
        assert_eq!(
            scanner.feed(b"put\x07"),
            vec![Asked::Notification("Permission needs input".into())]
        );
        // And the ESC of an `ESC \` terminator can end one read.
        assert!(scanner.feed(b"\x1b]9;again\x1b").is_empty());
        assert_eq!(
            scanner.feed(b"\\"),
            vec![Asked::Notification("again".into())]
        );
    }

    #[test]
    fn other_sequences_are_left_alone() {
        // A title change, a hyperlink, a colour query, a CSI: none of them is an ask.
        assert!(scan(b"\x1b]0;a title\x07\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\\x1b[31mred\x1b[0m\x1b]11;?\x07").is_empty());
        // ConEmu's progress report rides on OSC 9 and is not a message.
        assert!(scan(b"\x1b]9;4;1;50\x07").is_empty());
        assert!(scan(b"\x1b]777;something-else;x\x07").is_empty());
    }

    #[test]
    fn a_message_is_one_readable_line() {
        assert_eq!(
            scan(b"\x1b]9;  two\n\tlines\x01 \x7f here \x07"),
            vec![Asked::Notification("two lines here".into())]
        );
        let long = format!("\x1b]9;{}\x07", "x".repeat(1000));
        let found = scan(long.as_bytes());
        let [Asked::Notification(message)] = found.as_slice() else {
            panic!("expected one notification");
        };
        assert_eq!(message.len(), MESSAGE_LIMIT);
    }

    #[test]
    fn a_sequence_too_long_to_be_a_message_is_given_up_on() {
        let mut bytes = b"\x1b]1337;File=inline=1:".to_vec();
        bytes.extend(std::iter::repeat_n(b'A', SEQUENCE_LIMIT + 100));
        bytes.extend(b"\x07plain\x07");
        // The image's own terminator goes with it; the bell after the plain text is heard.
        assert_eq!(scan(&bytes), vec![Asked::Bell]);
    }
}
