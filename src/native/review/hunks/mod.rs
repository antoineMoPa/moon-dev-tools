//! The diff itself: unstaged hunks, then staged ones, grouped by file.
//!
//! Selecting lines is how everything specific happens here - a comment is anchored to the
//! lines it was written against, and a partial stage applies exactly those lines. That is
//! the contract a text selection has. The selection itself is
//! character-precise - a drag sweeps characters, a double-click takes a word - but a comment
//! is still anchored to the whole lines the selection covers.

mod actions;
mod comments;
mod lines;

use std::sync::Arc;

use egui::{CornerRadius, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::{
    api::HunkView,
    native::{
        app::App,
        review::diff::DiffLine,
        review::image_diff,
        theme::{CODE_SIZE, Palette, SMALL_SIZE},
        widgets,
    },
};

use actions::{copy_selected_lines, current_selection, draw_hunk_toolbar, open_draft};
use lines::draw_hunk_body;

pub(super) const GUTTER_WIDTH: f32 = 74.0;
pub(super) const LINE_HEIGHT: f32 = 15.0;

/// One diff line's widget id. Derived from the hunk and the line rather than from the
/// enclosing `Ui`, so it is the same wherever the line is drawn - which is also what lets the
/// tests find a line and click it.
pub(crate) fn diff_line_id(hunk_id: &str, index: usize) -> egui::Id {
    egui::Id::new(("moonreview-diff-line", hunk_id, index))
}

/// Where a line's body text starts, matching what `draw_line_text` paints: the gutter, then
/// the one-character `+`/`-`/space marker column. Only commentable lines are selectable, and
/// they all carry the marker.
pub(crate) fn body_text_x(rect: Rect) -> f32 {
    rect.min.x + GUTTER_WIDTH + 6.0 + 9.0
}

/// The character column of the body under a pointer x, by laying the body out the same way
/// it is painted. Past the end of the text this is the body's length.
pub(super) fn column_at(ui: &Ui, rect: Rect, line: &DiffLine, x: f32) -> usize {
    let font = egui::FontId::monospace(CODE_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(line.body().to_string(), font, egui::Color32::WHITE);
    galley
        .cursor_from_pos(vec2(x - body_text_x(rect), 0.0))
        .index
        .into()
}

/// The token containing a column, so a double-click picks up a whole identifier. Columns
/// past the end of the text belong to no token.
pub(super) fn word_bounds_at(body: &str, column: usize) -> Option<(usize, usize)> {
    let mut at = 0;
    for token in super::diff::tokenize(body) {
        let len = token.chars().count();
        if column < at + len {
            return Some((at, at + len));
        }
        at += len;
    }
    None
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    // The payload is shared, so the hunks below are read straight out of it rather than
    // copied - a diff of a lock file is far too much text to clone every frame.
    let Some(payload) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.as_ref().map(Arc::clone))
    else {
        return;
    };
    let read_only = payload.read_only;
    let is_commit_review = payload.active_commit.is_some();
    let preview_limit = payload.patch_preview_line_limit;

    // A review of one unchanged file has nothing to diff, so the file opens in a tab of its
    // own - `moonreview package.json` on a file nobody has touched is a request to read it.
    if payload.hunks.is_empty()
        && let Some(path) = payload.full_file_path.as_deref()
    {
        app.open_file_pane(session_id, path);
    }

    let hunks: Vec<&HunkView> = payload.hunks.iter().collect();
    if hunks.is_empty() {
        draw_empty(ui, palette);
        return;
    }

    copy_selected_lines(app, ui, session_id);

    let unstaged: Vec<&HunkView> = hunks.iter().copied().filter(|hunk| !hunk.staged).collect();
    let staged: Vec<&HunkView> = hunks.iter().copied().filter(|hunk| hunk.staged).collect();

    let scroll_target = app
        .model
        .review(session_id)
        .scroll_to_hunk
        .take();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        // Dragging is how lines get selected here, so it must not also mean "scroll" - not
        // even on a touch screen, where it is the default.
        .scroll_source(egui::containers::scroll_area::ScrollSource {
            drag: egui::containers::scroll_area::DragScroll::Never,
            ..Default::default()
        })
        .show(ui, |ui| {
            if read_only {
                draw_section(
                    app,
                    ui,
                    session_id,
                    if is_commit_review { "commit" } else { "diff" },
                    &hunks,
                    read_only,
                    preview_limit,
                    scroll_target.as_deref(),
                    palette,
                );
                return;
            }

            draw_section(
                app,
                ui,
                session_id,
                "unstaged",
                &unstaged,
                read_only,
                preview_limit,
                scroll_target.as_deref(),
                palette,
            );
            if !staged.is_empty() {
                ui.add_space(12.0);
                draw_section(
                    app,
                    ui,
                    session_id,
                    "staged",
                    &staged,
                    read_only,
                    preview_limit,
                    scroll_target.as_deref(),
                    palette,
                );
            }
        });

    finish_line_sweep(app, ui, session_id, &payload.hunks);
}

/// When the button comes up after a sweep, the run is settled and the composer opens on it.
fn finish_line_sweep(app: &mut App, ui: &Ui, session_id: &str, hunks: &[HunkView]) {
    let Some(hunk_id) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selecting_in.clone())
    else {
        return;
    };
    if !ui.input(|input| input.pointer.any_released()) {
        return;
    }
    app.model.review(session_id).selecting_in = None;

    let Some(hunk) = hunks.iter().find(|hunk| hunk.id == hunk_id) else {
        return;
    };
    let Some(selection) = current_selection(app, session_id, &hunk_id) else {
        return;
    };
    open_draft(app, session_id, hunk, selection);
}

fn draw_empty(ui: &mut Ui, palette: &Palette) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.3);
        ui.label(RichText::new("nothing to review").color(palette.ink));
        ui.add_space(5.0);
        ui.label(
            RichText::new("the working tree is clean")
                .size(SMALL_SIZE)
                .color(palette.muted),
        );
    });
}

#[allow(clippy::too_many_arguments, reason = "one call site; the alternative is a \
    parameter struct that only exists to be destructured immediately")]
fn draw_section(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    title: &str,
    hunks: &[&HunkView],
    read_only: bool,
    preview_limit: usize,
    scroll_target: Option<&str>,
    palette: &Palette,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted)
                .strong(),
        );
        ui.label(
            RichText::new(hunks.len().to_string())
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted),
        );
    });
    ui.add_space(4.0);

    if hunks.is_empty() {
        ui.label(
            RichText::new(if read_only {
                "no changes"
            } else {
                "everything is staged"
            })
            .color(palette.muted),
        );
        return;
    }

    // Hunks arrive grouped by file already; the headings just make that visible.
    let mut current_file: Option<&str> = None;
    for hunk in hunks {
        if current_file != Some(hunk.file_path.as_str()) {
            current_file = Some(&hunk.file_path);
            ui.add_space(6.0);
            draw_file_heading(app, ui, session_id, &hunk.file_path, palette);
        }
        draw_hunk_card(
            app,
            ui,
            session_id,
            hunk,
            read_only,
            preview_limit,
            scroll_target,
            palette,
        );
        ui.add_space(6.0);
    }
}

fn draw_file_heading(app: &mut App, ui: &mut Ui, session_id: &str, file_path: &str, palette: &Palette) {
    let collapsed = app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.collapsed_files.contains(file_path));

    ui.horizontal(|ui| {
        let arrow = if collapsed { "\u{23F5}" } else { "\u{23F7}" };
        if widgets::quiet_button_colored(ui, &format!("{arrow} {file_path}"), palette.ink).clicked() {
            let review = app.model.review(session_id);
            if collapsed {
                review.collapsed_files.remove(file_path);
            } else {
                review.collapsed_files.insert(file_path.to_string());
            }
        }
    });
}

#[allow(clippy::too_many_arguments, reason = "one call site; the alternative is a \
    parameter struct that only exists to be destructured immediately")]
fn draw_hunk_card(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    read_only: bool,
    preview_limit: usize,
    scroll_target: Option<&str>,
    palette: &Palette,
) {
    if app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.collapsed_files.contains(&hunk.file_path))
    {
        return;
    }

    let is_active = app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.active_hunk_id.as_deref() == Some(hunk.id.as_str()));

    // A card that is scrolled out of sight takes the space it took last time and nothing else.
    // A review of a thousand hunks is a thousand cards, and laying out every one of them on
    // every frame is what makes scrolling stagger; only the handful on screen is real work.
    //
    // The card being scrolled to is always drawn, because it is the one that has to report
    // where it landed.
    if scroll_target != Some(hunk.id.as_str())
        && let Some(height) = app.hunk_heights.get(&hunk.id).copied()
    {
        let skipped = Rect::from_min_size(
            ui.cursor().min,
            vec2(ui.available_width(), height),
        );
        if !ui.is_rect_visible(skipped) {
            ui.allocate_exact_size(skipped.size(), Sense::hover());
            return;
        }
    }

    let frame = egui::Frame::new()
        .fill(palette.code_bg)
        .stroke(Stroke::new(
            1.0,
            if is_active { palette.accent } else { palette.line },
        ))
        // Square: a hunk is a block of code, and a rounded box around monospaced rows that
        // run to its edges reads as a card the code is escaping rather than a frame round it.
        .corner_radius(CornerRadius::ZERO)
        // The rows inside take the whole width they are offered, and are painted over the
        // card's own background and border. Without a margin to sit inside, a full-width row
        // paints over the border on the right and the card looks like it has burst its edge.
        .inner_margin(egui::Margin::same(1))
        .outer_margin(egui::Margin::symmetric(2, 0));

    let response = frame
        .show(ui, |ui| {
            draw_hunk_toolbar(app, ui, session_id, hunk, read_only, palette);
            if let Some(image) = &hunk.image_diff {
                image_diff::draw_image_diff(app, ui, &hunk.file_path, image, palette);
                return;
            }
            draw_hunk_body(app, ui, session_id, hunk, read_only, preview_limit, palette);
        })
        .response;

    // A stripe down the left of the card, so the hunk the keyboard acts on is obvious even
    // when the pointer has moved on.
    if is_active {
        let rect = response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            CornerRadius::ZERO,
            palette.hunk_active_bg,
        );
    }

    if scroll_target == Some(hunk.id.as_str()) {
        response.scroll_to_me(Some(egui::Align::TOP));
        app.model.review(session_id).active_hunk_id = Some(hunk.id.clone());
    }

    // The hunk under the caret is what `s` and `u` act on, so pointing at one selects it.
    if response.hovered() && !is_active {
        app.model.review(session_id).active_hunk_id = Some(hunk.id.clone());
    }

    // What it measured this time is what the next frame skips it with.
    app.hunk_heights
        .insert(hunk.id.clone(), response.rect.height());
}
