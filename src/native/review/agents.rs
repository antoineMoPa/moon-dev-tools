//! The comment agent monitor: every comment that has been handed to an agent, what its run
//! is doing, and its output.
//!
//! Mirrors `web/src/components/workspace/AgentMonitorPane.tsx`.

use egui::{Align, CornerRadius, Layout, RichText, Stroke, Ui};

use crate::{
    api::{CommentDispatchStatus, ReviewCommentView},
    native::{app::App, theme::{Palette, SMALL_SIZE}, widgets},
};

/// A comment whose agent run is worth watching, paired with the review it belongs to.
struct Watched {
    session_id: String,
    comment: ReviewCommentView,
}

fn status_rank(status: CommentDispatchStatus) -> u8 {
    // Running work first, then what is waiting, then what is finished.
    match status {
        CommentDispatchStatus::Running => 0,
        CommentDispatchStatus::Queued => 1,
        CommentDispatchStatus::Batched => 2,
        CommentDispatchStatus::Failed => 3,
        CommentDispatchStatus::Canceled => 4,
        CommentDispatchStatus::Completed => 5,
        CommentDispatchStatus::Idle => 6,
    }
}

fn status_colors(status: CommentDispatchStatus, palette: &Palette) -> (egui::Color32, egui::Color32) {
    match status {
        CommentDispatchStatus::Running | CommentDispatchStatus::Queued => {
            (palette.accent, palette.status_open_bg)
        }
        CommentDispatchStatus::Batched => (palette.snoozed, palette.status_neutral_bg),
        CommentDispatchStatus::Completed => (palette.accent_2, palette.status_resolved_bg),
        CommentDispatchStatus::Failed => (palette.warn, palette.status_failed_bg),
        CommentDispatchStatus::Canceled => (palette.muted, palette.status_neutral_bg),
        CommentDispatchStatus::Idle => (palette.muted, palette.status_neutral_bg),
    }
}

fn watched_comments(app: &App) -> Vec<Watched> {
    let mut watched: Vec<Watched> = app
        .model
        .reviews
        .values()
        .filter_map(|review| review.payload.as_ref().map(|payload| (review, payload)))
        .flat_map(|(review, payload)| {
            payload
                .review_comments
                .iter()
                .filter(|comment| comment.dispatch.status != CommentDispatchStatus::Idle)
                .map(|comment| Watched {
                    session_id: review.session_id.clone(),
                    comment: comment.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    watched.sort_by_key(|entry| {
        (
            status_rank(entry.comment.dispatch.status),
            entry.comment.file_path.clone(),
        )
    });
    watched
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui) {
    let palette = app.palette_of();
    let watched = watched_comments(app);

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("comment agents").strong());
                ui.label(
                    RichText::new(format!("{}", watched.len()))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if app.model.agent_log.is_some() && widgets::quiet_button(ui, "hide log").clicked()
                    {
                        app.model.agent_log = None;
                    }
                });
            });
            widgets::divider(ui, &palette);
            ui.add_space(5.0);

            if watched.is_empty() {
                ui.label(
                    RichText::new("no comment has been handed to an agent yet")
                        .color(palette.muted),
                );
                return;
            }

            // The list fills the pane, so an open log has to be given its share up front —
            // drawn after a list that had taken every pixel, it landed below the bottom edge
            // and was never seen.
            let log_height = app
                .model
                .agent_log
                .is_some()
                .then(|| (ui.available_height() * 0.45).clamp(120.0, 320.0));

            let list = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("agent-monitor-list");
            let list = match log_height {
                Some(height) => list.max_height((ui.available_height() - height).max(60.0)),
                None => list,
            };
            list.show(ui, |ui| {
                for entry in &watched {
                    draw_row(app, ui, entry, &palette);
                    ui.add_space(5.0);
                }
            });

            draw_log(app, ui, &palette);
        });
}

fn draw_row(app: &mut App, ui: &mut Ui, entry: &Watched, palette: &Palette) {
    let comment = &entry.comment;
    let (ink, fill) = status_colors(comment.dispatch.status, palette);
    let showing_log = app
        .model
        .agent_log
        .as_ref()
        .is_some_and(|log| log.dispatch_key == comment.dispatch.key);

    egui::Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, if showing_log { palette.accent } else { palette.line }))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::pill(
                    ui,
                    crate::native::review::sidebar::status_label(comment),
                    ink,
                    fill,
                );
                ui.label(
                    RichText::new(widgets::elide_path(&comment.file_path, 46))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                )
                .on_hover_text(&comment.file_path);
                if comment.dispatch.agent != crate::api::AgentKind::None {
                    ui.label(
                        RichText::new(comment.dispatch.agent.label())
                            .size(SMALL_SIZE)
                            .color(palette.accent_2),
                    );
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if comment.dispatch.can_cancel && widgets::quiet_button(ui, "cancel").clicked() {
                        let session_id = entry.session_id.clone();
                        let hunk_id = comment.hunk_id.clone();
                        let index = comment.comment_index;
                        app.tasks.act(
                            &entry.session_id,
                            "could not cancel the run",
                            move |backend| backend.cancel_dispatch(&session_id, &hunk_id, index),
                        );
                    }
                    if comment.dispatch.has_log
                        && widgets::quiet_button(ui, if showing_log { "hide log" } else { "log" })
                            .clicked()
                    {
                        if showing_log {
                            app.model.agent_log = None;
                        } else {
                            load_log(app, &entry.session_id, &comment.dispatch.key);
                        }
                    }
                    if widgets::quiet_button(ui, "go to").clicked() {
                        let hunk_id = comment.hunk_id.clone();
                        let review = app.model.review(&entry.session_id);
                        review.scroll_to_hunk = Some(hunk_id.clone());
                        review.active_hunk_id = Some(hunk_id);
                    }
                });
            });

            ui.label(RichText::new(&comment.comment).color(palette.ink));
            if !comment.dispatch.detail.trim().is_empty() {
                ui.label(
                    RichText::new(&comment.dispatch.detail)
                        .size(SMALL_SIZE - 1.0)
                        .color(palette.muted),
                );
            }
        });
}

fn load_log(app: &mut App, session_id: &str, dispatch_key: &str) {
    let for_call = session_id.to_string();
    let for_apply = session_id.to_string();
    let key = dispatch_key.to_string();
    app.tasks.spawn_keyed(
        Some(format!("log:{dispatch_key}")),
        move |backend| backend.dispatch_log(&for_call, &key),
        move |model, result| match result {
            Ok(payload) => model.set_agent_log(for_apply.clone(), payload),
            Err(error) => model.error(format!("could not read the agent log: {error}")),
        },
    );
}

fn draw_log(app: &mut App, ui: &mut Ui, palette: &Palette) {
    let Some(log) = app.model.agent_log.as_ref() else {
        return;
    };
    let text = log.text.clone();
    let key = log.dispatch_key.clone();
    let session_id = log.session_id.clone();

    ui.add_space(6.0);
    widgets::divider(ui, palette);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("agent output")
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::quiet_button(ui, "copy").clicked() {
                ui.ctx().copy_text(text.clone());
                app.model.info("agent output copied");
            }
            if widgets::quiet_button(ui, "refresh").clicked() {
                // A running agent keeps writing, so the log is pulled again on demand — from
                // the review the dispatch belongs to, which is not always the only one open.
                load_log(app, &session_id, &key);
            }
        });
    });

    egui::ScrollArea::vertical()
        .id_salt("agent-log")
        .max_height(220.0)
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(palette.code_bg)
                .inner_margin(egui::Margin::same(7))
                .show(ui, |ui| {
                    if text.trim().is_empty() {
                        ui.label(RichText::new("(nothing yet)").color(palette.muted));
                        return;
                    }
                    ui.label(
                        RichText::new(&text)
                            .monospace()
                            .size(crate::native::theme::CODE_SIZE - 1.0)
                            .color(palette.ink),
                    );
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_work_is_listed_before_finished_work() {
        let mut statuses = vec![
            CommentDispatchStatus::Completed,
            CommentDispatchStatus::Running,
            CommentDispatchStatus::Failed,
            CommentDispatchStatus::Queued,
        ];
        statuses.sort_by_key(|status| status_rank(*status));

        assert_eq!(
            statuses,
            vec![
                CommentDispatchStatus::Running,
                CommentDispatchStatus::Queued,
                CommentDispatchStatus::Failed,
                CommentDispatchStatus::Completed,
            ]
        );
    }
}
