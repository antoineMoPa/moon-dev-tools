//! Task review - every repo a task still has a pending review in, as one diff.
//!
//! A task that touched eight submodules asks for eight reviews, and opening them one tab at a
//! time is eight clicks and eight places to keep track of. This pane lays them one under the
//! other in a single scroll, in the order the task's `request_for_review.txt` lists them, which
//! is the order they are to be committed and deployed. Each repo is still its own review: its
//! hunks are drawn by the review pane's own code against its own session, so staging, comments
//! and discards land in the repo they belong to.
//!
//! Each repo is committed where it is read: its diff ends in a commit row and a box with the
//! message and the terminal side by side - see [`crate::native::commit_pane::inline`]. While a
//! repo's diff is on screen its row is pinned to the bottom of the scroll, and it lands as the
//! head of the box when the end of the diff comes up. A committed repo folds to one line with
//! its push.
//!
//! The pane names the task rather than its sessions, because a session lasts only as long as
//! the server that made it. The sessions are found by repo in the model, and opened the first
//! time a repo is drawn.

use std::sync::Arc;

use egui::{Rect, RichText, Ui};

use crate::{
    api::{OpenSessionRequest, SessionPayload},
    moontasks::ReviewRequestView,
    native::{
        app::App,
        commit_pane::inline::{self, RowPress},
        model::ScrollTo,
        review::{
            hunks,
            minimap::{self, Minimap, Scrolled},
            sidebar,
        },
        submodules::changes_label,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How wide the pane has to be to stand the list of repos beside the diff.
const ROOM_FOR_THE_REPO_LIST: f32 = 760.0;

/// How tall the row pinned to the bottom of the scroll is drawn.
const PINNED_ROW_HEIGHT: f32 = 30.0;

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
        .filter(|request| {
            request.task_id == task_id
                && (request.pending() || committed_in_this_pane(app, request))
        })
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

    // One map down the whole scroll, as a single review's pane has down its own - see
    // [`crate::native::review::draw`] for why it is a panel, and why it is pressed before the
    // diff is drawn and painted after. Each repo's cards are reported to it as it is drawn.
    let minimap_beside = minimap::fits_beside(ui.available_width())
        && repos.iter().any(|repo| {
            repo.payload
                .as_ref()
                .is_some_and(|payload| !payload.hunks.is_empty())
        });
    let mut map = app
        .model
        .task_review_minimaps
        .remove(task_id)
        .unwrap_or_default();
    let style = palette.editor_style();
    let strip = minimap_beside.then(|| {
        egui::Panel::right(egui::Id::new(("task-review-minimap", task_id)))
            .resizable(false)
            .show_separator_line(false)
            .exact_size(style.minimap_width)
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ui, |ui| map.take_press(ui, strip_id(task_id)))
            .inner
    });

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.bg)
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui, |ui| {
            let mut scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt(("task-review-diff", task_id));
            if let Some(offset) = map.asked.take() {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }
            let scrolled = scroll_area.show(ui, |ui| {
                map.start(ui.cursor().min.y);
                let mut commit_places = Vec::new();
                for repo in &repos {
                    let top = ui.cursor().min.y;
                    if let Some(place) = draw_repo(app, ui, repo, &mut map, &palette) {
                        commit_places.push((top, place));
                    }
                    ui.add_space(18.0);
                }
                pin_commit_row(app, ui, &commit_places, &palette);
            });
            map.scrolled = Scrolled {
                offset: scrolled.state.offset.y,
                content_height: scrolled.content_size.y,
                view_height: scrolled.inner_rect.height(),
            };
        });

    if let Some(strip) = strip {
        let session_ids: Vec<&str> = repos
            .iter()
            .filter_map(|repo| repo.session_id.as_deref())
            .collect();
        map.paint(
            app,
            ui,
            strip_id(task_id),
            &session_ids,
            strip,
            &style,
            &palette,
        );
    }
    app.model
        .task_review_minimaps
        .insert(task_id.to_string(), map);
}

/// The minimap strip's widget id, derived from the task so the tests can find it.
pub(crate) fn strip_id(task_id: &str) -> egui::Id {
    egui::Id::new(("moonreview-task-minimap", task_id))
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
    // The lines of the repos whose diff has arrived: the size of the whole review at a glance.
    let hunks = repos
        .iter()
        .filter_map(|repo| repo.payload.as_ref())
        .flat_map(|payload| &payload.hunks);
    let (added, removed) = hunks.fold((0, 0), |(added, removed), hunk| {
        (added + hunk.added_line_count, removed + hunk.removed_line_count)
    });
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
        widgets::line_counts(ui, added, removed, SMALL_SIZE, palette);
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

/// Whether a repo whose line is no longer pending was committed from here, which keeps it in
/// the pane as its committed line until the push.
fn committed_in_this_pane(app: &App, request: &ReviewRequestView) -> bool {
    !request.done
        && !request.task_finished
        && app
            .model
            .review_sessions_of_repos
            .get(&request.repo_path)
            .is_some_and(|session_id| app.inline_commit_made(session_id))
}

/// Where a repo's commit row and box were drawn in their place, which is what decides whether
/// the row is pinned to the bottom of the scroll.
struct CommitPlace {
    session_id: String,
    repo_name: String,
    row: Rect,
    commit_box: Rect,
}

/// One repo: a bar naming it, its diff under the bar, and its commit under the diff. A repo
/// committed from here is its committed line instead.
fn draw_repo(
    app: &mut App,
    ui: &mut Ui,
    repo: &RepoUnderReview,
    map: &mut Minimap,
    palette: &Palette,
) -> Option<CommitPlace> {
    if !repo.request.pending() {
        let session_id = repo.session_id.as_deref()?;
        app.keep_inline_commit(session_id);
        inline::draw_committed_line(app, ui, session_id, &repo.request.name, palette);
        return None;
    }

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
                        && widgets::clickable(ui.button("commit pane"))
                            .on_hover_text(
                                "Open the commit pane of this repo: push, pull request, and a \
                                 message written by the agent",
                            )
                            .clicked()
                    {
                        app.open_commit_pane(session_id);
                    }
                });
            });
        });
    ui.add_space(6.0);

    match (&repo.session_id, &repo.payload, &repo.error) {
        (Some(session_id), Some(_), _) => {
            hunks::draw_in_place(app, ui, session_id, map, palette);
            Some(draw_commit(app, ui, session_id, &repo.request.name, palette))
        }
        (_, _, Some(error)) => {
            ui.label(RichText::new(error).color(palette.warn));
            None
        }
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("reading the diff…").color(palette.muted));
            });
            None
        }
    }
}

/// The commit under a repo's diff: the row, and the box under it.
fn draw_commit(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    repo_name: &str,
    palette: &Palette,
) -> CommitPlace {
    app.keep_inline_commit(session_id);
    ui.add_space(10.0);
    let row = commit_row_frame(palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.push_id(("commit-row", session_id), |ui| {
            inline::draw_row(app, ui, session_id, repo_name, false, palette)
        })
        .inner
    });
    let commit_box = egui::Frame::new()
        .fill(palette.panel)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            inline::draw_box(app, ui, session_id, palette);
        })
        .response
        .rect;
    if row.inner != RowPress::Nothing {
        ui.scroll_to_rect(commit_box, Some(egui::Align::Max));
    }
    CommitPlace {
        session_id: session_id.to_string(),
        repo_name: repo_name.to_string(),
        row: row.response.rect,
        commit_box,
    }
}

/// Pin the commit row of the repo being read to the bottom of the scroll: the repo whose top is
/// above the bottom of the view and whose row, in its place, is below it. Drawn after the diff
/// so it is over it, and takes the clicks there.
fn pin_commit_row(
    app: &mut App,
    ui: &mut Ui,
    commit_places: &[(f32, CommitPlace)],
    palette: &Palette,
) {
    let view = ui.clip_rect();
    let pinned_top = view.bottom() - PINNED_ROW_HEIGHT;
    let Some((_, place)) = commit_places
        .iter()
        .find(|(top, place)| *top < pinned_top && place.row.top() > pinned_top)
    else {
        return;
    };
    let rect = Rect::from_min_max(
        egui::pos2(place.row.left(), pinned_top),
        egui::pos2(place.row.right(), view.bottom()),
    );
    let press = ui
        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            commit_row_frame(palette)
                .stroke(egui::Stroke::new(1.0, palette.line))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.push_id(("pinned-commit-row", &place.session_id), |ui| {
                        inline::draw_row(
                            app,
                            ui,
                            &place.session_id,
                            &place.repo_name,
                            true,
                            palette,
                        )
                    })
                    .inner
                })
                .inner
        })
        .inner;
    if press != RowPress::Nothing {
        ui.scroll_to_rect(place.commit_box, Some(egui::Align::Max));
    }
}

fn commit_row_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.panel)
        .inner_margin(egui::Margin::symmetric(8, 5))
}
