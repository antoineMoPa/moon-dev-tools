//! The review's left column: files, commits, and comments.
//!
//! Mirrors `web/src/components/LeftSidebar.tsx` — the same three sections, in the same order,
//! each row doing the same thing when clicked.

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Ui, vec2};

use crate::{
    api::{CommitReviewStatus, CommitView},
    native::{
        app::App,
        model::SidebarTab,
        review::files::{
            FileStageStatus, SidebarFile, build_sidebar_files, local_changes_summary,
        },
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
    service::HISTORY_COMMIT_PAGE_SIZE,
};

pub(crate) fn draw(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    let Some(payload) = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.clone())
    else {
        return;
    };

    let files = build_sidebar_files(&payload);
    let read_only = payload.read_only;
    let is_commit_review = payload.active_commit.is_some();
    let commit_base = payload.commit_base.as_deref();
    let active_commit = payload.active_commit.as_deref();
    let local_summary = local_changes_summary(payload.local_change_summary);
    let commits = &payload.commits;
    let comments = &payload.review_comments;

    let tab = app
        .model
        .review_ref(session_id)
        .map(|review| review.sidebar_tab)
        .unwrap_or(SidebarTab::Files);

    ui.horizontal(|ui| {
        if widgets::segment(ui, tab == SidebarTab::Files, "files", palette).clicked() {
            app.model.review(session_id).sidebar_tab = SidebarTab::Files;
        }
        let comment_label = if comments.is_empty() {
            "comments".to_string()
        } else {
            format!("comments {}", comments.iter().filter(|c| !c.resolved).count())
        };
        if widgets::segment(ui, tab == SidebarTab::Comments, &comment_label, palette).clicked() {
            app.model.review(session_id).sidebar_tab = SidebarTab::Comments;
        }
    });


    ui.add_space(5.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            SidebarTab::Files => {
                draw_files_section(app, ui, session_id, &files, read_only, is_commit_review, palette);
                ui.add_space(10.0);
                widgets::divider(ui, palette);
                ui.add_space(8.0);
                draw_commits_section(
                    app,
                    ui,
                    session_id,
                    commits,
                    commit_base,
                    active_commit,
                    &local_summary,
                    palette,
                );
            }
            SidebarTab::Comments => draw_comments_section(app, ui, session_id, comments, palette),
        });
}

fn draw_files_section(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    files: &[SidebarFile],
    read_only: bool,
    is_commit_review: bool,
    palette: &Palette,
) {
    let show_reviewed = app
        .model
        .review_ref(session_id)
        .map(|review| review.show_reviewed)
        .unwrap_or(true);

    let visible: Vec<&SidebarFile> = files
        .iter()
        .filter(|file| show_reviewed || !file.reviewed)
        .collect();

    widgets::section_header(ui, "files", palette, |ui| {
        let mut toggle = show_reviewed;
        if ui
            .add(egui::Checkbox::new(&mut toggle, RichText::new("done").size(SMALL_SIZE - 1.0)))
            .on_hover_text("show files whose hunks are all reviewed")
            .changed()
        {
            app.model.review(session_id).show_reviewed = toggle;
        }
    });
    ui.add_space(3.0);

    if visible.is_empty() {
        ui.label(RichText::new("nothing to review here").color(palette.muted));
        return;
    }

    for file in visible {
        draw_file_row(app, ui, session_id, file, read_only, is_commit_review, palette);
    }
}

fn stage_status_color(status: FileStageStatus, palette: &Palette) -> Color32 {
    match status {
        FileStageStatus::Staged => palette.staged,
        FileStageStatus::Unstaged => palette.unstaged,
        FileStageStatus::Partial => palette.partial,
    }
}

fn draw_file_row(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    file: &SidebarFile,
    read_only: bool,
    is_commit_review: bool,
    palette: &Palette,
) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let hovered = response.hovered();

    if ui.is_rect_visible(rect) {
        if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), palette.control_active_bg);
        }

        // The dot on the left is the file's staging state at a glance.
        let dot = egui::pos2(rect.min.x + 6.0, rect.min.y + 10.0);
        ui.painter()
            .circle_filled(dot, 3.0, stage_status_color(file.status, palette));

        let name_ink = if file.reviewed {
            palette.muted
        } else {
            palette.ink
        };
        ui.painter().text(
            egui::pos2(rect.min.x + 15.0, rect.min.y + 3.0),
            Align2::LEFT_TOP,
            widgets::elide_path(&file.file_name, 30),
            egui::FontId::proportional(crate::native::theme::UI_SIZE),
            name_ink,
        );

        let mut detail = String::new();
        if file.added_line_count > 0 {
            detail.push_str(&format!("+{}", widgets::grouped(file.added_line_count)));
        }
        if file.removed_line_count > 0 {
            if !detail.is_empty() {
                detail.push(' ');
            }
            detail.push_str(&format!("−{}", widgets::grouped(file.removed_line_count)));
        }
        if let Some(from) = &file.moved_from_file_path {
            detail.push_str(&format!("  from {}", widgets::elide_path(from, 22)));
        } else if let Some(to) = &file.moved_to_file_path {
            detail.push_str(&format!("  to {}", widgets::elide_path(to, 22)));
        }
        ui.painter().text(
            egui::pos2(rect.min.x + 15.0, rect.min.y + 17.0),
            Align2::LEFT_TOP,
            detail,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            palette.muted,
        );

        let reviewed_note = format!("{}/{}", file.reviewed_hunk_count, file.hunk_count);
        ui.painter().text(
            egui::pos2(rect.max.x - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            reviewed_note,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            if file.reviewed {
                palette.accent_2
            } else {
                palette.muted
            },
        );
    }

    let hover_text = format!(
        "{}\n{} hunk{}, {} reviewed",
        file.file_path,
        file.hunk_count,
        if file.hunk_count == 1 { "" } else { "s" },
        file.reviewed_hunk_count
    );
    let response = response.on_hover_text(hover_text);

    if response.clicked() {
        let review = app.model.review(session_id);
        review.scroll_to_hunk = review
            .hunks()
            .iter()
            .find(|hunk| hunk.file_path == file.file_path)
            .map(|hunk| hunk.id.clone());
    }

    // Right-clicking a file offers what can be done to all of it at once.
    response.context_menu(|ui| {
        if is_commit_review || read_only {
            let next = !file.reviewed;
            if ui
                .button(if next {
                    "mark the file reviewed"
                } else {
                    "mark the file unreviewed"
                })
                .clicked()
            {
                let path = file.file_path.clone();
                let for_call = session_id.to_string();
                app.tasks
                    .act(session_id, "could not mark the file", move |backend| {
                        backend.set_file_reviewed(&for_call, &path, next)
                    });
                ui.close();
            }
            return;
        }

        if file.status != FileStageStatus::Staged && ui.button("stage the whole file").clicked() {
            let path = file.file_path.clone();
            let for_call = session_id.to_string();
            app.tasks
                .act(session_id, "could not stage the file", move |backend| {
                    backend.stage_file(&for_call, &path)
                });
            ui.close();
        }
        if file.status != FileStageStatus::Unstaged && ui.button("unstage the whole file").clicked() {
            let path = file.file_path.clone();
            let for_call = session_id.to_string();
            app.tasks
                .act(session_id, "could not unstage the file", move |backend| {
                    backend.unstage_file(&for_call, &path)
                });
            ui.close();
        }
        let next = !file.reviewed;
        if ui
            .button(if next {
                "mark the file reviewed"
            } else {
                "mark the file unreviewed"
            })
            .clicked()
        {
            let path = file.file_path.clone();
            let for_call = session_id.to_string();
            app.tasks
                .act(session_id, "could not mark the file", move |backend| {
                    backend.set_file_reviewed(&for_call, &path, next)
                });
            ui.close();
        }

        ui.separator();
        // Discarding a whole file goes through the batch endpoint so the file is either
        // fully reverted or left alone, rather than half-reverted on a mid-way failure.
        let confirming = app
            .model
            .review_ref(session_id)
            .is_some_and(|review| review.pending_discard.as_deref() == Some(file.file_path.as_str()));
        if confirming {
            if ui
                .button(RichText::new("really discard the whole file").color(palette.warn))
                .clicked()
            {
                let hunk_ids = app
                    .model
                    .review_ref(session_id)
                    .map(|review| {
                        review
                            .hunks()
                            .iter()
                            .filter(|hunk| hunk.file_path == file.file_path)
                            .map(|hunk| hunk.id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                app.model.review(session_id).pending_discard = None;
                let for_call = session_id.to_string();
                app.tasks
                    .act(session_id, "could not discard the file", move |backend| {
                        backend.discard_hunks(&for_call, &hunk_ids)
                    });
                ui.close();
            }
            if ui.button("keep it").clicked() {
                app.model.review(session_id).pending_discard = None;
                ui.close();
            }
        } else if ui
            .button(RichText::new("discard the whole file").color(palette.warn))
            .clicked()
        {
            app.model.review(session_id).pending_discard = Some(file.file_path.clone());
        }
    });
}

fn draw_commits_section(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    commits: &[CommitView],
    base: Option<&str>,
    active_commit: Option<&str>,
    local_summary: &str,
    palette: &Palette,
) {
    widgets::section_header(ui, "commits", palette, |ui| {
        if let Some(base) = base {
            ui.label(RichText::new(base).size(SMALL_SIZE - 1.0).color(palette.muted));
        }
    });
    ui.add_space(3.0);

    // "local changes" is the working-tree review, which is where moonreview starts.
    if draw_commit_row(
        ui,
        "local changes",
        None,
        local_summary,
        active_commit.is_none(),
        None,
        palette,
    ) {
        select_commit(app, session_id, None);
    }

    if commits.is_empty() {
        let message = match base {
            Some(base) => format!("no commits since {base}"),
            None => "no default branch found".to_string(),
        };
        ui.label(RichText::new(message).color(palette.muted));
    }
    for commit in commits {
        if draw_commit_row(
            ui,
            &commit.subject,
            Some(&commit.short_sha),
            &commit.author,
            active_commit == Some(commit.sha.as_str()),
            Some(commit.review_status),
            palette,
        ) {
            select_commit(app, session_id, Some(commit.sha.clone()));
        }
    }

    let (history, has_more, loading) = app
        .model
        .review_ref(session_id)
        .map(|review| {
            (
                review.history_loaded.clone(),
                review.history_has_more,
                review.loading_history,
            )
        })
        .unwrap_or_default();

    if history.is_empty() {
        return;
    }

    ui.add_space(8.0);
    widgets::section_header(ui, "history", palette, |ui| {
        ui.label(
            RichText::new(history.len().to_string())
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted),
        );
    });
    ui.add_space(3.0);

    for commit in &history {
        if draw_commit_row(
            ui,
            &commit.subject,
            Some(&commit.short_sha),
            &commit.author,
            active_commit == Some(commit.sha.as_str()),
            Some(commit.review_status),
            palette,
        ) {
            select_commit(app, session_id, Some(commit.sha.clone()));
        }
    }

    if !has_more {
        return;
    }
    ui.add_space(3.0);
    let button = ui.add_enabled(
        !loading,
        egui::Button::new(if loading { "loading…" } else { "show more" }).frame(false),
    );
    if button.clicked() {
        load_more_history(app, session_id, history.len());
    }
}

fn select_commit(app: &mut App, session_id: &str, commit: Option<String>) {
    // The commit is what the review is *of*, so switching it resets what the pane shows.
    {
        let review = app.model.review(session_id);
        review.history_loaded.clear();
        review.active_hunk_id = None;
        review.selection = None;
        review.draft = None;
        review.expanded_patches.clear();
    }
    let for_call = session_id.to_string();
    app.tasks
        .act(session_id, "could not switch commit", move |backend| {
            backend.set_active_commit(&for_call, commit)
        });
}

fn load_more_history(app: &mut App, session_id: &str, offset: usize) {
    app.model.review(session_id).loading_history = true;
    let for_call = session_id.to_string();
    let for_apply = session_id.to_string();
    app.tasks.spawn_keyed(
        Some(format!("history:{session_id}")),
        move |backend| backend.commit_history(&for_call, offset, HISTORY_COMMIT_PAGE_SIZE),
        move |model, result| {
            let review = model.review(&for_apply);
            review.loading_history = false;
            match result {
                Ok(page) => {
                    review.history_loaded.extend(page.commits);
                    review.history_has_more = page.has_more;
                }
                Err(error) => model.error(format!("could not load more history: {error}")),
            }
        },
    );
}

/// One selectable commit. Returns whether it was chosen.
fn draw_commit_row(
    ui: &mut Ui,
    subject: &str,
    short_sha: Option<&str>,
    meta: &str,
    active: bool,
    status: Option<CommitReviewStatus>,
    palette: &Palette,
) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 31.0), Sense::click());

    if ui.is_rect_visible(rect) {
        if active {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), palette.control_active_bg);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), palette.control_bg);
        }

        let sha_width = short_sha
            .map(|sha| {
                ui.painter().text(
                    egui::pos2(rect.max.x - 5.0, rect.min.y + 3.0),
                    Align2::RIGHT_TOP,
                    sha,
                    egui::FontId::monospace(SMALL_SIZE - 1.0),
                    palette.muted,
                );
                58.0
            })
            .unwrap_or(0.0);

        let available = (rect.width() - 12.0 - sha_width).max(40.0);
        let galley = ui.painter().layout(
            subject.to_string(),
            egui::FontId::proportional(crate::native::theme::UI_SIZE),
            if active { palette.accent } else { palette.ink },
            available,
        );
        ui.painter().galley(
            egui::pos2(rect.min.x + 6.0, rect.min.y + 3.0),
            galley,
            palette.ink,
        );

        ui.painter().text(
            egui::pos2(rect.min.x + 6.0, rect.min.y + 18.0),
            Align2::LEFT_TOP,
            meta,
            egui::FontId::proportional(SMALL_SIZE - 1.0),
            palette.muted,
        );

        if let Some(status) = status {
            let (text, ink) = match status {
                CommitReviewStatus::Reviewed => ("reviewed", palette.accent_2),
                CommitReviewStatus::Partial => ("partial", palette.partial),
                CommitReviewStatus::Unreviewed => ("", palette.muted),
            };
            if !text.is_empty() {
                ui.painter().text(
                    egui::pos2(rect.max.x - 5.0, rect.min.y + 18.0),
                    Align2::RIGHT_TOP,
                    text,
                    egui::FontId::proportional(SMALL_SIZE - 2.0),
                    ink,
                );
            }
        }
    }

    let hover = match short_sha {
        Some(sha) => format!("{sha} {subject}"),
        None => subject.to_string(),
    };
    response.on_hover_text(hover).clicked()
}

fn draw_comments_section(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    comments: &[crate::api::ReviewCommentView],
    palette: &Palette,
) {
    if comments.is_empty() {
        ui.label(RichText::new("no comments yet").color(palette.muted));
        ui.add_space(6.0);
        ui.label(
            RichText::new("select lines in a hunk and press c to write one")
                .size(SMALL_SIZE - 1.0)
                .color(palette.muted),
        );
        return;
    }

    for comment in comments {
        let status = status_label(comment);
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), 46.0),
            Sense::click(),
        );

        if ui.is_rect_visible(rect) {
            let fill = if comment.resolved {
                palette.status_resolved_bg
            } else {
                palette.status_open_bg
            };
            ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
            if response.hovered() {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    egui::Stroke::new(1.0, palette.accent),
                    egui::StrokeKind::Inside,
                );
            }

            ui.painter().text(
                egui::pos2(rect.min.x + 6.0, rect.min.y + 3.0),
                Align2::LEFT_TOP,
                widgets::elide_path(&comment.file_path, 32),
                egui::FontId::proportional(SMALL_SIZE - 1.0),
                palette.muted,
            );
            ui.painter().text(
                egui::pos2(rect.max.x - 6.0, rect.min.y + 3.0),
                Align2::RIGHT_TOP,
                status,
                egui::FontId::proportional(SMALL_SIZE - 2.0),
                palette.accent,
            );

            let galley = ui.painter().layout(
                comment.comment.clone(),
                egui::FontId::proportional(crate::native::theme::UI_SIZE),
                palette.ink,
                rect.width() - 12.0,
            );
            ui.painter().galley(
                egui::pos2(rect.min.x + 6.0, rect.min.y + 17.0),
                galley,
                palette.ink,
            );
        }

        let response = response.on_hover_text(format!("{}\n\n{}", comment.selection, comment.comment));
        if response.clicked() && comment.jumpable {
            let review = app.model.review(session_id);
            review.scroll_to_hunk = Some(comment.hunk_id.clone());
            review.active_hunk_id = Some(comment.hunk_id.clone());
        }

        if !comment.resolved {
            response.context_menu(|ui| {
                if ui.button("mark resolved").clicked() {
                    let hunk_id = comment.hunk_id.clone();
                    let index = comment.comment_index;
                    let for_call = session_id.to_string();
                    app.tasks
                        .act(session_id, "could not resolve the comment", move |backend| {
                            backend.resolve_comment(&for_call, &hunk_id, index)
                        });
                    ui.close();
                }
            });
        }
        ui.add_space(4.0);
    }
}

/// What to call a comment's state, preferring what its agent run is doing over open/resolved.
pub(crate) fn status_label(comment: &crate::api::ReviewCommentView) -> &'static str {
    use crate::api::CommentDispatchStatus as Status;
    match comment.dispatch.status {
        Status::Completed => "complete",
        Status::Failed => "failed",
        Status::Running => "running",
        Status::Queued => "queued",
        Status::Batched => "batched",
        Status::Canceled => "canceled",
        Status::Idle => {
            if comment.resolved {
                "resolved"
            } else {
                "open"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{AgentKind, CommentDispatchStatus, CommentDispatchView, ReviewCommentView};

    fn comment(resolved: bool, status: CommentDispatchStatus) -> ReviewCommentView {
        ReviewCommentView {
            hunk_id: "hunk".to_string(),
            comment_index: 0,
            file_path: "src/a.rs".to_string(),
            header: String::new(),
            selection: String::new(),
            comment: String::new(),
            resolved,
            dispatch: CommentDispatchView {
                key: String::new(),
                status,
                detail: String::new(),
                agent: AgentKind::None,
                can_cancel: false,
                has_log: false,
            },
            jumpable: true,
        }
    }

    #[test]
    fn a_running_dispatch_outranks_open_and_resolved() {
        assert_eq!(
            status_label(&comment(false, CommentDispatchStatus::Running)),
            "running"
        );
        assert_eq!(
            status_label(&comment(true, CommentDispatchStatus::Failed)),
            "failed"
        );
    }

    #[test]
    fn an_idle_comment_reports_whether_it_is_resolved() {
        assert_eq!(
            status_label(&comment(false, CommentDispatchStatus::Idle)),
            "open"
        );
        assert_eq!(
            status_label(&comment(true, CommentDispatchStatus::Idle)),
            "resolved"
        );
    }
}
