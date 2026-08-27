//! The toolbar above a hunk, and what its buttons do: staging, copying the selected
//! lines, and starting a comment on them.

use egui::{RichText, Ui};

use crate::{
    api::HunkView,
    native::{
        app::App,
        model::{Draft, LineSelection, hash_of},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

pub(super) fn draw_hunk_toolbar(
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
            "{} {}\n{}% similar - click to jump there",
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

pub(super) fn draw_truncation_notice(
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

/// cmd+c over a diff: put the selected characters on the clipboard.
///
/// The lines a diff is read by are painted rather than laid out as text - a lock file is
/// tens of thousands of rows, and egui text that egui itself could sweep through would have
/// to be laid out whether or not it is on screen. So the diff's own selection is what
/// copies: exactly the characters it covers, a swept word as that word, a clicked line as
/// the whole line.
///
/// What lands on the clipboard is the code without its `+`/`-`/space marker: someone copying
/// out of a diff is nearly always taking the code somewhere it has to compile.
pub(super) fn copy_selected_lines(app: &mut App, ui: &Ui, session_id: &str) {
    // Looked for before the selection is gathered, which reads the patch: almost every frame
    // has no copy in it, and the answer must not cost anything on those.
    let asked = ui.input(|input| input.events.iter().any(|event| *event == egui::Event::Copy));
    if !asked {
        return;
    }
    // A copy pressed while the keyboard is in a shell belongs to that shell: the selection
    // the user just made is in there, and the terminal answers the chord itself. Any other
    // focus - the composer's box, say - leaves the chord to the diff as it always has.
    let keyboard_in_a_shell = ui
        .ctx()
        .memory(|memory| memory.focused())
        .is_some_and(|focused| app.model.terminal_with_keyboard == Some(focused));
    if keyboard_in_a_shell {
        return;
    }

    let Some(hunk_id) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.active_hunk_id.clone())
    else {
        return;
    };
    let Some(selection) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.selection)
        .filter(|selection| selection.hunk_id_hash == hash_of(&hunk_id))
    else {
        return;
    };
    // A zero-width selection covers no characters; the chord stays with whoever else wants it.
    if selection.anchor == selection.head {
        return;
    }
    let Some(patch) = selected_patch(app, session_id, &hunk_id) else {
        return;
    };

    // Exactly the characters the selection covers: a swept word comes out as that word, a
    // clicked line as the whole line without its `+`/`-`/space marker - someone copying out
    // of a diff is nearly always taking the code somewhere it has to compile.
    let lines = app.diff_lines(&hunk_id, &patch);
    let covered: Vec<String> = selection
        .line_range()
        .filter_map(|index| {
            let line = lines.get(index)?;
            let (from, to) = selection.columns_on(index)?;
            let body: Vec<char> = line.body().chars().collect();
            let from = from.min(body.len());
            let to = to.min(body.len());
            Some(body[from..to].iter().collect())
        })
        .collect();
    if covered.is_empty() {
        return;
    }

    // Taken only now that there is something to copy, so a review with nothing selected
    // leaves the chord to whatever else on screen wants it - the composer's text box, or a
    // second review open beside this one.
    ui.ctx()
        .input_mut(|input| input.events.retain(|event| *event != egui::Event::Copy));

    ui.ctx().copy_text(covered.join("\n"));
}

/// The patch the selection's indices point into: the expanded one where the hunk was
/// expanded, the preview otherwise.
fn selected_patch(app: &App, session_id: &str, hunk_id: &str) -> Option<String> {
    let review = app.model.review_ref(session_id)?;
    review.expanded_patches.get(hunk_id).cloned().or_else(|| {
        review
            .hunks()
            .iter()
            .find(|hunk| hunk.id == hunk_id)
            .map(|hunk| hunk.patch_preview.clone())
    })
}

/// The raw patch lines the user has selected in this hunk, if any.
pub(super) fn current_selection(app: &mut App, session_id: &str, hunk_id: &str) -> Option<String> {
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
        .line_range()
        .filter_map(|index| lines.get(index))
        .map(|line| line.text.as_str())
        .collect();
    if selected.is_empty() {
        return None;
    }
    Some(selected.join("\n"))
}

/// Open a composer on this anchor.
///
/// Composers that have nothing typed in them are put away first - an unwritten box was a
/// place to type, not a comment. Typed ones stay parked exactly where they are: a comment
/// being written never moves and is never thrown away by selecting somewhere else.
pub(super) fn open_draft(app: &mut App, session_id: &str, hunk: &HunkView, selection: String) {
    let review = app.model.review(session_id);
    review.drafts.retain(|draft| !draft.note.trim().is_empty());
    if let Some(existing) = review
        .drafts
        .iter_mut()
        .find(|draft| draft.hunk_id == hunk.id && draft.selection == selection)
    {
        // The composer for this very anchor is already open: give it the keyboard back.
        existing.focus = true;
        return;
    }
    review.drafts.push(Draft {
        hunk_id: hunk.id.clone(),
        file_path: hunk.file_path.clone(),
        header: hunk.header.clone(),
        selection,
        note: String::new(),
        focus: true,
        pending_discard: false,
    });
}

/// Make a selection current and open a composer on it. Composers already open stay as they
/// are - see `open_draft`.
pub(super) fn select_and_open(app: &mut App, session_id: &str, hunk: &HunkView, selection: LineSelection) {
    let review = app.model.review(session_id);
    review.selection = Some(selection);
    review.active_hunk_id = Some(hunk.id.clone());

    let Some(anchor) = current_selection(app, session_id, &hunk.id) else {
        return;
    };
    open_draft(app, session_id, hunk, anchor);
}
