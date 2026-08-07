//! The diff itself: unstaged hunks, then staged ones, grouped by file.
//!
//! Selecting lines is how everything specific happens here — a comment is anchored to the
//! lines it was written against, and a partial stage applies exactly those lines. That is
//! the same contract the web frontend's text selection has, expressed as line ranges.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Key, Rect, RichText, Sense, Stroke, Ui, vec2};

use crate::{
    api::HunkView,
    comments::{AnchoredComment, build_anchored_comment_value, parse_anchored_comments},
    native::{
        app::App,
        panes::OpenPaneRequest,
        model::{Draft, LineSelection, hash_of},
        palette::CommandAction,
        review::diff::{DiffLine, LineKind, insertion_line},
        review::image_diff,
        review::search,
        theme::{CODE_SIZE, Palette, SMALL_SIZE},
        widgets,
    },
};

const GUTTER_WIDTH: f32 = 74.0;
const LINE_HEIGHT: f32 = 15.0;

/// One diff line's widget id. Derived from the hunk and the line rather than from the
/// enclosing `Ui`, so it is the same wherever the line is drawn — which is also what lets the
/// tests find a line and click it.
pub(crate) fn diff_line_id(hunk_id: &str, index: usize) -> egui::Id {
    egui::Id::new(("moonreview-diff-line", hunk_id, index))
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    // The payload is shared, so the hunks below are read straight out of it rather than
    // copied — a diff of a lock file is far too much text to clone every frame.
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
    // own — `moonreview package.json` on a file nobody has touched is a request to read it.
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
        // Dragging is how lines get selected here, so it must not also mean "scroll" — not
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
                    is_commit_review,
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
                is_commit_review,
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
                    is_commit_review,
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
    start_draft(app, session_id, hunk, selection, String::new());
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
    is_commit_review: bool,
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
            is_commit_review,
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
    is_commit_review: bool,
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
            draw_hunk_toolbar(app, ui, session_id, hunk, read_only, is_commit_review, palette);
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

fn draw_hunk_toolbar(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    read_only: bool,
    is_commit_review: bool,
    palette: &Palette,
) {
    egui::Frame::new()
        .fill(palette.control_bg)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            // The actions get a line of their own above the header, so a long move hint or a
            // wide count never squeezes them out to the edge of the card.
            if !read_only || is_commit_review {
                ui.horizontal(|ui| {
                    draw_hunk_actions(app, ui, session_id, hunk, read_only, is_commit_review, palette);
                });
            }

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&hunk.header)
                        .monospace()
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );

                if hunk.added_line_count > 0 {
                    ui.label(
                        RichText::new(format!("+{}", widgets::grouped(hunk.added_line_count)))
                            .size(SMALL_SIZE)
                            .color(palette.added),
                    );
                }
                if hunk.removed_line_count > 0 {
                    ui.label(
                        RichText::new(format!("−{}", widgets::grouped(hunk.removed_line_count)))
                            .size(SMALL_SIZE)
                            .color(palette.removed),
                    );
                }
                if hunk.reviewed {
                    widgets::pill(ui, "reviewed", palette.accent_2, palette.status_resolved_bg);
                }

                if let Some(hint) = &hunk.moved_from {
                    moved_hint(app, ui, session_id, "moved from", hint, palette);
                }
                if let Some(hint) = &hunk.moved_to {
                    moved_hint(app, ui, session_id, "moved to", hint, palette);
                }
            });
        });
}

fn moved_hint(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    label: &str,
    hint: &crate::api::HunkMoveHint,
    palette: &Palette,
) {
    let text = format!("{label} {}", widgets::elide_path(&hint.target_file_path, 26));
    if widgets::quiet_button_colored(ui, &text, palette.snoozed)
        .on_hover_text(format!(
            "{} {}\n{}% similar — click to jump there",
            hint.target_file_path,
            hint.target_header,
            (hint.score * 100.0).round()
        ))
        .clicked()
    {
        let review = app.model.review(session_id);
        review.scroll_to_hunk = Some(hint.target_hunk_id.clone());
    }
}

fn draw_hunk_actions(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    read_only: bool,
    is_commit_review: bool,
    palette: &Palette,
) {
    if !read_only {
        if hunk.staged {
            if widgets::quiet_button(ui, "[unstage hunk]").clicked() {
                let hunk_id = hunk.id.clone();
                let for_call = session_id.to_string();
                app.tasks
                    .act(session_id, "could not unstage the hunk", move |backend| {
                        backend.unstage_hunk(&for_call, &hunk_id)
                    });
            }
        } else {
            if widgets::quiet_button(ui, "[stage hunk]").on_hover_text("stage this hunk (s)").clicked() {
                let hunk_id = hunk.id.clone();
                let for_call = session_id.to_string();
                app.tasks
                    .act(session_id, "could not stage the hunk", move |backend| {
                        backend.stage_hunk(&for_call, &hunk_id)
                    });
            }
        }

        let discarding = app
            .model
            .review_ref(session_id)
            .is_some_and(|review| review.pending_discard.as_deref() == Some(hunk.id.as_str()));
        if discarding {
            match widgets::confirm(
                ui,
                &palette,
                "[really discard]",
                "this throws the change away and cannot be undone",
            ) {
                widgets::Confirmed::Yes => {
                    app.model.review(session_id).pending_discard = None;
                    let hunk_id = hunk.id.clone();
                    let for_call = session_id.to_string();
                    app.tasks
                        .act(session_id, "could not discard the hunk", move |backend| {
                            backend.discard_hunk(&for_call, &hunk_id)
                        });
                }
                widgets::Confirmed::No => app.model.review(session_id).pending_discard = None,
                widgets::Confirmed::Waiting => {}
            }
        } else if widgets::quiet_button_colored(ui, "[discard hunk]", palette.warn).clicked() {
            // Discarding is destructive and has no undo, so it takes a second press.
            app.model.review(session_id).pending_discard = Some(hunk.id.clone());
        }
    }

    if is_commit_review || read_only {
        let next = !hunk.reviewed;
        if widgets::quiet_button(ui, if next { "[mark reviewed]" } else { "[mark unreviewed]" }).clicked() {
            let hunk_id = hunk.id.clone();
            let for_call = session_id.to_string();
            app.tasks
                .act(session_id, "could not mark the hunk", move |backend| {
                    backend.set_reviewed(&for_call, &hunk_id, Some(next))
                });
        }
    }

}

/// ⌘C over a diff: put the selected lines on the clipboard.
///
/// The lines a diff is read by are painted rather than laid out as text — a lock file is tens
/// of thousands of rows, and egui text that could be swept through character by character
/// would have to be laid out whether or not it is on screen. So the selection a diff already
/// has, the run of lines picked for a comment, is what copies.
///
/// What lands on the clipboard is the code without its `+`/`-`/space marker: someone copying
/// out of a diff is nearly always taking the code somewhere it has to compile.
fn copy_selected_lines(app: &mut App, ui: &Ui, session_id: &str) {
    // Looked for before the selection is gathered, which reads the patch: almost every frame
    // has no copy in it, and the answer must not cost anything on those.
    let asked = ui.input(|input| input.events.iter().any(|event| *event == egui::Event::Copy));
    if !asked {
        return;
    }

    let Some(hunk_id) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.active_hunk_id.clone())
    else {
        return;
    };
    let Some(selected) = current_selection(app, session_id, &hunk_id) else {
        return;
    };

    // Taken only now that there is something to copy, so a review with nothing selected
    // leaves the chord to whatever else on screen wants it — the composer's text box, or a
    // second review open beside this one.
    ui.ctx()
        .input_mut(|input| input.events.retain(|event| *event != egui::Event::Copy));

    let text: String = selected
        .lines()
        .map(|line| line.get(1..).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let lines = text.lines().count();
    ui.ctx().copy_text(text);
    app.model.info(format!(
        "copied {lines} line{}",
        if lines == 1 { "" } else { "s" }
    ));
}

/// The raw patch lines the user has selected in this hunk, if any.
fn current_selection(app: &mut App, session_id: &str, hunk_id: &str) -> Option<String> {
    let review = app.model.review_ref(session_id)?;
    let selection = review.selection?;
    if selection.hunk_id_hash != hash_of(hunk_id) {
        return None;
    }
    // The lines have to come from the same patch the user was clicking on, which is the
    // expanded one where the hunk was expanded.
    let patch = review
        .expanded_patches
        .get(hunk_id)
        .cloned()
        .or_else(|| {
            review
                .hunks()
                .iter()
                .find(|hunk| hunk.id == hunk_id)
                .map(|hunk| hunk.patch_preview.clone())
        })?;

    let lines = app.diff_lines(hunk_id, &patch);
    let selected: Vec<&str> = selection
        .range()
        .filter_map(|index| lines.get(index))
        .map(|line| line.text.as_str())
        .collect();
    if selected.is_empty() {
        return None;
    }
    Some(selected.join("\n"))
}

fn start_draft(
    app: &mut App,
    session_id: &str,
    hunk: &HunkView,
    selection: String,
    note: String,
) {
    // Focus only when the composer is new: re-focusing on every extend would fight the
    // keyboard while the user is still choosing lines.
    let focus = note.is_empty();
    app.model.review(session_id).draft = Some(Draft {
        hunk_id: hunk.id.clone(),
        file_path: hunk.file_path.clone(),
        header: hunk.header.clone(),
        selection,
        note,
        focus,
    });
}

fn draw_hunk_body(
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

    // The composer goes below the last selected line, so the run it is about stays together
    // above it. A saved comment is placed by matching its text, because by then the lines it
    // was written against may have moved; a draft still knows exactly which lines they are.
    let draft_at = app.model.review_ref(session_id).and_then(|review| {
        let draft = review.draft.as_ref().filter(|draft| draft.hunk_id == hunk.id)?;
        review
            .selection
            .filter(|selection| selection.hunk_id_hash == hash_of(&hunk.id))
            .map(|selection| *selection.range().end())
            .or_else(|| insertion_line(&lines, &draft.selection, &used))
    });

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
        if draft_at == Some(index) {
            draw_composer(app, ui, session_id, hunk, read_only, palette);
        }
    }

    // A draft anchored to lines the preview does not contain still has to be reachable.
    if draft_at.is_none()
        && app
            .model
            .review_ref(session_id)
            .and_then(|review| review.draft.as_ref())
            .is_some_and(|draft| draft.hunk_id == hunk.id)
    {
        draw_composer(app, ui, session_id, hunk, read_only, palette);
    }

    if hunk.patch_line_count > preview_limit && full_patch.is_none() {
        draw_truncation_notice(app, ui, session_id, hunk, preview_limit, palette);
    }
}

fn draw_truncation_notice(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    preview_limit: usize,
    palette: &Palette,
) {
    let hidden = hunk.patch_line_count.saturating_sub(preview_limit);
    egui::Frame::new()
        .fill(palette.control_active_bg)
        .inner_margin(egui::Margin::symmetric(7, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} more lines", widgets::grouped(hidden)))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                let busy = app.tasks.is_busy(&format!("patch:{}", hunk.id));
                if widgets::clickable(ui.add_enabled(
                    !busy,
                    egui::Button::new(if busy { "loading…" } else { "show the whole hunk" }),
                ))
                .clicked()
                {
                    load_full_patch(app, session_id, &hunk.id);
                }
            });
        });
}

fn load_full_patch(app: &mut App, session_id: &str, hunk_id: &str) {
    let for_call = session_id.to_string();
    let for_apply = session_id.to_string();
    let hunk_id = hunk_id.to_string();
    let for_state = hunk_id.clone();

    app.tasks.spawn_keyed(
        Some(format!("patch:{hunk_id}")),
        move |backend| backend.hunk_patch(&for_call, &hunk_id),
        move |model, result| match result {
            Ok(payload) => {
                model
                    .review(&for_apply)
                    .expanded_patches
                    .insert(for_state, payload.patch);
            }
            Err(error) => model.error(format!("could not load the hunk: {error}")),
        },
    );
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
            // text in the web frontend.
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );

    let selected = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selection)
        .is_some_and(|selection| {
            selection.hunk_id_hash == hash_of(&hunk.id) && selection.contains(index)
        });

    // Added and removed lines keep their own tint, and a selected one is tinted again on top
    // of it — so a selected removal still reads as a removal.
    if let Some(background) = palette.diff_line_bg(line.kind.prefix()) {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, background);
    }
    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, palette.line_target_bg);
        // A solid bar down the left edge: the tint alone is easy to miss against a diff that
        // is already coloured.
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, vec2(2.5, rect.height())),
            CornerRadius::ZERO,
            palette.accent,
        );
    }
    if response.hovered() && selectable && !selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, palette.diff_gutter_bg);
    }

    draw_gutter(ui, rect, line, palette);
    let marks = find_marks(app, session_id, &hunk.id, index, line);
    draw_line_text(ui, rect, line, palette, &marks);

    if !selectable {
        return;
    }

    // A drag sweeps a run of lines. It starts on the line pressed, and every line the pointer
    // passes over extends it — the drag belongs to the line it began on, so each other line
    // has to notice the pointer itself rather than wait for an event it will never get.
    let hunk_hash = hash_of(&hunk.id);
    if response.drag_started() {
        let review = app.model.review(session_id);
        review.selection = Some(LineSelection {
            hunk_id_hash: hunk_hash,
            anchor: index,
            head: index,
        });
        review.selecting_in = Some(hunk.id.clone());
        review.active_hunk_id = Some(hunk.id.clone());
        // The composer waits for the button to come up: opening it mid-sweep would take the
        // keyboard away while lines are still being chosen.
        review.draft = None;
        return;
    }

    let sweeping_here = app
        .model
        .review_ref(session_id)
        .is_some_and(|review| review.selecting_in.as_deref() == Some(hunk.id.as_str()));
    if sweeping_here {
        let pointer_here = ui
            .input(|input| input.pointer.interact_pos())
            .is_some_and(|at| rect.y_range().contains(at.y));
        if pointer_here {
            let review = app.model.review(session_id);
            if let Some(existing) = review.selection
                && existing.hunk_id_hash == hunk_hash
                && existing.head != index
            {
                review.selection = Some(LineSelection {
                    hunk_id_hash: hunk_hash,
                    anchor: existing.anchor,
                    head: index,
                });
            }
        }
        return;
    }

    if !response.clicked() {
        return;
    }

    // Selecting lines and writing a comment are one gesture, the way dragging over text in
    // the web frontend pops its composer: a click selects and opens the composer at once.
    let extend = ui.input(|input| input.modifiers.shift);
    let hunk_hash = hash_of(&hunk.id);
    let review = app.model.review(session_id);
    let next = match review.selection {
        // Shift-click grows the run, which is how a multi-line comment gets its anchor.
        Some(existing) if extend && existing.hunk_id_hash == hunk_hash => Some(LineSelection {
            hunk_id_hash: hunk_hash,
            anchor: existing.anchor,
            head: index,
        }),
        Some(existing)
            if existing.hunk_id_hash == hunk_hash
                && existing.anchor == index
                && existing.head == index =>
        {
            // Clicking the one selected line again puts the composer away.
            None
        }
        _ => Some(LineSelection {
            hunk_id_hash: hunk_hash,
            anchor: index,
            head: index,
        }),
    };
    review.selection = next;
    review.active_hunk_id = Some(hunk.id.clone());

    if next.is_none() {
        app.model.review(session_id).draft = None;
        return;
    }

    let Some(selection) = current_selection(app, session_id, &hunk.id) else {
        return;
    };
    // Extending a run keeps whatever has already been typed.
    let existing_note = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.draft.as_ref())
        .filter(|draft| draft.hunk_id == hunk.id)
        .map(|draft| draft.note.clone())
        .unwrap_or_default();
    start_draft(app, session_id, hunk, selection, existing_note);
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

/// What the find bar wants marked on one line: every match it covers, and which of them the
/// bar has stepped to.
#[derive(Default)]
struct FindMarks {
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

fn draw_line_text(
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

fn draw_inline_comment(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    comment_index: usize,
    entry: &AnchoredComment,
    palette: &Palette,
) {
    let dispatch = hunk.comment_dispatches.get(comment_index).cloned();

    egui::Frame::new()
        .fill(palette.inline_comment_bg)
        .stroke(Stroke::new(1.0, palette.inline_comment_border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .outer_margin(egui::Margin {
            left: GUTTER_WIDTH as i8 + 6,
            right: 6,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if entry.resolved { "resolved" } else { "comment" })
                        .size(SMALL_SIZE - 1.0)
                        .color(if entry.resolved {
                            palette.accent_2
                        } else {
                            palette.accent
                        }),
                );

                if let Some(dispatch) = &dispatch
                    && dispatch.status != crate::api::CommentDispatchStatus::Idle
                {
                    let label = format!("{} · {}", dispatch.agent.label(), dispatch.detail);
                    let label = label.trim_end_matches(" · ");
                    if dispatch.status == crate::api::CommentDispatchStatus::Batched {
                        widgets::pill(ui, label, palette.ink, palette.batch_bg);
                    } else {
                        ui.label(
                            RichText::new(label)
                                .size(SMALL_SIZE - 1.0)
                                .color(palette.muted),
                        );
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(dispatch) = &dispatch {
                        if dispatch.can_cancel && widgets::quiet_button(ui, "cancel").clicked() {
                            let hunk_id = hunk.id.clone();
                            let for_call = session_id.to_string();
                            app.tasks.act(
                                session_id,
                                "could not cancel the run",
                                move |backend| {
                                    backend.cancel_dispatch(&for_call, &hunk_id, comment_index)
                                },
                            );
                        }
                        if dispatch.has_log && widgets::quiet_button(ui, "log").clicked() {
                            open_dispatch_log(app, session_id, &dispatch.key);
                        }
                    }
                    if !entry.resolved && widgets::quiet_button(ui, "resolve").clicked() {
                        let hunk_id = hunk.id.clone();
                        let for_call = session_id.to_string();
                        app.tasks.act(
                            session_id,
                            "could not resolve the comment",
                            move |backend| {
                                backend.resolve_comment(&for_call, &hunk_id, comment_index)
                            },
                        );
                    }
                    if widgets::quiet_button_colored(ui, "delete", palette.warn).clicked() {
                        delete_comment(app, session_id, hunk, comment_index);
                    }
                });
            });

            ui.label(RichText::new(&entry.comment).color(palette.ink));
        });
}

fn delete_comment(app: &mut App, session_id: &str, hunk: &HunkView, comment_index: usize) {
    let mut anchored = parse_anchored_comments(&hunk.comment);
    if comment_index >= anchored.len() {
        return;
    }
    anchored.remove(comment_index);
    let comment = build_anchored_comment_value(&anchored);

    let hunk_id = hunk.id.clone();
    let for_call = session_id.to_string();
    app.tasks
        .act(session_id, "could not delete the comment", move |backend| {
            backend.set_comment(
                &for_call,
                crate::api::CommentRequest {
                    hunk_id,
                    comment,
                    batch: false,
                },
            )
        });
}

fn open_dispatch_log(app: &mut App, session_id: &str, dispatch_key: &str) {
    // The agent monitor is what shows a log, so asking for one from a comment card has to
    // bring that pane up — otherwise the click loads a log nothing is drawing.
    app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::Agents));

    let for_call = session_id.to_string();
    let for_apply = session_id.to_string();
    let dispatch_key = dispatch_key.to_string();
    app.tasks.spawn(
        move |backend| backend.dispatch_log(&for_call, &dispatch_key),
        move |model, result| match result {
            Ok(payload) => model.set_agent_log(for_apply.clone(), payload),
            Err(error) => model.error(format!("could not read the agent log: {error}")),
        },
    );
}

fn draw_composer(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    read_only: bool,
    palette: &Palette,
) {
    let Some(mut draft) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.draft.clone())
        .filter(|draft| draft.hunk_id == hunk.id)
    else {
        return;
    };
    let agent = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.as_ref())
        .map(|payload| payload.selected_agent)
        .unwrap_or_default();

    let mut send = false;
    let mut batch = false;
    let mut cancel = false;

    egui::Frame::new()
        .fill(palette.composer_bg)
        .stroke(Stroke::new(1.0, palette.accent))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .outer_margin(egui::Margin {
            left: GUTTER_WIDTH as i8 + 6,
            right: 6,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            let line_count = draft.selection.lines().count();
            ui.label(
                RichText::new(format!(
                    "{} {} · {line_count} line{}",
                    draft.file_path,
                    draft.header,
                    if line_count == 1 { "" } else { "s" }
                ))
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted),
            );

            let entry = ui.add(
                egui::TextEdit::multiline(&mut draft.note)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("what should change here?"),
            );
            if draft.focus {
                entry.request_focus();
                draft.focus = false;
            }

            // ⌘return sends without reaching for the mouse, which is how these get written.
            if entry.has_focus()
                && ui.input(|input| input.key_pressed(Key::Enter) && input.modifiers.command)
            {
                send = true;
            }
            if ui.input(|input| input.key_pressed(Key::Escape)) {
                cancel = true;
            }

            ui.horizontal(|ui| {
                let ready = !draft.note.trim().is_empty();
                let has_agent = agent != crate::api::AgentKind::None;

                // Saving a comment with an agent selected hands it over there and then; that
                // is what the server does, so the button has to say so.
                let (label, hint) = if has_agent {
                    (
                        format!("send to {}", agent.label()),
                        "write this comment and hand it over now (⌘return)",
                    )
                } else {
                    (
                        "save".to_string(),
                        "keep the comment in the review (⌘return)",
                    )
                };
                if widgets::clickable(ui.add_enabled(ready, egui::Button::new(label)))
                    .on_hover_text(hint)
                    .clicked()
                {
                    send = true;
                }

                // The other path holds the comment back so a batch of them can go at once.
                if has_agent
                    && widgets::clickable(
                        ui.add_enabled(ready, egui::Button::new("hold for batch")),
                    )
                    .on_hover_text("keep it back, to send with the others from the header")
                    .clicked()
                {
                    batch = true;
                }
                if widgets::clickable(ui.button("cancel")).clicked() {
                    cancel = true;
                }
            });
            let _ = read_only;
        });

    if cancel {
        app.model.review(session_id).draft = None;
        return;
    }

    if !send && !batch {
        app.model.review(session_id).draft = Some(draft);
        return;
    }
    if draft.note.trim().is_empty() {
        app.model.review(session_id).draft = Some(draft);
        return;
    }

    // A saved comment joins the ones already on the hunk rather than replacing them.
    let mut anchored = parse_anchored_comments(&hunk.comment);
    anchored.push(AnchoredComment {
        selection: draft.selection.clone(),
        comment: draft.note.trim().to_string(),
        resolved: false,
    });
    let comment = build_anchored_comment_value(&anchored);

    app.model.review(session_id).draft = None;
    app.model.review(session_id).selection = None;

    let hunk_id = hunk.id.clone();
    let for_call = session_id.to_string();
    app.tasks
        .act(session_id, "could not save the comment", move |backend| {
            backend.set_comment(
                &for_call,
                crate::api::CommentRequest {
                    hunk_id,
                    comment,
                    batch,
                },
            )
        });
}
