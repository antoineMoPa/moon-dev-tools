//! Terminal output - bounded history and exact incremental resume cursors.
use super::{SCROLLBACK_LIMIT, answered_queries};
use anyhow::{Result, ensure};

#[derive(Clone, Debug)]
pub(super) struct OutputChunk {
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Scrollback {
    chunks: Vec<PrintedChunk>,
    bytes: usize,
    sequence: u64,
}
struct PrintedChunk {
    output: OutputChunk,
    seen_live: bool,
}
impl Scrollback {
    pub(super) fn push(&mut self, bytes: &[u8], seen_live: bool) -> OutputChunk {
        self.sequence += 1;
        let output = OutputChunk {
            sequence: self.sequence,
            bytes: bytes.to_vec(),
        };
        self.chunks.push(PrintedChunk {
            output: output.clone(),
            seen_live,
        });
        self.bytes += bytes.len();
        while self.bytes > SCROLLBACK_LIMIT && self.chunks.len() > 1 {
            self.bytes -= self.chunks.remove(0).output.bytes.len();
        }
        output
    }
    pub(super) fn replay(&self) -> Vec<u8> {
        let mut replay = Vec::with_capacity(self.bytes);
        for stretch in self
            .chunks
            .chunk_by(|before, after| before.seen_live == after.seen_live)
        {
            let printed: Vec<u8> = stretch
                .iter()
                .flat_map(|chunk| chunk.output.bytes.iter().copied())
                .collect();
            if stretch[0].seen_live {
                replay.extend(answered_queries::without_answered_queries(&printed));
            } else {
                replay.extend(printed);
            }
        }
        replay
    }
    pub(super) fn attachment(&self, after: Option<u64>) -> Result<OutputChunk> {
        let bytes = match after {
            None => self.replay(),
            Some(after) => {
                ensure!(after <= self.sequence, "invalid terminal output cursor");
                let oldest = self
                    .chunks
                    .first()
                    .map_or(self.sequence + 1, |chunk| chunk.output.sequence);
                ensure!(
                    after >= oldest - 1,
                    "Terminal output exceeded retained history while inactive. Reconnect to this shell."
                );
                // The existing emulator already consumed everything through `after`. Keep
                // its parser/modes and deliver ALL remaining bytes, including queries a
                // subscribed socket may never have delivered before the tab lost focus.
                self.chunks
                    .iter()
                    .filter(|chunk| chunk.output.sequence > after)
                    .flat_map(|chunk| chunk.output.bytes.iter().copied())
                    .collect()
            }
        };
        Ok(OutputChunk {
            sequence: self.sequence,
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_is_incremental_and_keeps_unreceived_queries() {
        let mut history = Scrollback::default();
        let mode = history.push(b"\x1b[?1049h\x1b[?1h\x1b[?2004h", true);
        history.push(b"new output\x1b[6n", true);
        assert_eq!(
            history.attachment(Some(mode.sequence)).unwrap().bytes,
            b"new output\x1b[6n"
        );
        assert!(
            !history
                .attachment(None)
                .unwrap()
                .bytes
                .ends_with(b"\x1b[6n")
        );
    }
    #[test]
    fn modes_evicted_before_cursor_are_not_reset_or_replayed() {
        let mut history = Scrollback::default();
        use libghostty_vt::{Terminal, TerminalOptions, terminal::Mode};
        let mut emulator = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .unwrap();
        let mode = history.push(b"\x1b[?1049h\x1b[?1h\x1b[?2004h", true);
        emulator.vt_write(&mode.bytes);
        let flood = history.push(&vec![b'x'; SCROLLBACK_LIMIT], true);
        emulator.vt_write(&flood.bytes);
        let cursor = flood.sequence;
        history.push(b"delta", false);
        emulator.vt_write(&history.attachment(Some(cursor)).unwrap().bytes);
        for mode in [Mode::ALT_SCREEN_SAVE, Mode::DECCKM, Mode::BRACKETED_PASTE] {
            assert!(
                emulator.mode(mode).unwrap(),
                "mode {mode:?} lost during resume"
            );
        }
        assert_eq!(history.attachment(Some(cursor)).unwrap().bytes, b"delta");
        assert_eq!(history.attachment(Some(cursor + 1)).unwrap().bytes, b"");
    }
    #[test]
    fn missing_history_is_reported_instead_of_replaying_a_truncated_stream() {
        let mut history = Scrollback::default();
        let cursor = history.push(b"before pause", true).sequence;
        history.push(&vec![b'x'; SCROLLBACK_LIMIT], false);
        history.push(b"latest", false);
        assert!(
            history
                .attachment(Some(cursor))
                .unwrap_err()
                .to_string()
                .contains("retained history")
        );
    }
}
