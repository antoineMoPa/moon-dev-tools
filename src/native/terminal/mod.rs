//! A terminal pane, drawn in egui on top of Ghostty's terminal emulator.
//!
//! libghostty-vt owns everything that makes a terminal a terminal: the VT parser, the
//! screen and its scrollback, reflow on resize, and input encoding. This file is the part
//! Ghostty deliberately leaves out — turning the resulting grid into pixels, and turning
//! window events into terminal events.
//!
//! The emulator is `!Send`, so a pane and its terminal both live on the UI thread. Bytes
//! from the shell arrive over a channel, from either the in-process pty or a websocket to
//! a remote server.

mod keys;

use std::sync::{Arc, mpsc};

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, TextFormat, Ui, text::LayoutJob,
    vec2,
};
use libghostty_vt::{
    Terminal, TerminalOptions,
    key::{self, Action},
    render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowIterator},
    style::RgbColor,
    terminal::ScrollViewport,
};

use crate::{
    backend::{TerminalAttachment, TerminalInput},
    native::theme::Palette,
};

/// Lines of scrollback each shell keeps. The server keeps its own byte-capped replay
/// buffer; this is what stays scrollable in the window.
const MAX_SCROLLBACK: usize = 10_000;
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 4;

pub(crate) struct TerminalPane {
    pub(crate) terminal_id: String,
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_iterator: RowIterator<'static>,
    cell_iterator: CellIterator<'static>,
    encoder: key::Encoder<'static>,
    output: mpsc::Receiver<Vec<u8>>,
    input: Arc<dyn TerminalInput>,
    cols: u16,
    rows: u16,
    /// Set once the shell is gone, so the pane says so instead of looking merely idle.
    exited: bool,
    /// Cursor blink phase, in seconds since the pane was drawn first.
    blink_clock: f32,
}

impl TerminalPane {
    pub(crate) fn new(terminal_id: String, attachment: TerminalAttachment) -> anyhow::Result<Self> {
        let cols = 80;
        let rows = 24;
        let mut terminal = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback: MAX_SCROLLBACK,
        })
        .map_err(|error| anyhow::anyhow!("failed to create a terminal: {error}"))?;

        // Programs probe the terminal on startup and wait for the answers, so the replies
        // have to go back to the shell or the pane would look hung.
        let reply_to = Arc::clone(&attachment.input);
        terminal
            .on_pty_write(move |_terminal, data| {
                let _ = reply_to.write(data);
            })
            .map_err(|error| anyhow::anyhow!("failed to install the terminal reply hook: {error}"))?;

        Ok(Self {
            terminal_id,
            terminal,
            render_state: new_render_state()?,
            row_iterator: RowIterator::new()
                .map_err(|error| anyhow::anyhow!("failed to create a row iterator: {error}"))?,
            cell_iterator: CellIterator::new()
                .map_err(|error| anyhow::anyhow!("failed to create a cell iterator: {error}"))?,
            encoder: key::Encoder::new()
                .map_err(|error| anyhow::anyhow!("failed to create a key encoder: {error}"))?,
            output: attachment.output,
            input: attachment.input,
            cols,
            rows,
            exited: false,
            blink_clock: 0.0,
        })
    }

    pub(crate) fn has_exited(&self) -> bool {
        self.exited
    }

    /// Send bytes to the shell as if they had been typed.
    #[cfg(test)]
    pub(crate) fn send(&self, bytes: &[u8]) -> anyhow::Result<()> {
        self.input.write(bytes)
    }

    /// Everything currently on screen, one line per row and trailing blanks trimmed.
    ///
    /// This reads the emulator's own grid, which is what the pane paints from, so it is how
    /// the tests check that a shell's output actually made it through the VT parser.
    #[cfg(test)]
    pub(crate) fn visible_text(&mut self) -> anyhow::Result<String> {
        let Self {
            terminal,
            render_state,
            row_iterator,
            cell_iterator,
            ..
        } = self;

        let snapshot = render_state
            .update(terminal)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let mut rows = row_iterator
            .update(&snapshot)
            .map_err(|error| anyhow::anyhow!("{error}"))?;

        let mut out = String::new();
        let mut cell_text = String::new();
        while let Some(row) = rows.next() {
            let mut cells = cell_iterator
                .update(row)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let mut line = String::new();
            while let Some(view) = cells.next() {
                cell_text.clear();
                if view.graphemes_len().unwrap_or(0) > 0 {
                    let _ = view.graphemes_utf8(&mut cell_text);
                }
                line.push_str(if cell_text.is_empty() { " " } else { &cell_text });
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }

        Ok(out)
    }

    /// Feed pending shell output into the emulator without drawing, for tests that wait on it.
    #[cfg(test)]
    pub(crate) fn pump(&mut self) -> bool {
        self.pump_output()
    }

    /// The title the running program set, for the tab strip.
    pub(crate) fn title(&self) -> Option<String> {
        self.terminal
            .title()
            .ok()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned)
    }

    /// Feed everything the shell has written since the last frame. Returns whether anything
    /// arrived, so an idle pane does not keep the window repainting.
    fn pump_output(&mut self) -> bool {
        let mut received = false;
        loop {
            match self.output.try_recv() {
                Ok(chunk) => {
                    self.terminal.vt_write(&chunk);
                    received = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.exited = true;
                    break;
                }
            }
        }
        // A local shell's output channel stays open even after the shell is gone, because this
        // pane is what holds its session alive, so the handle is asked directly.
        if self.input.has_exited() {
            self.exited = true;
        }
        received
    }

    fn resize(&mut self, cols: u16, rows: u16, cell_size: egui::Vec2) -> anyhow::Result<()> {
        if cols == self.cols && rows == self.rows {
            return Ok(());
        }
        self.cols = cols;
        self.rows = rows;
        self.terminal
            .resize(cols, rows, cell_size.x as u32, cell_size.y as u32)
            .map_err(|error| anyhow::anyhow!("failed to resize the terminal: {error}"))?;
        // The shell needs to hear about it too, so its foreground program gets SIGWINCH.
        self.input.resize(cols, rows)?;
        Ok(())
    }

    /// Draw the pane and handle its input. Returns whether it wants another frame soon.
    ///
    /// `take_focus` is set for a shell the user just opened: it starts out with the keyboard,
    /// the way it would if they had clicked into it.
    pub(crate) fn ui(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        font: FontId,
        take_focus: bool,
    ) -> bool {
        let received = self.pump_output();
        let cell_size = cell_size(ui.painter(), &font);
        let available = ui.available_size();

        let cols = ((available.x - 2.0 * PADDING) / cell_size.x).floor().max(MIN_COLS as f32) as u16;
        let rows = ((available.y - 2.0 * PADDING) / cell_size.y).floor().max(MIN_ROWS as f32) as u16;
        if let Err(error) = self.resize(cols, rows, cell_size) {
            eprintln!("[moonreview] {error}");
        }

        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        if response.clicked() || take_focus {
            response.request_focus();
        }
        let focused = response.has_focus();

        if focused {
            self.handle_input(ui);
        }
        self.handle_scroll(ui, &response, cell_size);

        self.blink_clock += ui.input(|input| input.stable_dt).min(0.1);
        let origin = response.rect.min + vec2(PADDING, PADDING);
        let repaint = match self.draw(&painter, origin, cell_size, &font, palette, focused) {
            Ok(cursor_blinking) => cursor_blinking,
            Err(error) => {
                painter.text(
                    origin,
                    Align2::LEFT_TOP,
                    format!("terminal render failed: {error}"),
                    font,
                    palette.warn,
                );
                false
            }
        };

        if self.exited {
            let banner = format!("[{} exited — close this tab]", self.terminal_id);
            painter.text(
                response.rect.left_bottom() + vec2(PADDING, -PADDING - cell_size.y),
                Align2::LEFT_TOP,
                banner,
                FontId::monospace(cell_size.y * 0.7),
                palette.muted,
            );
        }

        received || repaint
    }

    fn handle_scroll(&mut self, ui: &Ui, response: &egui::Response, cell_size: egui::Vec2) {
        if !response.hovered() {
            return;
        }
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() < 0.5 {
            return;
        }
        // Wheel notches are lines, and up on a wheel means back into the scrollback.
        let lines = (scroll / cell_size.y).round() as isize;
        if lines != 0 {
            self.terminal.scroll_viewport(ScrollViewport::Delta(-lines));
        }
    }

    /// Encode this frame's key and text events and send them to the shell.
    ///
    /// libghostty decides what the bytes are — application cursor keys, the Kitty
    /// keyboard protocol, `modifyOtherKeys` — so all that happens here is pairing each
    /// physical key press with the text the platform produced for it.
    fn handle_input(&mut self, ui: &Ui) {
        let events = ui.input(|input| input.events.clone());
        let mut pending_text: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        let mut text_index = 0usize;

        self.encoder.set_options_from_terminal(&self.terminal);
        let mut encoded = Vec::new();

        for event in &events {
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let Some(vt) = keys::vt_key(*key) else {
                continue;
            };
            if keys::is_modifier_key(vt) {
                continue;
            }
            // Ghostty encodes releases only when the program asked for them, and egui does
            // not tell us that, so presses and repeats are what get sent.
            if !pressed {
                continue;
            }

            let mods = keys::vt_mods(*modifiers);
            // A control or command combo is a command, not text: the platform's character
            // for it (or lack of one) must not override libghostty's encoding.
            let text = if mods.intersects(key::Mods::CTRL | key::Mods::SUPER) {
                None
            } else {
                let text = pending_text.get(text_index).cloned();
                if text.is_some() {
                    text_index += 1;
                }
                text
            };

            if let Err(error) = self.encode_key(vt, mods, text, *repeat, &mut encoded) {
                eprintln!("[moonreview] failed to encode a keystroke: {error}");
            }
        }

        // Anything left is text with no key event behind it: an IME commit, or a paste.
        for text in pending_text.drain(text_index.min(pending_text.len())..) {
            encoded.extend_from_slice(text.as_bytes());
        }

        if !encoded.is_empty()
            && let Err(error) = self.input.write(&encoded)
        {
            eprintln!("[moonreview] failed to write to the shell: {error}");
        }
    }

    fn encode_key(
        &mut self,
        vt: key::Key,
        mods: key::Mods,
        text: Option<String>,
        repeat: bool,
        out: &mut Vec<u8>,
    ) -> anyhow::Result<()> {
        let mut event = key::Event::new()
            .map_err(|error| anyhow::anyhow!("failed to create a key event: {error}"))?;
        event.set_action(if repeat { Action::Repeat } else { Action::Press });
        event.set_key(vt);
        event.set_mods(mods);
        if let Some(text) = text {
            event.set_utf8(Some(text));
        }
        if let Some(codepoint) = keys::unshifted_codepoint(vt) {
            event.set_unshifted_codepoint(codepoint);
        }

        self.encoder
            .encode_to_vec(&event, out)
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Paint one frame of the grid. Returns whether a blinking cursor wants a repaint.
    fn draw(
        &mut self,
        painter: &egui::Painter,
        origin: egui::Pos2,
        cell: egui::Vec2,
        font: &FontId,
        palette: &Palette,
        focused: bool,
    ) -> anyhow::Result<bool> {
        let Self {
            terminal,
            render_state,
            row_iterator,
            cell_iterator,
            blink_clock,
            ..
        } = self;
        // Read before the fields below are borrowed, so the cursor's phase does not depend
        // on how the emulator's own borrows are arranged.
        let blink_clock = *blink_clock;

        let snapshot = render_state
            .update(terminal)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let colors = snapshot
            .colors()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let default_fg = color_of(colors.foreground);
        let default_bg = color_of(colors.background);

        let mut rows = row_iterator
            .update(&snapshot)
            .map_err(|error| anyhow::anyhow!("{error}"))?;

        let mut y = origin.y;
        let mut text = String::with_capacity(16);

        while let Some(row) = rows.next() {
            let mut cells = cell_iterator
                .update(row)
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let mut x = origin.x;
            // One layout job per row: neighbouring cells that share a style become one run,
            // so a full screen costs a handful of galleys rather than a couple of thousand.
            let mut job = LayoutJob::default();
            let mut run: Option<Run> = None;

            while let Some(view) = cells.next() {
                let selected = view.is_selected().unwrap_or(false);
                let grapheme_count = view.graphemes_len().unwrap_or(0);
                let cell_bg = view.bg_color().ok().flatten().map(color_of);

                let mut fg = view.fg_color().ok().flatten().map(color_of).unwrap_or(default_fg);
                let mut bg = cell_bg;
                let mut bold = false;
                let mut italic = false;
                let mut underline = false;
                let mut strikethrough = false;

                if view.has_styling().unwrap_or(false)
                    && let Ok(style) = view.style()
                {
                    if style.inverse {
                        let swapped = bg.unwrap_or(default_bg);
                        bg = Some(fg);
                        fg = swapped;
                    }
                    bold = style.bold;
                    italic = style.italic;
                    underline = style.underline != libghostty_vt::style::Underline::None;
                    strikethrough = style.strikethrough;
                }
                if selected {
                    let swapped = bg.unwrap_or(default_bg);
                    bg = Some(fg);
                    fg = swapped;
                }

                if let Some(bg) = bg {
                    painter.rect_filled(
                        Rect::from_min_size(egui::pos2(x, y), cell),
                        CornerRadius::ZERO,
                        bg,
                    );
                }

                text.clear();
                if grapheme_count > 0 {
                    let _ = view.graphemes_utf8(&mut text);
                }
                // A blank cell still has to occupy its column in the row's layout.
                let glyph = if text.is_empty() { " " } else { text.as_str() };

                let style = CellStyle {
                    fg,
                    bold,
                    italic,
                    underline,
                    strikethrough,
                };
                match &mut run {
                    Some(current) if current.style == style => current.text.push_str(glyph),
                    Some(current) => {
                        current.flush(&mut job, font, palette);
                        *current = Run::new(style, glyph);
                    }
                    None => run = Some(Run::new(style, glyph)),
                }

                x += cell.x;
            }

            if let Some(mut current) = run.take() {
                current.flush(&mut job, font, palette);
            }
            if !job.sections.is_empty() {
                let galley = painter.layout_job(job);
                painter.galley(egui::pos2(origin.x, y), galley, default_fg);
            }

            let _ = row.set_dirty(false);
            y += cell.y;
        }

        let mut wants_repaint = false;
        if snapshot.cursor_visible().unwrap_or(false)
            && let Ok(Some(viewport)) = snapshot.cursor_viewport()
        {
            let blinking = snapshot.cursor_blinking().unwrap_or(false);
            let visible = !blinking || blink_clock.rem_euclid(1.06) < 0.53;
            wants_repaint = blinking;

            if visible {
                let color = colors.cursor.map(color_of).unwrap_or(palette.accent);
                let at = egui::pos2(
                    origin.x + f32::from(viewport.x) * cell.x,
                    origin.y + f32::from(viewport.y) * cell.y,
                );
                draw_cursor(
                    painter,
                    Rect::from_min_size(at, cell),
                    snapshot.cursor_visual_style().unwrap_or(CursorVisualStyle::Block),
                    color,
                    focused,
                );
            }
        }

        let _ = snapshot.set_dirty(Dirty::Clean);
        Ok(wants_repaint)
    }
}

fn new_render_state() -> anyhow::Result<RenderState<'static>> {
    RenderState::new().map_err(|error| anyhow::anyhow!("failed to create a render state: {error}"))
}

/// A window that is not focused shows a hollow cursor, the way terminals normally do.
fn draw_cursor(
    painter: &egui::Painter,
    rect: Rect,
    style: CursorVisualStyle,
    color: Color32,
    focused: bool,
) {
    // An unfocused pane, and a program that asked for a hollow cursor, both draw an outline.
    if !focused || style == CursorVisualStyle::BlockHollow {
        painter.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::ZERO,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
        return;
    }

    let filled = match style {
        CursorVisualStyle::Bar => Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
        CursorVisualStyle::Underline => Rect::from_min_size(
            egui::pos2(rect.min.x, rect.max.y - 2.0),
            vec2(rect.width(), 2.0),
        ),
        _ => rect,
    };
    painter.rect_filled(filled, CornerRadius::ZERO, color);
}

const PADDING: f32 = 6.0;

#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

/// A run of neighbouring cells that share a style, accumulated so the row can be laid out
/// in as few sections as possible.
struct Run {
    style: CellStyle,
    text: String,
}

impl Run {
    fn new(style: CellStyle, glyph: &str) -> Self {
        Self {
            style,
            text: glyph.to_string(),
        }
    }

    fn flush(&mut self, job: &mut LayoutJob, font: &FontId, palette: &Palette) {
        if self.text.is_empty() {
            return;
        }
        let mut format = TextFormat {
            font_id: font.clone(),
            color: self.style.fg,
            italics: self.style.italic,
            ..Default::default()
        };
        if self.style.underline {
            format.underline = Stroke::new(1.0, self.style.fg);
        }
        if self.style.strikethrough {
            format.strikethrough = Stroke::new(1.0, self.style.fg);
        }
        // egui's bundled monospace font has no bold face, so bold is rendered as a brighter
        // ink against the same background rather than a heavier stroke.
        if self.style.bold {
            format.color = brighten(self.style.fg, palette);
        }
        job.append(&self.text, 0.0, format);
        self.text.clear();
    }
}

fn brighten(color: Color32, palette: &Palette) -> Color32 {
    let toward = palette.ink;
    let mix = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
    Color32::from_rgb(
        mix(color.r(), toward.r()),
        mix(color.g(), toward.g()),
        mix(color.b(), toward.b()),
    )
}

fn color_of(rgb: RgbColor) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// The advance and line height of the monospace font, which is what a terminal cell is.
/// Measured from a laid-out glyph so the background rects the grid draws line up with the
/// text egui puts inside them.
pub(crate) fn cell_size(painter: &egui::Painter, font: &FontId) -> egui::Vec2 {
    let galley = painter.layout_no_wrap("M".to_string(), font.clone(), Color32::WHITE);
    vec2(galley.size().x.max(1.0), galley.size().y.max(1.0))
}
