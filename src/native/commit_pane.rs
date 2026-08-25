//! Committing what the review has staged, and pushing it, without leaving the window.
//!
//! Both actions run as `git` in a pty rather than as a captured process, and the pane shows
//! that pty. That is what makes a signed commit work: gpg asks for the passphrase through
//! pinentry, a terminal pinentry needs a terminal to ask on, and this is it. Anything else
//! git wants typed — a hook's question, a push over ssh — lands in the same place.

use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use egui_frames::PaneId;

use crate::{
    api::FileChangeKind,
    committing::{CommitAction, CommitState},
    native::{
        app::{App, AttachedTerminal, TerminalHolder},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How tall the run's terminal starts out. A terminal pinentry draws a box some twenty rows
/// tall, and the passphrase has to be readable without dragging anything — so the run takes
/// real room while it is there, and is dragged smaller by anyone who wants it smaller.
const RUN_TERMINAL_HEIGHT: f32 = 320.0;
/// What dragging that divider can leave it at.
const RUN_TERMINAL_RANGE: std::ops::RangeInclusive<f32> = 90.0..=640.0;
/// How many lines of message the box shows before it scrolls: a subject, a blank line, and a
/// couple of lines of body.
const MESSAGE_ROWS: usize = 5;
/// How many times the pane asks how a run ended before giving up on being told.
///
/// The answer is recorded before the pty closes, so the first ask finds it. This is only a
/// stop for the case where it cannot be answered at all — a server that went away mid-run —
/// so that the pane says the run is over rather than waiting on it forever.
const OUTCOME_ASKS: u8 = 30;
/// How often an open commit pane rereads what is staged. Staging happens in the review pane
/// beside it, which has no way to tell this one, so it looks again on the review's own poll
/// cadence rather than being told.
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// What the pane says about a run, decided where the run starts rather than read off the
/// action later.
#[derive(Clone, Copy)]
struct RunWords {
    running: &'static str,
    worked: &'static str,
    failed: &'static str,
}

const COMMIT_WORDS: RunWords = RunWords {
    running: "committing…",
    worked: "committed",
    failed: "git would not commit — see below",
};

const PUSH_WORDS: RunWords = RunWords {
    running: "pushing…",
    worked: "pushed",
    failed: "git would not push — see below",
};

/// One review's commit pane. Kept per review rather than per pane, so closing the tab does not
/// throw away a message that was half written.
pub(crate) struct CommitPane {
    pub(crate) message: String,
    /// What git says the repo looks like, once it has been read.
    state: Option<CommitState>,
    /// Set when that reading is known to be out of date — after a run ends — which reads it
    /// again at once rather than on the next poll.
    stale: bool,
    /// When it was last read, so an open pane keeps up with staging done next door without
    /// running git on every frame it draws.
    last_read: Option<Instant>,
    /// The last run, kept after it ends so its output stays on screen until the next one.
    run: Option<CommitRun>,
    error: Option<String>,
}

impl CommitPane {
    fn new() -> Self {
        Self {
            message: String::new(),
            state: None,
            stale: true,
            last_read: None,
            run: None,
            error: None,
        }
    }

    /// How many files a commit would take in, once git has been asked. `None` until then —
    /// which is also when the commit button is still off.
    #[cfg(test)]
    pub(crate) fn staged_count_for_test(&self) -> Option<usize> {
        self.state.as_ref().map(|state| state.staged_files.len())
    }

    /// Whether git is going right now, which is when the buttons are off.
    fn is_running(&self) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.exit_code.is_none() && run.asks < OUTCOME_ASKS)
    }
}

struct CommitRun {
    terminal_id: String,
    words: RunWords,
    /// Whether a run that worked leaves the message behind. A commit's message has been used
    /// up; a push never had one.
    clears_message: bool,
    /// `None` while git is going, the exit code once it is over.
    exit_code: Option<i32>,
    /// How many times the ending has been asked for — see [`OUTCOME_ASKS`].
    asks: u8,
}

impl CommitRun {
    fn worked(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl App {
    /// Open the commit pane of a review, in a column down the right so the review it is
    /// committing stays on screen. Deferred like every other pane opened from inside a pane.
    pub(crate) fn open_commit_pane(&mut self, session_id: &str) {
        if self.pending_action.is_some() {
            return;
        }
        self.pending_action = Some(crate::native::palette::CommandAction::OpenPane(
            crate::native::panes::OpenPaneRequest::Commit {
                session_id: session_id.to_string(),
            },
        ));
    }

    fn commit_pane(&mut self, session_id: &str) -> &mut CommitPane {
        self.model
            .commit_panes
            .entry(session_id.to_string())
            .or_insert_with(CommitPane::new)
    }

    /// Read what git would commit and where a push would go, when what the pane is showing is
    /// out of date.
    fn refresh_commit_state(&mut self, session_id: &str) {
        let pane = self.commit_pane(session_id);
        let due = pane
            .last_read
            .is_none_or(|last| last.elapsed() >= STATE_POLL_INTERVAL);
        if !pane.stale && !due {
            return;
        }
        pane.stale = false;
        pane.last_read = Some(Instant::now());

        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(format!("commit-state:{session_id}")),
            move |backend| backend.commit_state(&for_call),
            move |model, result| {
                let Some(pane) = model.commit_panes.get_mut(&for_apply) else {
                    return;
                };
                match result {
                    Ok(state) => {
                        pane.state = Some(state);
                        pane.error = None;
                    }
                    Err(error) => pane.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Stage the whole working tree, and read back what that left staged.
    fn stage_all(&mut self, session_id: &str) {
        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(format!("stage-all:{session_id}")),
            move |backend| backend.stage_all(&for_call),
            move |model, result| {
                if let Some(pane) = model.commit_panes.get_mut(&for_apply) {
                    match result {
                        Ok(()) => {
                            pane.error = None;
                            pane.stale = true;
                        }
                        Err(error) => pane.error = Some(format!("{error}")),
                    }
                }
                model.review(&for_apply).refresh_requested = true;
            },
        );
    }

    /// Start `git` on one action, and attach the pane to the pty it runs in.
    fn start_commit_run(&mut self, session_id: &str, action: CommitAction, words: RunWords) {
        let clears_message = matches!(action, CommitAction::Commit { .. });
        // The run before this one has had its say; its pty goes with its pane's next run.
        if let Some(previous) = self.commit_pane(session_id).run.take() {
            self.commit_terminals.remove(&previous.terminal_id);
        }
        self.commit_pane(session_id).error = None;

        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        let inbox = std::sync::Arc::clone(&self.attaching);

        self.tasks.spawn_keyed(
            Some(format!("commit-run:{session_id}")),
            move |backend| {
                let terminal_id = backend.start_commit_run(&for_call, &action)?;
                let attachment = backend.attach_terminal(&for_call, &terminal_id);
                Ok((terminal_id, attachment))
            },
            move |model, result| {
                let Some(pane) = model.commit_panes.get_mut(&for_apply) else {
                    return;
                };
                match result {
                    Ok((terminal_id, attachment)) => {
                        pane.run = Some(CommitRun {
                            terminal_id: terminal_id.clone(),
                            words,
                            clears_message,
                            exit_code: None,
                            asks: 0,
                        });
                        if let Ok(mut inbox) = inbox.lock() {
                            inbox.push(AttachedTerminal {
                                terminal_id,
                                attachment,
                                held_by: TerminalHolder::CommitPane,
                            });
                        }
                    }
                    Err(error) => pane.error = Some(format!("{error}")),
                }
            },
        );
    }

    /// Notice a run that has ended, and ask how it went. What git printed stays on screen
    /// either way; what changes is what the pane does next.
    fn poll_commit_run(&mut self, session_id: &str) {
        let Some(pane) = self.model.commit_panes.get(session_id) else {
            return;
        };
        let Some(run) = &pane.run else {
            return;
        };
        if run.exit_code.is_some() || run.asks >= OUTCOME_ASKS {
            return;
        }
        let ended = self
            .commit_terminals
            .get(&run.terminal_id)
            .is_some_and(egui_tty::Terminal::has_exited);
        if !ended {
            return;
        }

        let key = format!("commit-outcome:{session_id}");
        if self.tasks.is_busy(&key) {
            return;
        }
        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        let terminal_id = run.terminal_id.clone();

        self.tasks.spawn_keyed(
            Some(key),
            move |backend| backend.commit_run_outcome(&for_call, &terminal_id),
            move |model, result| {
                let Some(pane) = model.commit_panes.get_mut(&for_apply) else {
                    return;
                };
                let Some(run) = &mut pane.run else {
                    return;
                };
                run.asks = run.asks.saturating_add(1);
                match result {
                    Ok(None) => return,
                    Ok(Some(exit_code)) => run.exit_code = Some(exit_code),
                    Err(error) => {
                        pane.error = Some(format!("{error}"));
                        run.asks = OUTCOME_ASKS;
                    }
                }

                let worked = run.worked();
                let clears_message = run.clears_message;
                let note = run.words.worked;
                if worked && clears_message {
                    pane.message.clear();
                }
                // Either way the repo has moved on: a refused commit may still have run a
                // hook that changed the tree.
                pane.stale = true;
                if worked {
                    model.info(note.to_string());
                }
                model.review(&for_apply).refresh_requested = true;
            },
        );
    }

    /// Draw the pty a run is going in, or went in. Unlike a shell's pane this one is kept
    /// after git is gone: what it printed is the account of how the run went.
    fn draw_commit_terminal(&mut self, ui: &mut Ui, terminal_id: &str, takes_keyboard: bool) {
        let palette = self.palette_of();
        let Some(terminal) = self.commit_terminals.get_mut(terminal_id) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("starting git…").color(palette.muted));
            });
            return;
        };
        if takes_keyboard {
            terminal.request_focus();
        }
        terminal.ui(ui, &palette.terminal_style());
    }
}

pub(crate) fn draw(app: &mut App, ui: &mut Ui, pane_id: PaneId, session_id: &str) {
    let palette = app.palette_of();
    app.commit_pane(session_id);
    app.refresh_commit_state(session_id);
    app.poll_commit_run(session_id);

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
    let run_note = pane.run.as_ref().map(|run| match run.exit_code {
        None => (run.words.running, palette.muted),
        Some(0) => (run.words.worked, palette.added),
        Some(_) => (run.words.failed, palette.warn),
    });
    let error = pane.error.clone();
    let state = pane.state.clone();
    let mut message = pane.message.clone();

    egui::Panel::top(egui::Id::new(("commit-header", session_id)))
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(2, 4)))
        .show(ui, |ui| draw_branch_line(ui, state.as_ref(), &palette));

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
            // where they stay put as the staged listing under them grows and shrinks.
            let output = egui::TextEdit::multiline(&mut message)
                .hint_text("what this commit does")
                .desired_width(f32::INFINITY)
                .desired_rows(MESSAGE_ROWS)
                .show(ui);
            if takes_keyboard && !running {
                output.response.request_focus();
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
                        COMMIT_WORDS,
                    );
                }
                if !can_commit && !running {
                    commit.on_disabled_hover_text("a commit takes a staged change and a message");
                }

                if widgets::clickable(ui.add_enabled(!running, egui::Button::new("push"))).clicked()
                {
                    app.start_commit_run(session_id, CommitAction::Push, PUSH_WORDS);
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
}

fn draw_branch_line(ui: &mut Ui, state: Option<&CommitState>, palette: &Palette) {
    let Some(state) = state else {
        ui.label(RichText::new("reading the repo…").color(palette.muted));
        return;
    };

    ui.horizontal(|ui| {
        let branch = state.branch_name.as_deref().unwrap_or("detached HEAD");
        ui.label(RichText::new(branch).strong());
        match &state.upstream_ref {
            Some(upstream) => {
                ui.label(RichText::new("→").color(palette.line));
                ui.label(RichText::new(upstream).color(palette.accent));
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
            RichText::new("nothing staged — stage what to commit in the review")
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
            for file in &state.staged_files {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(change_mark(file.change_kind))
                            .color(change_ink(file.change_kind, palette))
                            .monospace()
                            .size(SMALL_SIZE),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&file.file_path)
                                .color(palette.ink)
                                .size(SMALL_SIZE),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&file.file_path);
                });
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
