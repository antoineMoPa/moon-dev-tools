//! Drawing the commit pane: the branch line, the staged files, the message box and the
//! suggestion offered for it.

use egui::{RichText, Ui};
use egui_frames::PaneId;

use crate::{
    api::FileChangeKind,
    commit_suggestion::CommitSuggestion,
    committing::{CommitAction, CommitState},
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

use super::{MESSAGE_ROWS, RUN_TERMINAL_HEIGHT, RUN_TERMINAL_RANGE, Reached, words_for};

pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId, session_id: &str) {
    let palette = app.palette_of();
    app.commit_pane(session_id);
    app.refresh_commit_state(session_id);
    app.poll_commit_run(session_id);
    app.fill_in_the_requested_commit(session_id);
    app.auto_ask_for_commit_message(session_id);

    // While git is going, the keyboard belongs to the pty: that is where pinentry asks for
    // the passphrase. At rest it belongs to the message.
    let takes_keyboard = app.pane_taking_keyboard == Some(pane_id);
    if takes_keyboard {
        app.pane_taking_keyboard = None;
    }

    // A run that has been asked for but has not answered yet is as good as going: the pty is
    // on its way, and pressing again would start a second git.
    let starting = app.tasks.is_busy(&format!("commit-run:{session_id}"));
    let pane = &app.model.commit_panes[session_id];
    let running = starting || pane.is_running();
    let run_terminal = pane.run.as_ref().map(|run| run.terminal_id.clone());
    let run_note = pane.run.as_ref().map(|run| {
        let words = words_for(run.kind);
        match run.exit_code {
            None => (words.running, palette.muted),
            Some(0) => (words.worked, palette.added),
            Some(_) => (words.failed, palette.warn),
        }
    });
    let reached = pane.reached;
    let error = pane.error.clone();
    let state = pane.state.clone();
    let suggestion = pane.suggestion.clone();
    let suggestion_error = pane.suggestion_error.clone();
    let mut message = pane.message.clone();
    // Set when `[use]` was pressed, and answered once the pane is drawn.
    let mut used_suggestion = false;
    let writing_message = app.tasks.is_busy(&App::suggestion_key(session_id));
    // The repo this is committing, read off the review it belongs to: the pane may be one of
    // several open on several repos - a changed submodule has its own review and its own
    // commit pane - and the branch alone does not say which of them this one is.
    let repo = app
        .model
        .review_ref(session_id)
        .and_then(|review| review.payload.clone());
    // The branch a task's `request_for_review.txt` asked this commit to be made on, when one
    // did. Said beside the branch the repo is actually on, and no more than said - moving
    // someone's HEAD under a commit they are about to make is not the pane's to do.
    let asked_branch = app
        .requested_review_of(session_id)
        .and_then(|request| request.branch.clone());

    egui::Panel::top(egui::Id::new(("commit-header", session_id)))
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(2, 4)))
        .show(ui, |ui| {
            draw_branch_line(
                ui,
                repo.as_ref().map(|payload| Repo {
                    name: &payload.repo_name,
                    path: &payload.repo_path,
                }),
                state.as_ref(),
                asked_branch.as_deref(),
                &palette,
            );
        });

    if let Some(terminal_id) = &run_terminal {
        egui::Panel::bottom(egui::Id::new(("commit-run", session_id)))
            .resizable(true)
            .default_size(RUN_TERMINAL_HEIGHT)
            .size_range(RUN_TERMINAL_RANGE)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(2, 4)))
            .show(ui, |ui| {
                app.draw_commit_terminal(ui, terminal_id, takes_keyboard && running);
            });
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(2, 4)))
        .show(ui, |ui| {
            if let Some(error) = &error {
                ui.label(RichText::new(error).color(palette.warn).size(SMALL_SIZE));
                ui.add_space(4.0);
            }

            // The message and the buttons sit at the top of the pane, where the eye lands and
            // where they stay put as the staged listing under them grows and shrinks. Off while
            // git is going: the message has been handed to it already, and what is typed then -
            // a passphrase meant for pinentry, while hooks run - is not a change to it.
            let output = ui
                .add_enabled_ui(!running, |ui| {
                    egui::TextEdit::multiline(&mut message)
                        .hint_text("what this commit does")
                        .desired_width(f32::INFINITY)
                        .desired_rows(MESSAGE_ROWS)
                        .show(ui)
                })
                .inner;
            if takes_keyboard && !running {
                output.response.request_focus();
            }

            // Under the box, where a message written by the agent reads as an offer of what to
            // put in it rather than as something already in it. `[use]` is off with the box,
            // since it writes into it.
            ui.add_space(4.0);
            if draw_suggested_message(
                ui,
                &palette,
                suggestion.as_ref(),
                suggestion_error.as_deref(),
                writing_message,
                !running,
            ) && let Some(suggestion) = &suggestion
            {
                message = suggestion.as_message();
                used_suggestion = true;
            }
            ui.add_space(6.0);

            let can_commit = !running
                && !message.trim().is_empty()
                && state
                    .as_ref()
                    .is_some_and(|state| !state.staged_files.is_empty());

            ui.horizontal(|ui| {
                // Staging a hunk at a time is the review's job next door; this is the sweep
                // for when the whole working tree is what the commit is.
                let unstaged = state.as_ref().map_or(0, |state| state.unstaged_count);
                let stage_all = widgets::clickable(ui.add_enabled(
                    !running && unstaged > 0,
                    egui::Button::new(format!("stage all ({unstaged})")),
                ));
                if stage_all.clicked() {
                    app.stage_all(session_id);
                }
                if unstaged == 0 {
                    stage_all.on_disabled_hover_text("everything is staged already");
                }

                let commit =
                    widgets::clickable(ui.add_enabled(can_commit, egui::Button::new("commit")));
                if commit.clicked() {
                    app.start_commit_run(
                        session_id,
                        CommitAction::Commit {
                            message: message.clone(),
                        },
                    );
                }
                if !can_commit && !running {
                    commit.on_disabled_hover_text("a commit takes a staged change and a message");
                }

                // Each button waits for the thing it acts on to exist: something committed
                // to push, something pushed to open a pull request on. A branch that arrived
                // with commits its upstream has not got is already past the first of those.
                let ahead = state.as_ref().map_or(0, |state| state.ahead);
                if ahead > 0 || reached != Reached::Nothing {
                    // A push that worked took everything there was; `ahead` alone cannot say
                    // so, because the reading it came from may predate the push. The next
                    // commit - or a reading that finds commits from elsewhere - turns it back on.
                    let pushed_it_all = reached == Reached::Pushed;
                    let push = widgets::clickable(
                        ui.add_enabled(!running && !pushed_it_all, egui::Button::new("push")),
                    );
                    if push.clicked() {
                        app.start_commit_run(session_id, CommitAction::Push);
                    }
                    if pushed_it_all && !running {
                        push.on_disabled_hover_text("everything is pushed already");
                    }
                }

                // And only where `gh` is installed: without it there is no pull request to
                // open, and a button that could never work is worse than no button.
                let gh_installed = state.as_ref().is_some_and(|state| state.gh_installed);
                if gh_installed
                    && reached == Reached::Pushed
                    && widgets::clickable(ui.add_enabled(!running, egui::Button::new("open PR")))
                        .on_hover_text("gh pr create -w - fills the form in the browser")
                        .clicked()
                {
                    app.start_commit_run(session_id, CommitAction::OpenPr);
                }

                // Once the branch is sent the pane has done what it is for, and the review it
                // was committing has already closed itself. Offering the way out here saves a
                // trip to the tab strip.
                if reached == Reached::Pushed
                    && widgets::clickable(ui.button("close"))
                        .on_hover_text("close this commit pane")
                        .clicked()
                {
                    app.pending_close = Some(pane_id);
                }

                if let Some((note, color)) = run_note {
                    ui.label(RichText::new(note).color(color).size(SMALL_SIZE));
                }
            });

            ui.add_space(8.0);
            widgets::divider(ui, &palette);
            ui.add_space(6.0);
            draw_staged_files(ui, session_id, state.as_ref(), &palette);
        });

    let pane = app.commit_pane(session_id);
    if pane.message != message {
        pane.message = message;
    }
    if used_suggestion {
        // It is in the box now, and the box is where it is edited from here.
        pane.suggestion = None;
    }
}

/// The message the agent wrote, under the box it would go in: a line while it is being
/// written, the message itself with `[use]` beside it once it is, and why it did not come when
/// it did not. `[use]` is off unless `usable`. Answers whether `[use]` was pressed, which is
/// read after the pane is drawn - the row is inside a closure that has the pane borrowed.
pub(in crate::native) fn draw_suggested_message(
    ui: &mut Ui,
    palette: &Palette,
    suggestion: Option<&CommitSuggestion>,
    error: Option<&str>,
    writing: bool,
    usable: bool,
) -> bool {
    if writing {
        ui.horizontal(|ui| {
            widgets::small_spinner(ui, palette.muted);
            ui.label(
                RichText::new("writing a commit message…")
                    .color(palette.muted)
                    .size(SMALL_SIZE),
            );
        });
        return false;
    }

    if let Some(suggestion) = suggestion {
        let mut used = false;
        ui.horizontal(|ui| {
            used = widgets::clickable(ui.add_enabled(usable, egui::Button::new("use")))
                .on_hover_text("put this message in the box")
                .clicked();
            ui.add(
                egui::Label::new(RichText::new(&suggestion.subject).color(palette.ink)).truncate(),
            )
            .on_hover_text(&suggestion.subject);
        });
        if !suggestion.paragraph.trim().is_empty() {
            ui.label(
                RichText::new(&suggestion.paragraph)
                    .color(palette.muted)
                    .size(SMALL_SIZE),
            );
        }
        return used;
    }

    // A message that would not come is said once and left at that: writing the commit is the
    // pane's job with or without one, and there is nothing here to press.
    if let Some(error) = error {
        ui.add(
            egui::Label::new(RichText::new(error).color(palette.warn).size(SMALL_SIZE)).truncate(),
        )
        .on_hover_text(error);
    }
    false
}

/// The repo a commit pane is committing, as its header names it. Read off the review the pane
/// belongs to, so it is `None` only until that review's first answer arrives.
struct Repo<'a> {
    name: &'a str,
    /// Where it is on the machine the backend reads, which the name is read in full on.
    path: &'a str,
}

/// The header of the commit pane: which repo, on which branch, and how far that branch is from
/// its upstream.
///
/// The repo comes first and the branch after it, the way the review's own header reads - the
/// window can have a commit pane open on a repo and on each of its changed submodules, and
/// `main` on the tab strip says nothing about which of them is about to be committed.
fn draw_branch_line(
    ui: &mut Ui,
    repo: Option<Repo<'_>>,
    state: Option<&CommitState>,
    asked_branch: Option<&str>,
    palette: &Palette,
) {
    let Some(state) = state else {
        ui.label(RichText::new("reading the repo…").color(palette.muted));
        return;
    };

    ui.horizontal(|ui| {
        if let Some(repo) = repo {
            ui.label(RichText::new(repo.name).strong())
                .on_hover_text(repo.path);
            ui.label(RichText::new("·").color(palette.line));
        }
        let branch = state.branch_name.as_deref().unwrap_or("detached HEAD");
        ui.label(RichText::new(branch).strong());
        match &state.upstream_ref {
            Some(upstream) => {
                ui.label(RichText::new("→").color(palette.line));
                let label = ui.label(RichText::new(upstream).color(palette.accent));
                // An upstream git would not push to as it stands: the branch tracks one
                // named differently, the state starting a branch from `origin/dev` leaves it
                // in. The push goes under the branch's own name, and the label says so.
                if state.push_ref.is_none() {
                    label.on_hover_text(
                        "pushing sends it to origin under its own name and tracks it there",
                    );
                }
            }
            None => {
                ui.label(
                    RichText::new("no upstream yet")
                        .color(palette.muted)
                        .size(SMALL_SIZE),
                )
                .on_hover_text("pushing sets one on origin");
            }
        }
        if state.ahead > 0 {
            widgets::pill(
                ui,
                &format!("{} to push", state.ahead),
                palette.added,
                palette.status_neutral_bg,
            );
        }
        if state.behind > 0 {
            widgets::pill(
                ui,
                &format!("{} behind", state.behind),
                palette.warn,
                palette.status_neutral_bg,
            );
        }
        // Only when the two differ: a pane sitting on the branch that was asked for has nothing
        // to say about it, and a line saying so on every commit is a line nobody reads. When they
        // do differ this is also what says why the box is empty - the message written for that
        // branch is held back from a commit being made somewhere else.
        let elsewhere = asked_branch.filter(|asked| Some(*asked) != state.branch_name.as_deref());
        if let Some(asked) = elsewhere {
            widgets::pill(
                ui,
                &format!("asked for {asked}"),
                palette.warn,
                palette.status_neutral_bg,
            )
            .on_hover_text(format!(
                "the task asking for this review means the commit for {asked}, \
                 so the message it wrote is not put in the box here"
            ));
        }
    });
}

fn draw_staged_files(
    ui: &mut Ui,
    session_id: &str,
    state: Option<&CommitState>,
    palette: &Palette,
) {
    let Some(state) = state else {
        return;
    };
    if state.staged_files.is_empty() {
        ui.label(
            RichText::new("nothing staged - stage what to commit in the review")
                .color(palette.muted)
                .size(SMALL_SIZE),
        );
        return;
    }

    widgets::section_header(
        ui,
        &format!("staged ({})", state.staged_files.len()),
        palette,
        |_| {},
    );
    egui::ScrollArea::vertical()
        .id_salt(("commit-staged", session_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (directory, files) in
                widgets::by_directory(state.staged_files.iter(), |file| file.file_path.as_str())
            {
                // The directory once, over the names in it: a commit is usually a handful of
                // files in two or three places, and repeating the path on every row buries the
                // names under it.
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&directory)
                            .color(palette.muted)
                            .size(SMALL_SIZE - 1.0),
                    )
                    .truncate(),
                )
                .on_hover_text(&directory);

                for file in files {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(change_mark(file.change_kind))
                                .color(change_ink(file.change_kind, palette))
                                .monospace()
                                .size(SMALL_SIZE),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(widgets::file_name_of(&file.file_path))
                                    .color(palette.ink)
                                    .size(SMALL_SIZE),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&file.file_path);
                    });
                }
            }
        });
}

fn change_mark(change_kind: FileChangeKind) -> &'static str {
    match change_kind {
        FileChangeKind::Added => "+",
        FileChangeKind::Deleted => "−",
        FileChangeKind::Modified => "~",
    }
}

fn change_ink(change_kind: FileChangeKind, palette: &Palette) -> egui::Color32 {
    match change_kind {
        FileChangeKind::Added => palette.added,
        FileChangeKind::Deleted => palette.removed,
        FileChangeKind::Modified => palette.muted,
    }
}
