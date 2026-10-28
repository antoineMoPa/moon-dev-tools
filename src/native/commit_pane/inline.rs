//! Inline commit - a repo's commit drawn inside the task review, under that repo's diff.
//!
//! Three pieces, all on the same [`CommitPane`] state the commit pane keeps per review, so a
//! message written here is the one the commit pane shows and the other way round:
//! - the commit row: the repo, the subject about to be committed, what is staged, `commit`.
//!   The task review pins it to the bottom of its scroll while the repo's diff is on screen.
//! - the commit box under the row: the message on the left, the run's terminal on the right,
//!   side by side, so a pinentry asking for the passphrase is in sight of the message.
//! - the committed line a repo folds into once its commit is made, with the push after it.

use egui::{RichText, Ui};

use crate::{
    committing::{CommitAction, CommitState},
    native::{
        app::App,
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

use super::{CommitPane, RUN_TERMINAL_HEIGHT, Reached, RunKind, words_for};

/// How tall the box under the row is: the height a terminal pinentry needs, which the message
/// beside it takes too so the box does not change shape when a run starts.
pub(crate) const COMMIT_BOX_HEIGHT: f32 = RUN_TERMINAL_HEIGHT;
/// How tall the push's terminal is under a committed line. A push asks for an ssh passphrase
/// at most, which is a line or two.
const PUSH_TERMINAL_HEIGHT: f32 = 140.0;

/// What a press on the commit row asks of the task review, which owns the scroll.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowPress {
    Nothing,
    /// Bring the box into view: the row is pinned and its message is further down.
    ShowBox,
    /// A commit was started. Its terminal is in the box, which has to be in view to answer it.
    Committed,
}

impl App {
    /// What an inline commit needs done before it is drawn, every frame it is: read the repo,
    /// follow the run, and put the message the task wrote for this repo in the box.
    pub(crate) fn keep_inline_commit(&mut self, session_id: &str) {
        self.commit_pane(session_id);
        self.refresh_commit_state(session_id);
        self.poll_commit_run(session_id);
        self.fill_in_the_requested_commit(session_id);
        self.forget_suggestion_of_nothing_staged(session_id);
    }

    /// Whether this review's commit has been made, which is what folds its repo to one line.
    pub(crate) fn inline_commit_made(&self, session_id: &str) -> bool {
        self.model
            .commit_panes
            .get(session_id)
            .is_some_and(|pane| pane.reached != Reached::Nothing)
    }

    /// Whether a run has been asked for and has not ended. The buttons are off while it is.
    fn inline_commit_running(&self, session_id: &str) -> bool {
        self.tasks.is_busy(&format!("commit-run:{session_id}"))
            || self.model.commit_panes[session_id].is_running()
    }
}

/// The row over a repo's commit box. `pinned` when the task review draws it at the bottom of
/// its scroll rather than in its place, which trades `stage all` for a way down to the box.
pub(crate) fn draw_row(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    repo_name: &str,
    pinned: bool,
    palette: &Palette,
) -> RowPress {
    let running = app.inline_commit_running(session_id);
    let pane = &app.model.commit_panes[session_id];
    let subject = pane.message.lines().next().unwrap_or("").trim().to_string();
    let message = pane.message.clone();
    let state = pane.state.clone();
    let run_note = run_note(pane, palette);

    let mut press = RowPress::Nothing;
    ui.horizontal(|ui| {
        ui.label(RichText::new(repo_name).strong().color(palette.ink));
        let (subject_text, subject_ink) = if subject.is_empty() {
            ("no message yet", palette.muted)
        } else {
            (subject.as_str(), palette.ink)
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_commit = can_commit(running, &message, state.as_ref());
            let commit =
                widgets::clickable(ui.add_enabled(can_commit, egui::Button::new("commit")));
            if commit.clicked() {
                app.start_commit_run(
                    session_id,
                    CommitAction::Commit {
                        message: message.clone(),
                    },
                );
                press = RowPress::Committed;
            }
            if !can_commit && !running {
                commit.on_disabled_hover_text("a commit takes a staged change and a message");
            }

            if pinned {
                if widgets::clickable(ui.button("↓ message"))
                    .on_hover_text("scroll to this repo's message and terminal")
                    .clicked()
                {
                    press = RowPress::ShowBox;
                }
            } else {
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
            }

            match (&run_note, &state) {
                (Some((note, ink)), _) => {
                    ui.label(RichText::new(*note).color(*ink).size(SMALL_SIZE));
                }
                (None, Some(state)) => {
                    ui.label(
                        RichText::new(format!("{} staged", state.staged_files.len()))
                            .color(palette.muted)
                            .size(SMALL_SIZE),
                    );
                }
                (None, None) => {
                    ui.label(
                        RichText::new("reading the repo…")
                            .color(palette.muted)
                            .size(SMALL_SIZE),
                    );
                }
            }

            ui.add(
                egui::Label::new(
                    RichText::new(subject_text)
                        .monospace()
                        .size(SMALL_SIZE)
                        .color(subject_ink),
                )
                .truncate(),
            );
        });
    });
    press
}

/// The box under the row: the message on the left and the terminal on the right.
pub(crate) fn draw_box(app: &mut App, ui: &mut Ui, session_id: &str, palette: &Palette) {
    let running = app.inline_commit_running(session_id);
    let pane = &app.model.commit_panes[session_id];
    let mut message = pane.message.clone();
    let error = pane.error.clone();
    let state = pane.state.clone();
    // The run's terminal takes the keyboard the first frame it is drawn: that is where pinentry
    // asks for the passphrase. After that it is the person's to click away from.
    let run = pane
        .run
        .as_ref()
        .map(|run| (run.terminal_id.clone(), !run.keyboard_handed_over));

    let half = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(120.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(half, COMMIT_BOX_HEIGHT), |ui| {
            ui.set_min_size(egui::vec2(half, COMMIT_BOX_HEIGHT));
            ui.vertical(|ui| {
                if let Some(error) = &error {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(palette.warn).size(SMALL_SIZE))
                            .truncate(),
                    )
                    .on_hover_text(error);
                }
                // Off while git is going, as in the commit pane: the message has been handed to
                // it, and what is typed then is meant for pinentry.
                let rows = ((ui.available_height() - 8.0)
                    / ui.text_style_height(&egui::TextStyle::Monospace))
                .floor()
                .max(3.0) as usize;
                ui.add_enabled_ui(!running, |ui| {
                    egui::TextEdit::multiline(&mut message)
                        .id_salt(("inline-commit-message", session_id))
                        .hint_text("what this commit does")
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(rows)
                        .show(ui)
                });
            });
        });

        ui.allocate_ui(egui::vec2(half, COMMIT_BOX_HEIGHT), |ui| {
            ui.set_min_size(egui::vec2(half, COMMIT_BOX_HEIGHT));
            match &run {
                Some((terminal_id, takes_keyboard)) => {
                    app.draw_commit_terminal(ui, terminal_id, *takes_keyboard);
                    if *takes_keyboard && app.commit_terminals.contains_key(terminal_id) {
                        hand_over_keyboard(app, session_id);
                    }
                }
                None => {
                    ui.vertical(|ui| draw_what_runs(ui, state.as_ref(), palette));
                }
            }
        });
    });

    let pane = app.commit_pane(session_id);
    if pane.message != message {
        pane.message = message;
    }
}

/// The line a repo folds into once its commit is made: what was done, and the push after it.
/// The push's terminal sits under the line for as long as its run is the last one.
pub(crate) fn draw_committed_line(
    app: &mut App,
    ui: &mut Ui,
    session_id: &str,
    repo_name: &str,
    palette: &Palette,
) {
    let running = app.inline_commit_running(session_id);
    let pane = &app.model.commit_panes[session_id];
    let reached = pane.reached;
    let run_note = run_note(pane, palette);
    let push_run = pane
        .run
        .as_ref()
        .filter(|run| run.kind == RunKind::Push)
        .map(|run| (run.terminal_id.clone(), !run.keyboard_handed_over));

    egui::Frame::new()
        .fill(palette.header_bg)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(repo_name).strong().color(palette.ink));
                let done = if reached == Reached::Pushed {
                    "✓ pushed"
                } else {
                    "✓ committed"
                };
                widgets::pill(ui, done, palette.added, palette.status_neutral_bg);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if reached == Reached::Committed
                        && widgets::clickable(ui.add_enabled(!running, egui::Button::new("push")))
                            .on_hover_text("git push this repo's branch")
                            .clicked()
                    {
                        app.start_commit_run(session_id, CommitAction::Push { message: None });
                    }
                    if let Some((note, ink)) = run_note {
                        ui.label(RichText::new(note).color(ink).size(SMALL_SIZE));
                    }
                });
            });
        });

    if let Some((terminal_id, takes_keyboard)) = push_run {
        ui.add_space(4.0);
        let width = ui.available_width();
        ui.allocate_ui(egui::vec2(width, PUSH_TERMINAL_HEIGHT), |ui| {
            ui.set_min_size(egui::vec2(width, PUSH_TERMINAL_HEIGHT));
            app.draw_commit_terminal(ui, &terminal_id, takes_keyboard);
        });
        if takes_keyboard && app.commit_terminals.contains_key(&terminal_id) {
            hand_over_keyboard(app, session_id);
        }
    }
}

/// What the box's right half says before any run: what a commit would take in and what it
/// would leave, so the box reads the same shape before and during a run.
fn draw_what_runs(ui: &mut Ui, state: Option<&CommitState>, palette: &Palette) {
    egui::Frame::new()
        .fill(palette.code_bg)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let Some(state) = state else {
                ui.label(RichText::new("reading the repo…").color(palette.muted));
                return;
            };
            let branch = state.branch_name.as_deref().unwrap_or("detached HEAD");
            ui.label(
                RichText::new(format!("git commit on {branch}"))
                    .monospace()
                    .size(SMALL_SIZE)
                    .color(palette.muted),
            );
            ui.add_space(4.0);
            if state.staged_files.is_empty() {
                ui.label(
                    RichText::new("nothing staged - stage what to commit in the diff")
                        .color(palette.muted)
                        .size(SMALL_SIZE),
                );
            }
            for file in &state.staged_files {
                ui.add(
                    egui::Label::new(
                        RichText::new(&file.file_path)
                            .monospace()
                            .size(SMALL_SIZE)
                            .color(palette.ink),
                    )
                    .truncate(),
                );
            }
            if state.unstaged_count > 0 {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} not staged", state.unstaged_count))
                        .color(palette.muted)
                        .size(SMALL_SIZE),
                );
            }
        });
}

/// A commit takes a staged change and a message, and nothing already going.
fn can_commit(running: bool, message: &str, state: Option<&CommitState>) -> bool {
    !running
        && !message.trim().is_empty()
        && state.is_some_and(|state| !state.staged_files.is_empty())
}

/// What the last run says about itself, once there is one.
fn run_note(pane: &CommitPane, palette: &Palette) -> Option<(&'static str, egui::Color32)> {
    pane.run.as_ref().map(|run| {
        let words = words_for(run.kind);
        match run.exit_code {
            None => (words.running, palette.muted),
            Some(0) => (words.worked, palette.added),
            Some(_) => (words.failed, palette.warn),
        }
    })
}

fn hand_over_keyboard(app: &mut App, session_id: &str) {
    if let Some(run) = &mut app.commit_pane(session_id).run {
        run.keyboard_handed_over = true;
    }
}
