//! One diff line: its gutter, its text, the find marks on it, and the selection sweeping
//! across it.

use egui::{Align2, CornerRadius, Sense, Stroke, Ui, vec2};

use crate::{
    api::HunkView,
    comments::parse_anchored_comments,
    native::{
        app::App,
        model::{LINE_END, LineSelection, SelectionPoint, hash_of},
        review::diff::{DiffLine, LineKind, insertion_line},
        review::search,
        theme::{CODE_SIZE, Palette},
    },
};

use super::actions::{current_selection, draw_truncation_notice, select_and_open};
use super::comments::{draw_composer, draw_inline_comment};
use super::{GUTTER_WIDTH, LINE_HEIGHT, body_text_x, column_at, diff_line_id, word_bounds_at};

pub(super) fn draw_hunk_body(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    read_only: bool,
    preview_limit: usize,
    palette: &Palette,
) {
    // The server sends a preview; the whole patch is fetched only if asked for.
    let full_patch = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.expanded_patches.get(&hunk.id))
        .cloned();
    let patch = full_patch.clone().unwrap_or_else(|| hunk.patch_preview.clone());
    let lines = app.diff_lines(&hunk.id, &patch);

    let anchored = parse_anchored_comments(&hunk.comment);
    let mut comment_at: Vec<(usize, usize)> = Vec::new();
    let mut used = Vec::new();
    for (index, entry) in anchored.iter().enumerate() {
        if let Some(at) = insertion_line(&lines, &entry.selection, &used) {
            used.push(at);
            comment_at.push((at, index));
        }
    }
    comment_at.sort_unstable();

    // Each composer goes below the last line of the run it is about, so the run stays
    // together above it. The one for the current selection knows exactly which lines those
    // are; a parked one is placed by matching its text, like a saved comment, because the
    // lines it was written against may have moved.
    let selection_anchor = current_selection(app, session_id, &hunk.id);
    let selection_end = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selection)
        .filter(|selection| selection.hunk_id_hash == hash_of(&hunk.id))
        .map(|selection| *selection.line_range().end());
    let mut draft_at: Vec<(usize, String)> = Vec::new();
    let mut unplaced: Vec<String> = Vec::new();
    if let Some(review) = app.model.review_ref(session_id) {
        for draft in review.drafts.iter().filter(|draft| draft.hunk_id == hunk.id) {
            let at = if selection_anchor.as_deref() == Some(draft.selection.as_str()) {
                selection_end
            } else {
                None
            }
            .or_else(|| insertion_line(&lines, &draft.selection, &used));
            match at {
                Some(at) => {
                    used.push(at);
                    draft_at.push((at, draft.selection.clone()));
                }
                // A draft anchored to lines the preview does not contain still has to be
                // reachable; it goes at the end of the hunk.
                None => unplaced.push(draft.selection.clone()),
            }
        }
    }
    draft_at.sort_unstable();

    for (index, line) in lines.iter().enumerate() {
        // Hidden lines still count: a selection is matched against the raw patch text, so
        // the indices have to stay in step with it.
        if line.is_chrome() {
            continue;
        }
        draw_diff_line(app, ui, session_id, hunk, index, line, palette);

        for (_, comment_index) in comment_at.iter().filter(|(at, _)| *at == index) {
            if let Some(entry) = anchored.get(*comment_index) {
                draw_inline_comment(app, ui, session_id, hunk, *comment_index, entry, palette);
            }
        }
        for (_, anchor) in draft_at.iter().filter(|(at, _)| *at == index) {
            draw_composer(app, ui, session_id, hunk, anchor, read_only, palette);
        }
    }

    for anchor in &unplaced {
        draw_composer(app, ui, session_id, hunk, anchor, read_only, palette);
    }

    if hunk.patch_line_count > preview_limit && full_patch.is_none() {
        draw_truncation_notice(app, ui, session_id, hunk, preview_limit, palette);
    }
}

fn draw_diff_line(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    index: usize,
    line: &DiffLine,
    palette: &Palette,
) {
    let width = ui.available_width();
    let selectable = line.kind.commentable();
    // Claim the row's space without registering anything for it. A diff of `Cargo.lock` is
    // tens of thousands of rows, and neither laying out text nor hit-testing a row that is
    // scrolled out of sight is work worth doing.
    let (rect, _) = ui.allocate_exact_size(vec2(width, LINE_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }

    let response = ui.interact(
        rect,
        diff_line_id(&hunk.id, index),
        if selectable {
            // Dragging is how a run of lines gets picked, the same gesture as sweeping over
            // text anywhere else.
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );

    let selected_columns = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selection)
        .filter(|selection| selection.hunk_id_hash == hash_of(&hunk.id))
        .and_then(|selection| selection.columns_on(index));

    // Added and removed lines keep their own tint, and a selected one is tinted again on top
    // of it - so a selected removal still reads as a removal.
    if let Some(background) = palette.diff_line_bg(line.kind.prefix()) {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, background);
    }
    if let Some((from, to)) = selected_columns {
        let span = if (from, to) == (0, LINE_END) {
            rect
        } else {
            // A partial line shows exactly the characters the sweep covered, measured the
            // same way they are painted.
            let font = egui::FontId::monospace(CODE_SIZE);
            let body: Vec<char> = line.body().chars().collect();
            let width_of = |from: usize, to: usize| {
                let text: String =
                    body[from.min(body.len())..to.min(body.len())].iter().collect();
                ui.painter()
                    .layout_no_wrap(text, font.clone(), palette.ink)
                    .size()
                    .x
            };
            let left = body_text_x(rect) + width_of(0, from);
            egui::Rect::from_min_size(
                egui::pos2(left, rect.min.y),
                vec2(width_of(from, to), rect.height()),
            )
        };
        ui.painter()
            .rect_filled(span, CornerRadius::ZERO, palette.line_target_bg);
        // A solid bar down the left edge: the tint alone is easy to miss against a diff that
        // is already coloured.
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(2.5, rect.height())),
            CornerRadius::ZERO,
            palette.accent,
        );
    }
    if response.hovered() && selectable && selected_columns.is_none() {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, palette.diff_gutter_bg);
    }

    draw_gutter(ui, rect, line, palette);
    let marks = find_marks(app, session_id, &hunk.id, index, line);
    draw_line_text(ui, rect, line, palette, &marks);

    if !selectable {
        return;
    }

    // Starting a selection here takes the keyboard from whoever had it. A shell keeps its
    // focus through a sweep that never touches it - egui only surrenders focus on clicks -
    // and the copy chord has to follow the selection the user just made, not the one made
    // before it.
    if response.drag_started()
        || response.clicked()
        || response.double_clicked()
        || response.triple_clicked()
    {
        ui.ctx().memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }

    // A drag sweeps characters. It starts where the button went down, and every line the
    // pointer passes over extends it - the drag belongs to the line it began on, so each
    // other line has to notice the pointer itself rather than wait for an event it will
    // never get.
    let hunk_hash = hash_of(&hunk.id);
    if response.drag_started() {
        // By the time egui decides a press is a drag the pointer has already travelled, so
        // the anchor comes from where the press began, not from where the pointer is now.
        let start_x = ui
            .input(|input| input.pointer.press_origin())
            .map(|at| at.x)
            .unwrap_or(rect.min.x);
        let at = SelectionPoint {
            line: index,
            column: column_at(ui, rect, line, start_x),
        };
        let review = app.model.review(session_id);
        review.selection = Some(LineSelection {
            hunk_id_hash: hunk_hash,
            anchor: at,
            head: at,
        });
        review.selecting_in = Some(hunk.id.clone());
        review.active_hunk_id = Some(hunk.id.clone());
        // The draft is left alone: whatever has been typed re-anchors to the new run when
        // the button comes up.
        return;
    }

    let sweeping_here = app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.selecting_in.as_deref() == Some(hunk.id.as_str()));
    if sweeping_here {
        let pointer_at = ui
            .input(|input| input.pointer.interact_pos())
            .filter(|at| rect.y_range().contains(at.y));
        if let Some(at) = pointer_at {
            let head = SelectionPoint {
                line: index,
                column: column_at(ui, rect, line, at.x),
            };
            let review = app.model.review(session_id);
            if let Some(existing) = review.selection
                && existing.hunk_id_hash == hunk_hash
                && existing.head != head
            {
                review.selection = Some(LineSelection {
                    head,
                    ..existing
                });
            }
        }
        return;
    }

    // A double-click takes the word under the pointer, split the same way the word diff
    // splits a line; a triple-click takes the whole line back.
    if response.triple_clicked() {
        select_and_open(app, session_id, hunk, LineSelection::whole_line(hunk_hash, index));
        return;
    }
    if response.double_clicked() {
        let selection = response
            .interact_pointer_pos()
            .and_then(|at| {
                word_bounds_at(line.body(), column_at(ui, rect, line, at.x))
            })
            .map(|(from, to)| LineSelection {
                hunk_id_hash: hunk_hash,
                anchor: SelectionPoint {
                    line: index,
                    column: from,
                },
                head: SelectionPoint {
                    line: index,
                    column: to,
                },
            })
            .unwrap_or_else(|| LineSelection::whole_line(hunk_hash, index));
        select_and_open(app, session_id, hunk, selection);
        return;
    }

    if !response.clicked() {
        return;
    }

    // Selecting lines and writing a comment are one gesture: a click selects and opens the
    // composer at once.
    let extend = ui.input(|input| input.modifiers.shift);
    let whole_line = LineSelection::whole_line(hunk_hash, index);
    let existing = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selection)
        .filter(|selection| selection.hunk_id_hash == hunk_hash);

    if let Some(existing) = existing
        && extend
    {
        // Shift-click grows the run by whole lines, which is how a multi-line comment gets
        // its anchor. The anchor line stays put; its covered end faces the new head.
        let anchor_line = existing.anchor.line;
        let (anchor, head) = if index >= anchor_line {
            (
                SelectionPoint {
                    line: anchor_line,
                    column: 0,
                },
                SelectionPoint {
                    line: index,
                    column: LINE_END,
                },
            )
        } else {
            (
                SelectionPoint {
                    line: anchor_line,
                    column: LINE_END,
                },
                SelectionPoint {
                    line: index,
                    column: 0,
                },
            )
        };
        select_and_open(
            app,
            session_id,
            hunk,
            LineSelection {
                hunk_id_hash: hunk_hash,
                anchor,
                head,
            },
        );
        return;
    }

    if let Some(existing) = existing
        && existing.anchor == whole_line.anchor
        && existing.head == whole_line.head
    {
        // Clicking the one selected line again deselects and puts an unwritten composer
        // away. A typed one stays parked where it is - a stray click must not throw text
        // away, and the way to be rid of it is its own cancel.
        let review = app.model.review(session_id);
        review.selection = None;
        review.active_hunk_id = Some(hunk.id.clone());
        review.drafts.retain(|draft| !draft.note.trim().is_empty());
        return;
    }

    select_and_open(app, session_id, hunk, whole_line);
}

fn draw_gutter(ui: &Ui, rect: egui::Rect, line: &DiffLine, palette: &Palette) {
    let painter = ui.painter();
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, vec2(GUTTER_WIDTH, rect.height())),
        CornerRadius::ZERO,
        palette.diff_gutter_bg,
    );
    painter.vline(
        rect.min.x + GUTTER_WIDTH,
        rect.y_range(),
        Stroke::new(1.0, palette.diff_gutter_line),
    );

    let font = egui::FontId::monospace(CODE_SIZE - 1.0);
    let number = |value: Option<usize>| {
        value
            .map(|value| value.to_string())
            .unwrap_or_default()
    };
    painter.text(
        egui::pos2(rect.min.x + 32.0, rect.center().y),
        Align2::RIGHT_CENTER,
        number(line.old_line_number),
        font.clone(),
        palette.muted,
    );
    painter.text(
        egui::pos2(rect.min.x + 66.0, rect.center().y),
        Align2::RIGHT_CENTER,
        number(line.new_line_number),
        font,
        palette.muted,
    );
}

pub(super) fn draw_line_text(
    ui: &Ui,
    rect: egui::Rect,
    line: &DiffLine,
    palette: &Palette,
    marks: &FindMarks,
) {
    // A diff line is as long as the code is, and the pane does not scroll sideways, so a long
    // one has to stop at the edge of its row. Without this it carries on over the hunk card's
    // border and out across whatever the pane is showing beside it.
    let painter = ui.painter().with_clip_rect(rect);
    let font = egui::FontId::monospace(CODE_SIZE);
    let ink = match line.kind {
        LineKind::Header => palette.accent_2,
        LineKind::Other => palette.muted,
        _ => palette.diff_line_ink(line.kind.prefix()),
    };
    let text_origin = egui::pos2(rect.min.x + GUTTER_WIDTH + 6.0, rect.center().y);

    // The prefix column stays fixed so code lines up whatever the change is.
    let prefix = match line.kind {
        LineKind::Added => "+",
        LineKind::Removed => "-",
        LineKind::Context => " ",
        _ => "",
    };
    if !prefix.is_empty() {
        painter.text(text_origin, Align2::LEFT_CENTER, prefix, font.clone(), ink);
    }

    let body_origin = egui::pos2(
        text_origin.x + if prefix.is_empty() { 0.0 } else { 9.0 },
        text_origin.y,
    );
    if !marks.is_empty() {
        draw_find_marks(&painter, rect, line, body_origin, &font, marks, palette);
    }

    let Some(words) = &line.words else {
        painter.text(body_origin, Align2::LEFT_CENTER, line.body(), font, ink);
        return;
    };

    // Word-level runs: the parts that actually changed get a tinted background so an edited
    // line reads as an edit rather than a wholesale replacement.
    let changed_bg = match line.kind {
        LineKind::Added => palette.added_word_bg,
        _ => palette.removed_word_bg,
    };
    let mut x = body_origin.x;
    for part in words {
        let galley = painter.layout_no_wrap(part.text.clone(), font.clone(), ink);
        let size = galley.size();
        if part.changed {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, rect.min.y + 1.0),
                    vec2(size.x, rect.height() - 2.0),
                ),
                CornerRadius::same(2),
                changed_bg,
            );
        }
        painter.galley(egui::pos2(x, rect.center().y - size.y / 2.0), galley, ink);
        x += size.x;
    }
}

/// What the find bar wants marked on one line: every match it covers, and which of them the
/// bar has stepped to.
#[derive(Default)]
pub(super) struct FindMarks {
    spans: Vec<(usize, usize)>,
    current: Option<(usize, usize)>,
}

impl FindMarks {
    fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

fn find_marks(
    app: &App,
    session_id: &str,
    hunk_id: &str,
    index: usize,
    line: &DiffLine,
) -> FindMarks {
    let Some(review) = app.model.review_ref(session_id) else {
        return FindMarks::default();
    };
    if review.find_query.is_empty() {
        return FindMarks::default();
    }
    FindMarks {
        spans: search::spans_in(line, &review.find_query),
        current: review
            .find_match
            .as_ref()
            .filter(|found| found.hunk_id == hunk_id && found.line_index == index)
            .map(|found| (found.column, found.width)),
    }
}

/// Paint the find bar's matches behind the text of a line.
///
/// Behind rather than through it: the line is drawn in word-diff runs, and a highlight that
/// had to be woven into those runs would have to agree with them about every boundary.
fn draw_find_marks(
    painter: &egui::Painter,
    rect: egui::Rect,
    line: &DiffLine,
    origin: egui::Pos2,
    font: &egui::FontId,
    marks: &FindMarks,
    palette: &Palette,
) {
    let body: Vec<char> = line.body().chars().collect();
    let width_of = |from: usize, to: usize| {
        let text: String = body[from.min(body.len())..to.min(body.len())].iter().collect();
        painter
            .layout_no_wrap(text, font.clone(), palette.ink)
            .size()
            .x
    };

    for (column, width) in &marks.spans {
        let left = origin.x + width_of(0, *column);
        let span = egui::Rect::from_min_size(
            egui::pos2(left, rect.min.y + 1.0),
            vec2(width_of(*column, column + width), rect.height() - 2.0),
        );
        // The same tint a text selection gets elsewhere in the window, which is strong
        // enough to pick a match out of a tinted diff line without hiding the code.
        painter.rect_filled(
            span,
            CornerRadius::same(2),
            palette.accent.linear_multiply(0.35),
        );
        // The one the bar has stepped to is outlined, so stepping through matches is
        // visible without the others disappearing.
        if marks.current == Some((*column, *width)) {
            painter.rect_stroke(
                span,
                CornerRadius::same(2),
                Stroke::new(1.0, palette.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}
