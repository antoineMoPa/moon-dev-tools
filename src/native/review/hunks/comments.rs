//! Comments drawn under the lines they were written against, and the composer that writes
//! them.

use egui::{CornerRadius, Key, RichText, Stroke, Ui};

use crate::{
    api::HunkView,
    comments::{AnchoredComment, build_anchored_comment_value, parse_anchored_comments},
    native::{
        app::App,
        panes::OpenPaneRequest,
        model::{Draft, hash_of},
        palette::CommandAction,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

use super::GUTTER_WIDTH;

pub(super) fn draw_inline_comment(
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
    // bring that pane up - otherwise the click loads a log nothing is drawing.
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

/// One composer, identified by the hunk and the anchor text of the draft it edits - several
/// can be on screen at once, so an index would go stale the moment one of them closes.
pub(super) fn draw_composer(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    hunk: &HunkView,
    anchor: &str,
    read_only: bool,
    palette: &Palette,
) {
    let Some(mut draft) = app
        .model
        .review_ref(session_id)
        .and_then(|review| {
            review
                .drafts
                .iter()
                .find(|draft| draft.hunk_id == hunk.id && draft.selection == anchor)
                .cloned()
        })
    else {
        return;
    };
    let payload = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.clone());
    let agent = payload
        .as_ref()
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
                    // An id of its own, tied to what the comment is about. The default id is
                    // positional, and anything shifting the layout above the composer - a
                    // poll refresh, a hunk staged, a comment card appearing - would hand the
                    // box a new id and take the keyboard away mid-sentence.
                    .id(egui::Id::new((
                        "moonreview-composer",
                        session_id,
                        hunk.id.as_str(),
                        hash_of(anchor),
                    )))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("what should change here?"),
            );
            if draft.focus {
                entry.request_focus();
                draft.focus = false;
            }
            // Typing again is its own answer to "discard?".
            if entry.changed() {
                draft.pending_discard = false;
            }

            // cmd+return sends without reaching for the mouse, which is how these get written.
            if entry.has_focus()
                && ui.input(|input| input.key_pressed(Key::Enter) && input.modifiers.command)
            {
                send = true;
            }
            // Escape belongs to the composer only while the keyboard is in it - an Escape
            // aimed at the palette, the find bar, or a terminal in the next split must not
            // reach in here. It closes an empty composer; over typed text it only puts the
            // keyboard down (the box surrenders focus itself), and the cancel button stays
            // the deliberate way to throw text away.
            if (entry.has_focus() || entry.lost_focus())
                && ui.input(|input| input.key_pressed(Key::Escape))
                && draft.note.trim().is_empty()
            {
                cancel = true;
            }

            ui.horizontal(|ui| {
                let ready = !draft.note.trim().is_empty();
                let has_agent = agent != crate::api::AgentKind::None;

                // The comment goes wherever this says, so the choice sits beside the button
                // that sends it.
                if let Some(payload) = payload.as_ref() {
                    crate::native::review::header::draw_agent_select(
                        app,
                        ui,
                        session_id,
                        hash_of(anchor),
                        agent,
                        &payload.available_agents,
                        palette,
                    );
                }

                // Saving a comment with an agent selected hands it over there and then; that
                // is what the server does, so the button has to say so.
                let (label, hint) = if has_agent {
                    (
                        format!("send to {}", agent.label()),
                        "write this comment and hand it over now (cmd+return)",
                    )
                } else {
                    (
                        "save".to_string(),
                        "keep the comment in the review (cmd+return)",
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
                // Throwing typed text away takes two presses: the first one only asks.
                if draft.pending_discard {
                    match widgets::confirm(
                        ui,
                        palette,
                        "[discard the comment]",
                        "throw the typed text away",
                    ) {
                        widgets::Confirmed::Yes => cancel = true,
                        widgets::Confirmed::No => draft.pending_discard = false,
                        widgets::Confirmed::Waiting => {}
                    }
                } else if widgets::clickable(ui.button("cancel")).clicked() {
                    if draft.note.trim().is_empty() {
                        cancel = true;
                    } else {
                        draft.pending_discard = true;
                    }
                }
            });
            let _ = read_only;
        });

    let remove_this_draft = |app: &mut App| {
        app.model
            .review(session_id)
            .drafts
            .retain(|draft| !(draft.hunk_id == hunk.id && draft.selection == anchor));
    };
    let keep_this_draft = |app: &mut App, draft: Draft| {
        let review = app.model.review(session_id);
        if let Some(existing) = review
            .drafts
            .iter_mut()
            .find(|draft| draft.hunk_id == hunk.id && draft.selection == anchor)
        {
            *existing = draft;
        }
    };

    if cancel {
        remove_this_draft(app);
        return;
    }

    if (!send && !batch) || draft.note.trim().is_empty() {
        keep_this_draft(app, draft);
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

    remove_this_draft(app);
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
