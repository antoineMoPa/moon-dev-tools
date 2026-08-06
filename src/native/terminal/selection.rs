//! Selecting text with the pointer, and the clipboard either side of it.
//!
//! Ghostty owns the hard parts: a gesture state machine that turns presses, drags and
//! releases into selections (including the double-click word and triple-click line that
//! come with click counts), and the formatter that turns an installed selection back into
//! text. This file is the wiring between that machine and egui's pointer.

use std::time::Duration;

use libghostty_vt::{
    Terminal,
    paste,
    selection::{
        FormatOptions,
        gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent},
    },
    terminal::{Mode, Point, PointCoordinate},
};

/// How close together, and how near each other, two presses have to be to count as a
/// double click. These are the platform's own conventions rather than anything terminal.
const REPEAT_INTERVAL: Duration = Duration::from_millis(400);
const REPEAT_DISTANCE: f64 = 4.0;

/// A cell of the grid, in viewport coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cell {
    pub(crate) x: u16,
    pub(crate) y: u16,
}

/// The pointer's side of a selection: Ghostty's gesture machine and the event objects it
/// is driven with, which are reused rather than rebuilt per press.
pub(crate) struct Pointer {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    release: ReleaseEvent<'static>,
    /// Set between a press inside the grid and the release that ends it.
    pub(crate) dragging: bool,
}

impl Pointer {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Ok(Self {
            gesture: Gesture::new()
                .map_err(|error| anyhow::anyhow!("failed to create a selection gesture: {error}"))?,
            press: PressEvent::new()
                .map_err(|error| anyhow::anyhow!("failed to create a press event: {error}"))?,
            drag: DragEvent::new()
                .map_err(|error| anyhow::anyhow!("failed to create a drag event: {error}"))?,
            release: ReleaseEvent::new()
                .map_err(|error| anyhow::anyhow!("failed to create a release event: {error}"))?,
            dragging: false,
        })
    }

    /// A press at a cell. One click clears the selection, two select the word under it,
    /// three the line — all of which the gesture machine decides from the click count.
    ///
    /// `at` is the pointer in surface pixels, which is what the repeat-click distance and
    /// the click count are measured in.
    pub(crate) fn press(
        &mut self,
        terminal: &Terminal<'_, '_>,
        cell: Cell,
        at: (f32, f32),
        now: Duration,
    ) -> anyhow::Result<()> {
        let grid_ref = grid_ref_at(terminal, cell)?;
        self.press
            .set_position(f64::from(at.0), f64::from(at.1))
            .and_then(|event| event.set_time(now))
            .and_then(|event| event.set_repeat_interval(REPEAT_INTERVAL))
            .and_then(|event| event.set_repeat_distance(REPEAT_DISTANCE))
            .map_err(|error| anyhow::anyhow!("failed to describe the press: {error}"))?;

        let selection = self
            .press
            .apply(&mut self.gesture, terminal, grid_ref)
            .map_err(|error| anyhow::anyhow!("failed to apply the press: {error}"))?;
        terminal
            .set_selection(selection.as_ref())
            .map_err(|error| anyhow::anyhow!("failed to set the selection: {error}"))?;

        self.dragging = true;
        Ok(())
    }

    /// The pointer moving with the button down, which is what grows a selection.
    pub(crate) fn drag(
        &mut self,
        terminal: &Terminal<'_, '_>,
        cell: Cell,
        at: (f32, f32),
        geometry: Geometry,
        rectangle: bool,
    ) -> anyhow::Result<()> {
        let grid_ref = grid_ref_at(terminal, cell)?;
        self.drag
            .set_position(f64::from(at.0), f64::from(at.1))
            .and_then(|event| event.set_rectangle(rectangle))
            .map_err(|error| anyhow::anyhow!("failed to describe the drag: {error}"))?;

        let selection = self
            .drag
            .apply(&mut self.gesture, terminal, grid_ref, geometry)
            .map_err(|error| anyhow::anyhow!("failed to apply the drag: {error}"))?;
        // A drag that resolves to nothing leaves what was already selected alone, so a
        // stray pixel of movement cannot wipe a selection out.
        if let Some(selection) = selection {
            terminal
                .set_selection(Some(&selection))
                .map_err(|error| anyhow::anyhow!("failed to set the selection: {error}"))?;
        }
        Ok(())
    }

    pub(crate) fn release(&mut self, terminal: &Terminal<'_, '_>) -> anyhow::Result<()> {
        self.dragging = false;
        self.release
            .apply(&mut self.gesture, terminal, None)
            .map_err(|error| anyhow::anyhow!("failed to apply the release: {error}"))
    }

    /// Whether the gesture that just ended moved at all. A press and release on one spot is
    /// a click, which is what opens a link rather than selecting anything.
    pub(crate) fn dragged(&self, terminal: &Terminal<'_, '_>) -> bool {
        self.gesture.dragged(terminal).unwrap_or(false)
    }
}

fn grid_ref_at<'t>(
    terminal: &'t Terminal<'_, '_>,
    cell: Cell,
) -> anyhow::Result<libghostty_vt::screen::GridRef<'t>> {
    terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x: cell.x,
            y: u32::from(cell.y),
        }))
        .map_err(|error| anyhow::anyhow!("failed to read the cell under the pointer: {error}"))
}

/// The text of whatever is selected, as the clipboard should receive it: soft-wrapped lines
/// joined back up and trailing blanks dropped, which is what every other terminal copies.
pub(crate) fn selected_text(terminal: &Terminal<'_, '_>) -> Option<String> {
    let options = FormatOptions::new().with_unwrap(true).with_trim(true);
    let bytes = terminal.format_selection_alloc(None, options).ok()??;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Encode pasted text the way the running program expects it: wrapped in bracketed-paste
/// markers when it asked for them, and with the control bytes that could inject a command
/// stripped either way.
pub(crate) fn encode_paste(terminal: &Terminal<'_, '_>, text: &str) -> anyhow::Result<Vec<u8>> {
    let bracketed = terminal.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
    let mut data = text.as_bytes().to_vec();
    // The markers either side are 6 bytes each; the encoder says so itself if this is wrong.
    let mut out = vec![0u8; data.len() + 16];

    match paste::encode(&mut data, bracketed, &mut out) {
        Ok(written) => {
            out.truncate(written);
            Ok(out)
        }
        Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
            let mut data = text.as_bytes().to_vec();
            // The encoder reports what it needed, but not every path fills that in, so this
            // also grows by enough that a second failure is not possible.
            let mut out = vec![0u8; required.max(text.len() * 2 + 64)];
            let written = paste::encode(&mut data, bracketed, &mut out)
                .map_err(|error| anyhow::anyhow!("failed to encode the paste: {error}"))?;
            out.truncate(written);
            Ok(out)
        }
        Err(error) => Err(anyhow::anyhow!("failed to encode the paste: {error}")),
    }
}

/// The grid geometry a drag is measured against: how wide the grid is and where it sits in
/// the pane, so Ghostty can work out what a pointer past the last column means.
pub(crate) fn geometry(cols: u16, cell_width: f32, padding_left: f32, height: f32) -> Geometry {
    Geometry {
        columns: u32::from(cols).max(1),
        cell_width: (cell_width.round() as u32).max(1),
        padding_left: padding_left.round().max(0.0) as u32,
        screen_height: (height.round() as u32).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    const CELL_WIDTH: f32 = 8.0;
    const CELL_HEIGHT: f32 = 16.0;

    fn terminal_showing(text: &str) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 40,
            rows: 5,
            max_scrollback: 100,
        })
        .expect("expected a terminal");
        terminal.vt_write(text.as_bytes());
        terminal
    }

    /// A point some fraction of the way across a cell, in the surface pixels the gesture
    /// measures in. The fraction matters: a cell joins the selection only once the pointer
    /// is over more of it than not, which is the rule every other terminal follows.
    fn at(cell: Cell, across: f32) -> (f32, f32) {
        (
            f32::from(cell.x) * CELL_WIDTH + CELL_WIDTH * across,
            f32::from(cell.y) * CELL_HEIGHT + CELL_HEIGHT / 2.0,
        )
    }

    fn cell(x: u16, y: u16) -> Cell {
        Cell { x, y }
    }

    /// Sweep from one cell to another: press against the near edge of the first cell and let
    /// go against the far edge of the last, which is what the hand does.
    fn drag_across(terminal: &Terminal<'_, '_>, from: Cell, to: Cell) -> Pointer {
        let forward = (to.y, to.x) >= (from.y, from.x);
        let (from_at, to_at) = if forward {
            (at(from, 0.25), at(to, 0.75))
        } else {
            (at(from, 0.75), at(to, 0.25))
        };

        let mut pointer = Pointer::new().expect("expected a gesture");
        pointer
            .press(terminal, from, from_at, Duration::from_secs(1))
            .expect("expected the press to land");
        pointer
            .drag(
                terminal,
                to,
                to_at,
                geometry(40, CELL_WIDTH, 0.0, CELL_HEIGHT * 5.0),
                false,
            )
            .expect("expected the drag to land");
        pointer.release(terminal).expect("expected the release");
        pointer
    }

    #[test]
    fn dragging_across_a_line_selects_what_the_pointer_swept() {
        let terminal = terminal_showing("hello world");
        drag_across(&terminal, cell(0, 0), cell(4, 0));

        assert_eq!(selected_text(&terminal).as_deref(), Some("hello"));
    }

    #[test]
    fn a_selection_can_run_backwards_and_over_more_than_one_row() {
        let terminal = terminal_showing("first line\r\nsecond line");
        // Started on the second row and dragged back up to the first.
        drag_across(&terminal, cell(5, 1), cell(0, 0));

        let selected = selected_text(&terminal).expect("expected a selection");
        assert!(selected.starts_with("first line"), "got {selected:?}");
        assert!(
            selected.contains("second"),
            "a selection that started below has to reach back over the row break, got {selected:?}"
        );
    }

    /// A press and release on one spot is a click. It clears any selection rather than
    /// making a one-cell one, which is what lets a click through to open a link.
    #[test]
    fn a_click_that_never_moved_selects_nothing() {
        let terminal = terminal_showing("hello world");
        drag_across(&terminal, cell(0, 0), cell(4, 0));
        assert!(selected_text(&terminal).is_some(), "something is selected");

        let mut pointer = Pointer::new().expect("expected a gesture");
        pointer
            .press(&terminal, cell(2, 0), at(cell(2, 0), 0.5), Duration::from_secs(2))
            .expect("expected the press to land");
        assert!(!pointer.dragged(&terminal), "the pointer never moved");
        pointer.release(&terminal).expect("expected the release");

        assert_eq!(selected_text(&terminal), None, "the click cleared it");
    }

    #[test]
    fn a_drag_is_recognised_as_one() {
        let terminal = terminal_showing("hello world");
        let pointer = drag_across(&terminal, cell(0, 0), cell(4, 0));

        assert!(pointer.dragged(&terminal), "the pointer swept four cells");
        assert!(!pointer.dragging, "and the release ended the gesture");
    }

    #[test]
    fn nothing_selected_means_nothing_to_copy() {
        let terminal = terminal_showing("hello world");

        assert_eq!(selected_text(&terminal), None);
    }

    /// A program that turned bracketed paste on is told where the paste begins and ends, so
    /// it can refuse to run it. One that did not gets carriage returns instead of newlines,
    /// which is what a keyboard would have sent.
    #[test]
    fn paste_is_bracketed_only_when_the_program_asked_for_it() {
        let plain = terminal_showing("");
        let encoded = encode_paste(&plain, "one\ntwo").expect("expected an encoding");
        assert_eq!(encoded, b"one\rtwo");

        let bracketed = terminal_showing("\x1b[?2004h");
        let encoded = encode_paste(&bracketed, "one\ntwo").expect("expected an encoding");
        assert_eq!(encoded, b"\x1b[200~one\ntwo\x1b[201~");
    }

    /// A paste far bigger than the first guess at a buffer still goes through whole.
    #[test]
    fn a_long_paste_is_not_truncated() {
        let terminal = terminal_showing("");
        let long = "x".repeat(9_000);
        let encoded = encode_paste(&terminal, &long).expect("expected an encoding");

        assert_eq!(encoded.len(), long.len());
    }
}
