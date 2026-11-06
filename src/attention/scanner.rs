//! Picking the requests for attention out of a shell's output as it comes.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;

use super::Asked;
use crate::api::display::CLIPBOARD_LIMIT;

/// The most of a message that is kept. A notification is a line, not a document.
const MESSAGE_LIMIT: usize = 240;
/// The most of a sequence that is read before it is given up on as not a notification: a
/// program painting an image through an OSC would otherwise be buffered whole.
const SEQUENCE_LIMIT: usize = 4096;
/// The most of an OSC 52 that is read: its text is base64, four characters for every three
/// bytes, plus the `52 ; c ;` in front. A copy of more than [`CLIPBOARD_LIMIT`] is given up on.
const CLIPBOARD_SEQUENCE_LIMIT: usize = CLIPBOARD_LIMIT.div_ceil(3) * 4 + 16;
/// What an OSC 52 begins with: the clipboard, and the selection it is about.
const CLIPBOARD_PREFIX: &[u8] = b"52;";

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

/// What a shell's output asked for, as it was read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Heard {
    /// A request for a person's attention - see [`Asked`].
    Asked(Asked),
    /// Text a program copied to the clipboard with OSC 52, decoded. The window puts it on its
    /// own clipboard, which is where a copy from the terminal goes.
    Copied(String),
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
/// requests for attention and the clipboard copies. A sequence split across two reads is one
/// sequence.
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
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Heard> {
        let mut found = Vec::new();
        for &byte in chunk {
            match self.state {
                State::Plain => match byte {
                    0x07 => found.push(Heard::Asked(Asked::Bell)),
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
                        if self.sequence.len() < sequence_limit(&self.sequence) {
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

    fn finish(&mut self, found: &mut Vec<Heard>) {
        if !self.too_long
            && let Some(heard) = heard_in(&self.sequence)
        {
            found.push(heard);
        }
        self.sequence.clear();
        self.state = State::Plain;
    }
}

/// How much of an OSC is kept while it is read: an OSC 52 needs room for its whole copy.
fn sequence_limit(sequence: &[u8]) -> usize {
    if sequence.starts_with(CLIPBOARD_PREFIX) {
        CLIPBOARD_SEQUENCE_LIMIT
    } else {
        SEQUENCE_LIMIT
    }
}

/// What a complete OSC asks for, if it is one of the ones moon reads.
fn heard_in(sequence: &[u8]) -> Option<Heard> {
    if sequence.starts_with(CLIPBOARD_PREFIX) {
        return clipboard_in(&sequence[CLIPBOARD_PREFIX.len()..]).map(Heard::Copied);
    }
    notification_in(sequence).map(|message| Heard::Asked(Asked::Notification(message)))
}

/// The text an OSC 52 copies, from what follows `52 ;`: `Pc ; base64`. Only a copy to the
/// clipboard (`c`, or no selection at all) is taken. A query (`?`) is never answered, so no
/// program can read the clipboard through moon. Invalid base64, text that is not UTF-8, empty
/// text and text over [`CLIPBOARD_LIMIT`] are all ignored.
fn clipboard_in(rest: &[u8]) -> Option<String> {
    let separator = rest.iter().position(|&byte| byte == b';')?;
    let (selection, data) = (&rest[..separator], &rest[separator + 1..]);
    if !selection.is_empty() && !selection.contains(&b'c') {
        return None;
    }
    if data == b"?" {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    if bytes.len() > CLIPBOARD_LIMIT {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    (!text.is_empty()).then_some(text)
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

    /// What `bytes` asks for, leaving out any copy.
    fn scan(bytes: &[u8]) -> Vec<Asked> {
        asks(Scanner::default().feed(bytes))
    }

    fn asks(heard: Vec<Heard>) -> Vec<Asked> {
        heard
            .into_iter()
            .filter_map(|heard| match heard {
                Heard::Asked(asked) => Some(asked),
                Heard::Copied(_) => None,
            })
            .collect()
    }

    /// What `bytes` copies to the clipboard, leaving out any ask.
    fn copies(bytes: &[u8]) -> Vec<String> {
        Scanner::default()
            .feed(bytes)
            .into_iter()
            .filter_map(|heard| match heard {
                Heard::Copied(text) => Some(text),
                Heard::Asked(_) => None,
            })
            .collect()
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
            asks(scanner.feed(b"put\x07")),
            vec![Asked::Notification("Permission needs input".into())]
        );
        // And the ESC of an `ESC \` terminator can end one read.
        assert!(scanner.feed(b"\x1b]9;again\x1b").is_empty());
        assert_eq!(
            asks(scanner.feed(b"\\")),
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
    fn an_osc_52_copy_is_decoded_for_the_clipboard_and_is_no_ask() {
        // `hello, world` in base64, with BEL and with `ESC \`, and an empty selection as `c`.
        assert_eq!(
            copies(b"\x1b]52;c;aGVsbG8sIHdvcmxk\x07"),
            vec!["hello, world"]
        );
        assert_eq!(
            copies(b"\x1b]52;c;aGVsbG8sIHdvcmxk\x1b\\"),
            vec!["hello, world"]
        );
        assert_eq!(
            copies(b"\x1b]52;;aGVsbG8sIHdvcmxk\x07"),
            vec!["hello, world"]
        );
        assert_eq!(
            copies(b"\x1b]52;pc;aGVsbG8sIHdvcmxk\x07"),
            vec!["hello, world"]
        );
        // Newlines are kept as they were copied: a copy is not a message.
        assert_eq!(copies(b"\x1b]52;c;YQpi\x07"), vec!["a\nb"]);
        // A copy is not a bell, and the bell after it is still heard.
        assert_eq!(
            Scanner::default().feed(b"\x1b]52;c;YQ==\x07\x07"),
            vec![Heard::Copied("a".into()), Heard::Asked(Asked::Bell)]
        );
    }

    #[test]
    fn an_osc_52_copy_split_across_reads_is_one_copy() {
        let mut scanner = Scanner::default();
        assert!(scanner.feed(b"\x1b]52;c;aGVsbG8s").is_empty());
        assert!(scanner.feed(b"IHdvcmxk\x1b").is_empty());
        assert_eq!(
            scanner.feed(b"\\"),
            vec![Heard::Copied("hello, world".into())]
        );
    }

    #[test]
    fn an_osc_52_query_is_never_answered_and_other_selections_are_ignored() {
        // A query asks for the clipboard back: nothing is copied, nothing is read.
        assert!(copies(b"\x1b]52;c;?\x07").is_empty());
        assert!(copies(b"\x1b]52;;?\x1b\\").is_empty());
        // Primary and secondary selections are not the clipboard.
        assert!(copies(b"\x1b]52;p;aGVsbG8=\x07").is_empty());
        assert!(copies(b"\x1b]52;s;aGVsbG8=\x07").is_empty());
        assert!(copies(b"\x1b]52;s0;aGVsbG8=\x07").is_empty());
    }

    #[test]
    fn an_osc_52_that_is_not_a_copy_is_ignored() {
        // Invalid base64, text that is not UTF-8, and nothing to copy.
        assert!(copies(b"\x1b]52;c;!!!!\x07").is_empty());
        assert!(copies(b"\x1b]52;c;/w==\x07").is_empty());
        assert!(copies(b"\x1b]52;c;\x07").is_empty());
        // No selection parameter at all is not an OSC 52 copy.
        assert!(copies(b"\x1b]52\x07").is_empty());
    }

    #[test]
    fn an_osc_52_copy_is_capped_at_the_clipboard_limit() {
        let at_limit = "x".repeat(CLIPBOARD_LIMIT);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&at_limit);
        let found = copies(format!("\x1b]52;c;{encoded}\x07").as_bytes());
        assert_eq!(found, vec![at_limit]);

        let over = "x".repeat(CLIPBOARD_LIMIT + 1);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&over);
        assert!(encoded.len() <= CLIPBOARD_SEQUENCE_LIMIT);
        assert!(copies(format!("\x1b]52;c;{encoded}\x07").as_bytes()).is_empty());

        // Far over the raw limit: given up on as it is read, and the copy after it still lands.
        let mut bytes = b"\x1b]52;c;".to_vec();
        bytes.extend(std::iter::repeat_n(b'A', CLIPBOARD_SEQUENCE_LIMIT + 100));
        bytes.extend(b"\x07\x1b]52;c;aGk=\x07");
        assert_eq!(copies(&bytes), vec!["hi"]);
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
