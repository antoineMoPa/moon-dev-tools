//! The envelope every language server message travels in: a `Content-Length` header, a
//! blank line, and that many bytes of JSON.
//!
//! Bytes arrive from a pipe in whatever sizes the kernel hands over - half a header, three
//! messages at once - so decoding is a buffer that is fed reads and asked for whole
//! messages, rather than anything that reads from the pipe itself. That is also what makes
//! it testable without a server.

const HEADER_END: &[u8] = b"\r\n\r\n";
const CONTENT_LENGTH: &str = "content-length:";

/// One message, wrapped for the wire.
pub(crate) fn frame(body: &str) -> Vec<u8> {
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body.as_bytes());
    framed
}

/// What has arrived from the server and not yet been read out as messages.
#[derive(Default)]
pub(crate) struct Frames {
    buffer: Vec<u8>,
}

impl Frames {
    /// Take what one read off the pipe gave.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole message, if a whole one has arrived. `None` means the rest of it is
    /// still on its way.
    ///
    /// A header this cannot make sense of is dropped along with the byte it starts on,
    /// which lets the stream resynchronise rather than stalling forever on a server that
    /// printed something that is not a message.
    pub(crate) fn next_message(&mut self) -> Option<String> {
        loop {
            let header_end = find(&self.buffer, HEADER_END)?;
            let header = String::from_utf8_lossy(&self.buffer[..header_end]).to_string();
            let Some(length) = content_length(&header) else {
                self.buffer.drain(..1);
                continue;
            };

            let body_start = header_end + HEADER_END.len();
            if self.buffer.len() < body_start + length {
                return None;
            }
            let body: Vec<u8> = self.buffer.drain(..body_start + length).collect();
            return Some(String::from_utf8_lossy(&body[body_start..]).to_string());
        }
    }
}

/// The length off a header block. Header names are case-insensitive, and a server may send
/// a `Content-Type` line beside the length.
fn content_length(header: &str) -> Option<usize> {
    header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            format!("{}:", name.trim().to_ascii_lowercase())
                .eq(CONTENT_LENGTH)
                .then_some(value)
        })?
        .trim()
        .parse()
        .ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
