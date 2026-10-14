//! Task review - every repo a task still has a pending review in, as one diff.
//!
//! A task that touched eight submodules asks for eight reviews, and opening them one tab at a
//! time is eight clicks and eight places to keep track of. This pane lays them one under the
//! other in a single scroll, in the order the task's `request_for_review.txt` lists them, which
//! is the order they are to be committed and deployed. Each repo is still its own review: its
//! hunks are drawn by the review pane's own code against its own session, so staging, comments
//! and discards land in the repo they belong to, and each repo has its own button to its commit.
//!
//! The pane names the task rather than its sessions, because a session lasts only as long as
//! the server that made it. The sessions are found by repo in the model, and opened the first
//! time a repo is drawn.

use std::sync::Arc;

use egui::{RichText, Ui};

use crate::{
    api::{OpenSessionRequest, SessionPayload},
    moontasks::ReviewRequestView,
    native::{
        app::App,
        model::ScrollTo,
        review::{hunks, sidebar},
        submodules::changes_label,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide the pane has to be to stand the list of repos beside the diff.
const ROOM_FOR_THE_REPO_LIST: f32 = 760.0;

/// A repo of the task as this pane draws it: the line that asked for it, and where its review
/// stands.
struct RepoUnderReview {
    request: ReviewRequestView,
    session_id: Option<String>,
    payload: Option<Arc<SessionPayload>>,
    error: Option<String>,
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, task_id: &str) {
    let palette = app.palette_of();
    let requests: Vec<ReviewRequestView> = app
        .model
        .review_requests
        .iter()
        .filter(|request| request.task_id == task_id && request.pending())
        .cloned()
        .collect();
    open_sessions(app, &requests);
    let repos: Vec<RepoUnderReview> = requests
        .into_iter()
        .map(|request| repo_under_review(app, request))
        .collect();

    egui::Panel::top(egui::Id::new(("task-review-header", task_id)))
        .frame(
            egui::Frame::new()
                .fill(palette.header_bg)
                .inner_margin(egui::Margin::symmetric(8, 5)),
        )
        .show(ui, |ui| draw_header(ui, &repos, &palette));

    if repos.is_empty() {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    ui.label(RichText::new("nothing left to review").color(palette.ink));
                });
            });
        return;
    }

    if ui.available_width() >= ROOM_FOR_THE_REPO_LIST {
        egui::Panel::left(egui::Id::new(("task-review-repos", task_id)))
            .resizable(true)
            .default_size(270.0)
            .size_range(180.0..=520.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(ui, |ui| draw_repo_list(app, ui, &repos, &palette));
    }

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.bg)
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt(("task-review-diff", task_id))
                .show(ui, |ui| {
                    for repo in &repos {
                        draw_repo(app, ui, repo, &palette);
                        ui.add_space(18.0);
                    }
                });
        });
}

/// Ask the server for the session of each repo that has none yet. Asking for a session of a
/// repo that already has one answers with that one, so a repo whose own review is open shares
/// it with this pane.
fn open_sessions(app: &mut App, requests: &[ReviewRequestView]) {
    for request in requests {
        let repo_path = request.repo_path.clone();
        if app.model.review_sessions_of_repos.contains_key(&repo_path)
            || !app.model.opening_review_sessions.insert(repo_path.clone())
        {
            continue;
        }
        app.tasks.spawn(
            {
                let repo_path = repo_path.clone();
                move |backend| {
                    backend.open_session(OpenSessionRequest {
                        repo_path: repo_path.clone(),
                        diff_target: None,
                        active_commit: None,
                    })
                }
            },
            move |model, result| {
                model.opening_review_sessions.remove(&repo_path);
                match result {
                    Ok(opened) => {
                        model.review(&opened.session_id).refresh_requested = true;
                        model
                            .review_sessions_of_repos
                            .insert(repo_path, opened.session_id);
                    }
                    Err(error) => model.error(format!(
                        "could not open the review of {repo_path}: {error}"
                    )),
                }
            },
        );
    }
}

fn repo_under_review(app: &App, request: ReviewRequestView) -> RepoUnderReview {
    let session_id = app
        .model
        .review_sessions_of_repos
        .get(&request.repo_path)
        .cloned();
    let review = session_id
        .as_deref()
        .and_then(|session_id| app.model.review_ref(session_id));
    RepoUnderReview {
        payload: review.and_then(|review| review.payload.clone()),
        error: review.and_then(|review| review.error.clone()),
        request,
        session_id,
    }
}

fn draw_header(ui: &mut Ui, repos: &[RepoUnderReview], palette: &Palette) {
    let changed_files: usize = repos.iter().map(|repo| repo.request.changed_files).sum();
    ui.horizontal(|ui| {
        ui.label(RichText::new("all pending reviews").strong().color(palette.ink));
        ui.label(
            RichText::new(format!(
                "{} repos · {}",
                repos.len(),
                changes_label(changed_files)
            ))
            .size(SMALL_SIZE)
            .color(palette.muted),
        );
    });
}

/// The repos down the side, each with the files section of its own review under it - staging
/// dots, counts and comments as the review's sidebar has them. A click on a repo's name brings
/// its first hunk into view in the diff, and a click on a file brings the file's.
fn draw_repo_list(app: &mut App, ui: &mut Ui, repos: &[RepoUnderReview], palette: &Palette) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for repo in repos {
                let first_hunk = repo
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.hunks.first());
                let name = ui.add(
                    egui::Label::new(RichText::new(&repo.request.name).strong().color(palette.ink))
                        .sense(egui::Sense::click()),
                );
                if widgets::clickable(name).clicked()
                    && let (Some(session_id), Some(hunk)) = (&repo.session_id, first_hunk)
                {
                    app.model.review(session_id).scroll_to = Some(ScrollTo::hunk(hunk.id.clone()));
                }
                if let (Some(session_id), Some(_)) = (&repo.session_id, &repo.payload) {
                    ui.push_id(session_id, |ui| {
                        sidebar::draw_files_of(app, ui, session_id, palette);
                    });
                }
                ui.add_space(12.0);
            }
        });
}

/// One repo: a bar naming it, and its diff under the bar.
fn draw_repo(app: &mut App, ui: &mut Ui, repo: &RepoUnderReview, palette: &Palette) {
    egui::Frame::new()
        .fill(palette.header_bg)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&repo.request.name)
                        .strong()
                        .color(palette.ink),
                );
                if let Some(branch) = &repo.request.branch {
                    ui.label(
                        RichText::new(format!("#{}", widgets::elide_end(branch, 48)))
                            .size(SMALL_SIZE)
                            .color(palette.muted),
                    )
                    .on_hover_text(format!("#{branch}"));
                }
                ui.label(
                    RichText::new(changes_label(repo.request.changed_files))
                        .size(SMALL_SIZE)
                        .color(palette.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(session_id) = &repo.session_id
                        && widgets::clickable(ui.button("commit"))
                            .on_hover_text("Open the commit pane of this repo")
                            .clicked()
                    {
                        app.open_commit_pane(session_id);
                    }
                });
            });
        });
    ui.add_space(6.0);

    match (&repo.session_id, &repo.payload, &repo.error) {
        (Some(session_id), Some(_), _) => hunks::draw_in_place(app, ui, session_id, palette),
        (_, _, Some(error)) => {
            ui.label(RichText::new(error).color(palette.warn));
        }
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("reading the diff…").color(palette.muted));
            });
        }
    }
}
