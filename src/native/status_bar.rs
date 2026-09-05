//! The strip along the bottom of the window: what a language server is doing, and the last
//! thing the window said.
//!
//! Two things a window has always been bad at saying. A ⌘-click on a name in a file that
//! comes back with `rust is still indexing this project - try again in a moment` is the
//! window telling somebody about a wait it knew about all along and never mentioned; and a
//! toast that faded while they were reading code is a message that may as well never have
//! been posted. So the strip says what the servers are doing while they are doing it, and
//! the rest of the time it says the last thing the window said - and a click on it opens the
//! whole log, which is [`crate::native::messages`].
//!
//! **The bar is there from the first thing the window has to say, and then it stays.** It is
//! hidden only in the one state where it would be a blank strip over a window that has said
//! nothing and is waiting for nothing - the first seconds of a run. The moment there is
//! either a message or a server working it appears, and it never goes away again, because
//! the log it is reading from never empties. So the workspace is reflowed at most once in
//! the life of a window, early, rather than jumping up and down under the pointer every time
//! a server starts indexing and finishes - which is the thing that would make a bar that
//! comes and goes worse than no bar at all.
//!
//! It takes no keyboard: it is a strip that is read and clicked, and the editor or the shell
//! that has the keyboard keeps it.

use std::time::{Duration, Instant};

use egui::{Align, CornerRadius, RichText, Sense, Stroke, Ui, vec2};

use crate::{
    api::LspWork,
    native::{
        app::App,
        lsp_document::Served,
        panes::{OpenPaneRequest, Pane},
        theme::{Palette, SMALL_SIZE},
    },
};

/// How tall the strip is. One line of the small face with a little air around it: enough to
/// read, and little enough that the workspace above it does not notice.
const BAR_HEIGHT: f32 = 24.0;

/// How wide the progress bar at the right end is drawn, for work that says how far along it
/// is.
const PROGRESS_WIDTH: f32 = 90.0;

/// How long an answer stands for.
///
/// What the bar says is what a server said a moment ago. Once the window stops asking - the
/// last file pane with a server behind it was closed, or the poll started failing - the line
/// goes rather than freezing on screen for the rest of the run saying a project is being
/// indexed that finished being indexed ten minutes ago. Three intervals: long enough that a
/// single slow or missed answer never blinks the bar, short enough that a stale line is gone
/// before anyone reads it twice.
const WORK_STANDS_FOR: Duration = Duration::from_millis(2_250);

/// How often the window asks what the language servers are doing.
///
/// Not every frame: on a `--remote` session that is a network round trip, and sixty of them
/// a second for a line of text nobody is staring at would be absurd. Three quarters of a
/// second is slower than the eye notices on a percentage that takes tens of seconds to climb
/// and quick enough that the bar is never a second stale when indexing ends.
///
/// It is also only asked at all while there is something to ask about - a file pane whose
/// file actually has a server behind it. A window on a board and a shell asks nothing.
const WORK_IS_ASKED_ABOUT_EVERY: Duration = Duration::from_millis(750);

impl App {
    /// The strip, drawn below the frames. Called before the workspace so the space it takes
    /// is taken off the bottom of the window rather than out of a pane's scroll.
    pub(crate) fn draw_status_bar(&mut self, ui: &mut Ui) {
        let palette = self.palette_of();
        let Some(line) = self.status_line() else {
            return;
        };

        let clicked = egui::Panel::bottom("moonreview-status-bar")
            .exact_size(BAR_HEIGHT)
            .resizable(false)
            // It is a strip to read, not an edge to pull on.
            .drag_to_open(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.header_bg)
                    .stroke(Stroke::NONE)
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show(ui, |ui| draw_line(ui, &line, &palette))
            .inner;

        if clicked {
            // Deferred like every other pane the window opens from inside a draw: the tree
            // holding the panes is being drawn right now.
            self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
                OpenPaneRequest::Messages,
            ));
        }
    }

    /// What the bar has to say this frame, or `None` for a window that has said nothing and
    /// is waiting for nothing.
    fn status_line(&self) -> Option<StatusLine> {
        if let Some(work) = self.language_server_work() {
            return Some(StatusLine::Working {
                work: work.clone(),
                others: self.language_server_work_count() - 1,
            });
        }
        let latest = self.model.messages.latest()?;
        Some(StatusLine::Said {
            text: latest.text.clone(),
            failed: matches!(latest.kind, crate::native::model::ToastKind::Error),
            at_unix: latest.at_unix,
        })
    }

    /// The piece of work the bar names, out of everything the window's servers are doing.
    ///
    /// The first of them, by server name: two servers indexing at once is a window with a
    /// Rust file and a TypeScript file open, and a strip that named both would say less than
    /// one that names one. How many others there are goes on the line beside it.
    fn language_server_work(&self) -> Option<&LspWork> {
        self.language_server_work_now()
            .min_by(|left, right| left.server.cmp(&right.server))
    }

    /// How many pieces of work are going on altogether.
    fn language_server_work_count(&self) -> usize {
        self.language_server_work_now().count()
    }

    /// Everything the window has been told is going on, leaving out what it was told too
    /// long ago to still stand - see [`WORK_STANDS_FOR`].
    fn language_server_work_now(&self) -> impl Iterator<Item = &LspWork> {
        self.model
            .language_servers_working
            .values()
            .filter(|answer| answer.heard_at.elapsed() < WORK_STANDS_FOR)
            .flat_map(|answer| answer.working.iter())
    }

    /// Ask what the servers are doing, on a timer and only while there is something to ask
    /// about - see [`WORK_IS_ASKED_ABOUT_EVERY`].
    ///
    /// The sessions asked are the ones with an open file pane that actually has a server
    /// behind it. Everything else - a board, a shell, a markdown file - has no server to
    /// wait on, and a window full of those asks nothing at all.
    pub(crate) fn poll_language_server_work(&mut self) {
        let sessions = self.sessions_with_a_language_server();
        // A session whose last served pane has closed is simply not asked about any more,
        // and what it last said stops standing on its own - see [`WORK_STANDS_FOR`].
        if sessions.is_empty() {
            return;
        }
        if self.last_lsp_work_poll.elapsed() < WORK_IS_ASKED_ABOUT_EVERY {
            return;
        }
        self.last_lsp_work_poll = Instant::now();

        for session_id in sessions {
            let for_call = session_id.clone();
            self.tasks.spawn_keyed(
                Some(format!("lsp-working:{session_id}")),
                move |backend| backend.lsp_working(&for_call),
                move |model, result| {
                    // An answer that could not be had leaves the bar as it was: the servers
                    // are still doing whatever they were doing, and a bar that emptied every
                    // time a poll missed would flicker.
                    if let Ok(working) = result {
                        model
                            .language_servers_working
                            .insert(session_id, ServersWorking::heard_now(working));
                    }
                },
            );
        }
    }

    /// The sessions with a file pane open on a file something serves.
    fn sessions_with_a_language_server(&self) -> std::collections::HashSet<String> {
        self.model
            .layout
            .panes()
            .filter_map(|(pane_id, pane)| match pane {
                Pane::File { session_id, .. } => {
                    let editor = self.model.file_editors.get(&pane_id)?;
                    matches!(editor.server_heard(), Served::Yes(_)).then(|| session_id.clone())
                }
                _ => None,
            })
            .collect()
    }
}

/// One session's servers, and when the window last heard it. The moment is kept with the
/// answer because an answer nobody has refreshed is one the bar stops reading out.
pub(crate) struct ServersWorking {
    pub(crate) heard_at: Instant,
    pub(crate) working: Vec<LspWork>,
}

impl ServersWorking {
    pub(crate) fn heard_now(working: Vec<LspWork>) -> Self {
        Self {
            heard_at: Instant::now(),
            working,
        }
    }
}

/// What the bar is saying: a server at work, or the last thing the window said.
enum StatusLine {
    Working {
        work: LspWork,
        /// The other pieces of work going on at the same time, which the line counts rather
        /// than names.
        others: usize,
    },
    Said {
        text: String,
        failed: bool,
        at_unix: u64,
    },
}

/// Draw the strip and answer whether it was clicked.
fn draw_line(ui: &mut Ui, line: &StatusLine, palette: &Palette) -> bool {
    let hovered = ui.rect_contains_pointer(ui.max_rect());
    let response = ui.interact(
        ui.max_rect(),
        ui.id().with("status-bar"),
        // Click only: the strip is read and pressed, and it never asks for the keyboard the
        // editor or the shell above it is holding.
        Sense::click(),
    );
    if hovered {
        ui.painter()
            .rect_filled(ui.max_rect(), CornerRadius::ZERO, palette.row_hover_bg);
    }
    // No line of its own along the top: the workspace above already ends in its own border,
    // and a second one drawn inside this strip's margin reads as a double edge rather than as
    // a separation.

    ui.horizontal_centered(|ui| match line {
        StatusLine::Working { work, others } => draw_working(ui, work, *others, palette),
        StatusLine::Said {
            text,
            failed,
            at_unix,
        } => {
            let ink = if *failed { palette.warn } else { palette.muted };
            ui.label(
                RichText::new(crate::native::messages::clock_label(*at_unix))
                    .monospace()
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            ui.add(
                egui::Label::new(RichText::new(text).size(SMALL_SIZE).color(ink))
                    .selectable(false)
                    .truncate(),
            );
        }
    });

    let _ = crate::native::widgets::clickable(response.clone())
        .on_hover_text("Every message this window has posted");
    response.clicked()
}

/// A server at work: what it is doing, and how far through where it says.
///
/// **Work that reports no percentage is the ordinary case**, not a degraded one: a server
/// fetching a project's metadata says what it is doing and nothing about how long it will
/// take. So the line is written to be complete without a number - "rust-analyzer fetching
/// metadata" says the whole of what is known - and the bar at the right end is simply absent
/// where there is nothing to fill it with.
fn draw_working(ui: &mut Ui, work: &LspWork, others: usize, palette: &Palette) {
    // A dot rather than a spinner: a spinner is a picture that differs on every frame, which
    // makes the strip a thing no snapshot can hold still, and the line beside it already says
    // that something is going on.
    let (dot, _) = ui.allocate_exact_size(vec2(7.0, 7.0), Sense::hover());
    ui.painter()
        .circle_filled(dot.center(), 3.0, palette.accent);
    let mut said = format!("{} {}", work.server, work.title.to_lowercase());
    if let Some(detail) = &work.detail {
        said.push_str(" - ");
        said.push_str(detail);
    }
    if others > 0 {
        said.push_str(&format!(" (and {others} more)"));
    }
    ui.add(
        egui::Label::new(RichText::new(said).size(SMALL_SIZE).color(palette.ink))
            .selectable(false)
            .truncate(),
    );

    let Some(percentage) = work.percentage else {
        return;
    };
    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
        ui.label(
            RichText::new(format!("{percentage}%"))
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
        let (rect, _) = ui.allocate_exact_size(vec2(PROGRESS_WIDTH, 5.0), Sense::hover());
        // The track is drawn in the line color rather than a control's fill: it sits on the
        // strip's own ground, and a fill meant for a button is invisible against it.
        ui.painter()
            .rect_filled(rect, CornerRadius::same(2), palette.line);
        let mut filled = rect;
        filled.set_width(rect.width() * f32::from(percentage) / 100.0);
        ui.painter()
            .rect_filled(filled, CornerRadius::same(2), palette.accent);
    });
}
