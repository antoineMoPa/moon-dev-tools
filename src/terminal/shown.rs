//! What a shell is showing, read as text: `moon agent view`, which has no tab to look at.
//!
//! The server keeps what a shell printed as bytes - see [`Scrollback`](super::Scrollback) -
//! and a tab turns them into a grid by running them through Ghostty's emulator. This does the
//! same with no tab: an emulator the size of the shell's pty, fed everything kept, and read
//! back. So it shows what a tab opened on the shell now would, which is as far back as the
//! kept bytes go and no further.
//!
//! Nothing here answers the questions a program asks of its terminal, which the bytes are
//! full of: they were answered when they were asked, or will be by the tab that attaches.

use anyhow::{Context, Result, anyhow};
use libghostty_vt::{
    Terminal as Emulator, TerminalOptions,
    render::{CellIterator, RenderState, RowIterator},
    selection::FormatOptions,
};
use serde::{Deserialize, Serialize};

use super::{TerminalRegistry, TerminalSession};

/// How many rows that have scrolled off the top the emulator keeps, which is as many as a
/// tab's does.
const KEPT_ROWS: usize = 10_000;

/// Which part of what a shell is showing is wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Shown {
    /// The screen: the rows a tab on the shell would be drawing.
    Screen,
    /// This many rows from the bottom of everything it has printed, the screen and what has
    /// scrolled off it alike - for an agent, the end of the conversation.
    LastRows(usize),
}

impl TerminalRegistry {
    /// What a shell is showing, one line per row, with the blank rows under the last written
    /// one left off.
    pub(crate) fn shown(&self, terminal_id: &str, wanted: Shown) -> Result<String> {
        self.get(terminal_id)
            .ok_or_else(|| anyhow!("unknown terminal {terminal_id}"))?
            .shown(wanted)
    }
}

impl TerminalSession {
    /// What this shell is showing - see [`TerminalRegistry::shown`].
    pub(super) fn shown(&self, wanted: Shown) -> Result<String> {
        let size = self
            .master
            .lock()
            .unwrap()
            .get_size()
            .context("failed to read the size of the shell")?;
        let printed = self.scrollback.lock().unwrap().replay();
        text_of(&printed, size.cols, size.rows, wanted)
    }
}

/// What a grid of this size shows once a program has printed this into it.
fn text_of(printed: &[u8], cols: u16, rows: u16, wanted: Shown) -> Result<String> {
    let mut emulator = Emulator::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: KEPT_ROWS,
    })
    .map_err(|error| anyhow!("failed to create a terminal: {error}"))?;
    emulator.vt_write(printed);

    let rows = match wanted {
        Shown::Screen => written_rows(screen_rows(&emulator)?),
        Shown::LastRows(count) => {
            let mut rows = written_rows(every_row(&emulator)?);
            rows.drain(..rows.len().saturating_sub(count));
            rows
        }
    };
    Ok(rows.into_iter().map(|row| row + "\n").collect())
}

/// The rows down to the last one with anything written on it.
fn written_rows(mut rows: Vec<String>) -> Vec<String> {
    let written = rows
        .iter()
        .rposition(|row| !row.is_empty())
        .map_or(0, |last| last + 1);
    rows.truncate(written);
    rows
}

/// The rows of the screen, top to bottom, each without its trailing blanks.
///
/// Read off the emulator's own grid, the way a tab paints from it, so a row is a row of the
/// screen however much of what the program printed has scrolled off above it.
fn screen_rows(emulator: &Emulator<'_, '_>) -> Result<Vec<String>> {
    let mut render_state =
        RenderState::new().map_err(|error| anyhow!("failed to create a render state: {error}"))?;
    let mut row_iterator =
        RowIterator::new().map_err(|error| anyhow!("failed to create a row iterator: {error}"))?;
    let mut cell_iterator = CellIterator::new()
        .map_err(|error| anyhow!("failed to create a cell iterator: {error}"))?;

    let snapshot = render_state
        .update(emulator)
        .map_err(|error| anyhow!("failed to read the screen: {error}"))?;
    let mut rows = row_iterator
        .update(&snapshot)
        .map_err(|error| anyhow!("failed to walk the rows: {error}"))?;

    let mut screen = Vec::new();
    let mut cell_text = String::new();
    while let Some(row) = rows.next() {
        let mut cells = cell_iterator
            .update(row)
            .map_err(|error| anyhow!("failed to walk the cells: {error}"))?;
        let mut line = String::new();
        while let Some(cell) = cells.next() {
            cell_text.clear();
            if cell.graphemes_len().unwrap_or(0) > 0 {
                let _ = cell.graphemes_utf8(&mut cell_text);
            }
            line.push_str(if cell_text.is_empty() {
                " "
            } else {
                &cell_text
            });
        }
        screen.push(line.trim_end().to_string());
    }
    Ok(screen)
}

/// Every row the emulator holds, the ones that scrolled off the top first, each without its
/// trailing blanks. A line the grid wrapped stays as the rows it was wrapped into: a row
/// here is a row of the shell, which is what a count of them is a count of.
fn every_row(emulator: &Emulator<'_, '_>) -> Result<Vec<String>> {
    let Some(everything) = emulator
        .select_all()
        .map_err(|error| anyhow!("failed to select what the shell holds: {error}"))?
    else {
        // Nothing has been written on it.
        return Ok(Vec::new());
    };
    let options = FormatOptions::new()
        .with_unwrap(false)
        .with_trim(true)
        .with_selection(&everything);
    let text = emulator
        .format_selection_alloc(None, options)
        .map_err(|error| anyhow!("failed to read what the shell holds: {error}"))?
        .context("the shell holds something and it read as nothing")?;
    let text = String::from_utf8(text.to_vec()).context("the shell holds what is not text")?;
    Ok(text.lines().map(|row| row.trim_end().to_string()).collect())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use super::*;

    /// The whole of it against a real pty: what the program in a shell printed is what the
    /// shell is said to be showing, with what it painted the text in left out.
    #[cfg(unix)]
    #[test]
    fn a_shell_is_read_as_what_the_program_in_it_printed() {
        let registry = Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))));
        let terminal_id = crate::terminal::tests::spawn_fake_claude(
            &registry,
            "#!/bin/sh\nprintf '\\033[32mhello from the agent\\033[0m\\n> '\nread -r line\n",
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut screen = String::new();
        while Instant::now() < deadline && !screen.contains('>') {
            std::thread::sleep(Duration::from_millis(20));
            screen = registry
                .shown(&terminal_id, Shown::Screen)
                .expect("expected the screen");
        }
        assert_eq!(screen, "hello from the agent\n>\n");
        assert_eq!(
            registry
                .shown(&terminal_id, Shown::LastRows(1))
                .expect("expected the last row"),
            ">\n"
        );
        assert!(registry.shown("terminal-nobody-0", Shown::Screen).is_err());
        registry.remove(&terminal_id);
    }

    #[test]
    fn the_screen_is_read_as_the_rows_a_tab_would_draw() {
        // Red, and a title: neither is text on the grid.
        let printed = b"\x1b]0;a title\x07hello\r\n\x1b[31mworld\x1b[0m\r\n";

        assert_eq!(
            text_of(printed, 40, 6, Shown::Screen).expect("expected the screen"),
            "hello\nworld\n"
        );
    }

    /// A program that moves the cursor about - an agent's interface - is read as where its
    /// text ended up, not in the order it was printed.
    #[test]
    fn text_is_read_where_the_cursor_put_it() {
        let printed = b"first\r\nsecond\x1b[1;1Hthird";

        assert_eq!(
            text_of(printed, 40, 6, Shown::Screen).expect("expected the screen"),
            "third\nsecond\n"
        );
    }

    #[test]
    fn the_screen_leaves_out_what_has_scrolled_off_it() {
        let printed: String = (0..10).map(|row| format!("row {row}\r\n")).collect();

        assert_eq!(
            text_of(printed.as_bytes(), 40, 4, Shown::Screen).expect("expected the screen"),
            "row 7\nrow 8\nrow 9\n"
        );
    }

    #[test]
    fn the_last_rows_reach_back_past_the_top_of_the_screen() {
        let printed: String = (0..10).map(|row| format!("row {row}\r\n")).collect();

        assert_eq!(
            text_of(printed.as_bytes(), 40, 4, Shown::LastRows(6)).expect("expected the rows"),
            "row 4\nrow 5\nrow 6\nrow 7\nrow 8\nrow 9\n"
        );
        // More than there is, is all there is.
        assert_eq!(
            text_of(b"only\r\n", 40, 4, Shown::LastRows(50)).expect("expected the rows"),
            "only\n"
        );
    }

    #[test]
    fn a_shell_that_has_printed_nothing_shows_nothing() {
        for wanted in [Shown::Screen, Shown::LastRows(5)] {
            assert_eq!(text_of(b"", 40, 4, wanted).expect("expected no rows"), "");
        }
    }
}
