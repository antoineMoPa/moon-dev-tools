//! Committing what the review has staged, and pushing it, without leaving the window.
//!
//! Both actions run as `git` in a pty rather than as a captured process, and the pane shows
//! that pty. That is what makes a signed commit work: gpg asks for the passphrase through
//! pinentry, a terminal pinentry needs a terminal to ask on, and this is it. Anything else
//! git wants typed - a hook's question, a push over ssh - lands in the same place.

use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use egui_frames::PaneId;

use crate::{
    api::FileChangeKind,
    commit_suggestion::CommitSuggestion,
    committing::{CommitAction, CommitState},
    moontasks::ReviewRequestView,
    native::{
        app::{App, AttachedTerminal, TerminalHolder},
        theme::{Palette, SMALL_SIZE},
        widgets,
    },
};

/// How tall the run's terminal starts out. A terminal pinentry draws a box some twenty rows
/// tall, and the passphrase has to be readable without dragging anything - so the run takes
/// real room while it is there, and is dragged smaller by anyone who wants it smaller.
const RUN_TERMINAL_HEIGHT: f32 = 320.0;
/// What dragging that divider can leave it at.
const RUN_TERMINAL_RANGE: std::ops::RangeInclusive<f32> = 90.0..=640.0;
/// How many lines of message the box shows before it scrolls: a subject, a blank line, and a
/// couple of lines of body.
const MESSAGE_ROWS: usize = 5;
/// How often the pane asks how the run is going.
///
/// The shell a run goes in outlives the command it was given - that is the point of it - so
/// there is no pty closing to be woken by. The command writes down how it went, and this is
/// how often that is looked for.
const OUTCOME_ASK_INTERVAL: Duration = Duration::from_millis(300);
/// How often an open commit pane rereads what is staged. Staging happens in the review pane
/// beside it, which has no way to tell this one, so it looks again on the review's own poll
/// cadence rather than being told.
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// Which of the three things a run is. What the pane does when one ends differs by this, and
/// only by this: what it says, what it clears, and what it offers next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunKind {
    Commit,
    Push,
    OpenPr,
}

/// What the pane says about a run.
#[derive(Clone, Copy)]
struct RunWords {
    running: &'static str,
    worked: &'static str,
    failed: &'static str,
}

/// How far this review has got, which is what decides the buttons the pane shows: a commit is
/// what there is to push, and a push is what there is to open a pull request on. Only runs that
/// worked move it, and a fresh commit takes it back to having something to push.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Reached {
    #[default]
    Nothing,
    Committed,
    Pushed,
}

/// Stands in for the status of a run the server could not be asked about at all. Reads as a
/// failure, which is what not being able to find out means here.
const OUTCOME_UNREAD: i32 = -1;

const MAP_RUN_KIND_TO_WORDS: [(RunKind, RunWords); 3] = [
    (
        RunKind::Commit,
        RunWords {
            running: "committing…",
            worked: "committed",
            failed: "git would not commit - see below",
        },
    ),
    (
        RunKind::Push,
        RunWords {
            running: "pushing…",
            worked: "pushed",
            failed: "git would not push - see below",
        },
    ),
    (
        RunKind::OpenPr,
        RunWords {
            running: "opening the pull request…",
            worked: "pull request opened in the browser",
            failed: "gh would not open a pull request - see below",
        },
    ),
];

fn words_for(kind: RunKind) -> RunWords {
    MAP_RUN_KIND_TO_WORDS
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, words)| *words)
        .expect("every run kind has words")
}

fn kind_of(action: &CommitAction) -> RunKind {
    match action {
        CommitAction::Commit { .. } => RunKind::Commit,
        CommitAction::Push => RunKind::Push,
        CommitAction::OpenPr => RunKind::OpenPr,
    }
}

/// One review's commit pane. Kept per review rather than per pane, so closing the tab does not
/// throw away a message that was half written.
pub(crate) struct CommitPane {
    pub(crate) message: String,
    /// What git says the repo looks like, once it has been read.
    state: Option<CommitState>,
    /// Set when that reading is known to be out of date - after a run ends - which reads it
    /// again at once rather than on the next poll.
    stale: bool,
    /// When it was last read, so an open pane keeps up with staging done next door without
    /// running git on every frame it draws.
    last_read: Option<Instant>,
    /// The last run, kept after it ends so its output stays on screen until the next one.
    run: Option<CommitRun>,
    error: Option<String>,
    /// The message an agent wrote for what is staged, waiting under the box for `[use]`.
    suggestion: Option<CommitSuggestion>,
    /// Why there is no suggestion, when asking for one did not work out.
    suggestion_error: Option<String>,
    /// Whether one has been asked for since the pane last had nothing to commit. It is asked
    /// for once and no more - staging happens a hunk at a time next door, and an agent run for
    /// every one of those would be a run for a commit that is still being put together.
    suggestion_asked: bool,
    /// Whether the commit a board task wrote for this repo has been put in the box. Once, and
    /// never again: a box someone has emptied is a message they are writing themselves, and
    /// putting it back on the next frame would be arguing with them.
    requested_commit_filled: bool,
    /// Set when a commit has just worked, and answered by the next reading of the repo: it is
    /// that reading which knows whether the review beside this pane has anything left to show.
    closes_review: bool,
    reached: Reached,
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
            suggestion: None,
            suggestion_error: None,
            suggestion_asked: false,
            requested_commit_filled: false,
            closes_review: false,
            reached: Reached::Nothing,
        }
    }

    /// Put a written message under the box, the way an answer from the agent does. What the
    /// test drives `[use]` with, without an agent run.
    #[cfg(test)]
    pub(crate) fn set_suggestion_for_test(&mut self, suggestion: CommitSuggestion) {
        self.suggestion = Some(suggestion);
        self.suggestion_asked = true;
    }

    /// How many files a commit would take in, once git has been asked. `None` until then -
    /// which is also when the commit button is still off.
    #[cfg(test)]
    pub(crate) fn staged_count_for_test(&self) -> Option<usize> {
        self.state.as_ref().map(|state| state.staged_files.len())
    }

    /// Whether the command is going right now, which is when the buttons are off - and what
    /// makes this pane's shell work in progress for the warning quitting owes.
    pub(crate) fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.exit_code.is_none())
    }
}

struct CommitRun {
    terminal_id: String,
    kind: RunKind,
    /// `None` while the command is going, the status it ended on once it is over.
    exit_code: Option<i32>,
    /// When the pane last asked how it went - see [`OUTCOME_ASK_INTERVAL`].
    last_ask: Option<Instant>,
}

impl CommitRun {
    fn worked(&self) -> bool {
        self.exit_code == Some(0)
    }
}

impl App {
    /// Open the commit pane of a review, down the right so the review it is committing stays on
    /// screen. Of that review: a changed submodule has its own repo, its own branch and its own
    /// pane. Deferred like every other pane opened from inside a pane.
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
        // A read is already on its way. Letting this one past would mark the pane read and
        // wait out the poll interval for an answer that is never spawned.
        let key = format!("commit-state:{session_id}");
        if self.tasks.is_busy(&key) {
            return;
        }
        let pane = self.commit_pane(session_id);
        let due = pane
            .last_read
            .is_none_or(|last| last.elapsed() >= STATE_POLL_INTERVAL);
        if !pane.stale && !due {
            return;
        }
        pane.stale = false;
        pane.last_read = Some(Instant::now());
        // Read now rather than in the apply below, so only a reading that started after the
        // commit gets to answer for it. One that was already in flight predates the commit and
        // cannot say what it left behind.
        let answers_a_commit = pane.closes_review;
        // Read now for the same reason: a reading already in flight when the push ended would
        // answer with the pre-push count and hand the button back a job it has already done.
        let answers_for_the_push = pane.reached == Reached::Pushed;

        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(key),
            move |backend| backend.commit_state(&for_call),
            move |model, result| {
                let Some(pane) = model.commit_panes.get_mut(&for_apply) else {
                    return;
                };
                let mut review_is_over = false;
                match result {
                    Ok(state) => {
                        // A commit that took the whole of the working tree leaves nothing to
                        // review; one that left changes behind leaves them to be reviewed.
                        review_is_over = answers_a_commit
                            && state.staged_files.is_empty()
                            && state.unstaged_count == 0;
                        // A push that worked sent everything there was; commits that turn up
                        // after it - made anywhere - give the push button its job back.
                        if answers_for_the_push
                            && pane.reached == Reached::Pushed
                            && state.ahead > 0
                        {
                            pane.reached = Reached::Committed;
                        }
                        pane.state = Some(state);
                        pane.error = None;
                    }
                    Err(error) => pane.error = Some(format!("{error}")),
                }
                if answers_a_commit {
                    pane.closes_review = false;
                }
                if review_is_over {
                    model.close_review_panes(&for_apply);
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

    /// The key one review's message-writing run goes under, which is also how the pane knows
    /// one is in flight.
    fn suggestion_key(session_id: &str) -> String {
        format!("commit-message:{session_id}")
    }

    /// Ask the agent for a message for what is staged.
    fn ask_for_commit_message(&mut self, session_id: &str) {
        let key = Self::suggestion_key(session_id);
        if self.tasks.is_busy(&key) {
            return;
        }
        let pane = self.commit_pane(session_id);
        pane.suggestion_asked = true;
        pane.suggestion_error = None;

        let for_call = session_id.to_string();
        let for_apply = session_id.to_string();
        self.tasks.spawn_keyed(
            Some(key),
            move |backend| backend.suggest_commit_message(&for_call),
            move |model, result| {
                let Some(pane) = model.commit_panes.get_mut(&for_apply) else {
                    return;
                };
                match result {
                    Ok(suggestion) => {
                        pane.suggestion = Some(suggestion);
                        pane.suggestion_error = None;
                    }
                    // Its own line under the box rather than the pane's error line: a message
                    // that could not be written stops nothing, and the commit button is still
                    // there for the message the user writes instead.
                    Err(error) => {
                        pane.suggestion = None;
                        pane.suggestion_error = Some(format!("{error}"));
                    }
                }
            },
        );
    }

    /// What one of the board's tasks asked to have looked at in the repo this pane is
    /// committing, if any of them did - see [`crate::moontasks::ReviewRequestView`].
    ///
    /// Found by the repo rather than by the task: a pane is opened on a repo, and which task
    /// wrote the line that sent you there is not something it has to know.
    ///
    /// Several tasks can name the same repo, so the branch decides between them: the line asking
    /// for the branch the pane is on is the one about this commit. When none of them names it -
    /// which is the case the header's `asked for` pill is drawn for - the first line for the repo
    /// answers, because the pill has something to say either way and there is nothing better to
    /// say it about.
    ///
    /// A line on a task that has been finished is not answered with at all, not even for the
    /// pill: the card being in that column is the person saying the work is behind them, and a
    /// commit pane opened afterwards has nothing to hear from it.
    pub(super) fn requested_review_of(&self, session_id: &str) -> Option<&ReviewRequestView> {
        let repo_path = &self.model.review_ref(session_id)?.payload.as_ref()?.repo_path;
        let branch = self.branch_of_commit_pane(session_id);
        let for_this_repo = || {
            self.model
                .review_requests
                .iter()
                .filter(|request| &request.repo_path == repo_path && !request.task_finished)
        };
        for_this_repo()
            .find(|request| request.branch.is_some() && request.branch.as_deref() == branch)
            .or_else(|| for_this_repo().next())
    }

    /// The line whose commit this pane is about to make, which is the one whose message goes in
    /// the box.
    ///
    /// Narrower than [`Self::requested_review_of`], which answers for the header: a message is
    /// put in someone's box, so the line has to be about this commit and not merely about this
    /// repo.
    ///
    /// A line naming a branch the pane is not on is about work that lives somewhere else. Most
    /// often it is work already committed and merged: the branch is checked out nowhere any
    /// more, so the line resolves back onto the main checkout, where the next piece of work is
    /// now being written - and the message for the finished branch would land on it. A line
    /// crossed off by hand is finished with too, whatever branch it names.
    fn requested_commit_of(&self, session_id: &str) -> Option<&ReviewRequestView> {
        let request = self.requested_review_of(session_id)?;
        if request.done {
            return None;
        }
        match &request.branch {
            Some(branch) => (Some(branch.as_str()) == self.branch_of_commit_pane(session_id))
                .then_some(request),
            None => Some(request),
        }
    }

    /// The branch the pane's repo is on, once the pane has read it. `None` while that read is
    /// still going, and for a detached HEAD - neither of which is a branch a line can have asked
    /// for, so both hold the message back rather than letting the wrong one through.
    fn branch_of_commit_pane(&self, session_id: &str) -> Option<&str> {
        self.model
            .commit_panes
            .get(session_id)?
            .state
            .as_ref()?
            .branch_name
            .as_deref()
    }

    /// Put the commit a board task wrote for this repo in the box.
    ///
    /// Straight in the box rather than under it behind `[use]`: that gate is there for a message
    /// a model guessed from the diff, and this one was written by whoever did the work, for this
    /// repo, and is the message meant to be made. It is text like any other once it is there.
    ///
    /// Nothing to do with staging, unlike the message written from the diff - this one does not
    /// come from the diff. It is in the box from the moment the pane has read which branch it is
    /// on, so what is about to be committed is readable while the hunks are still being picked
    /// next door.
    fn fill_in_the_requested_commit(&mut self, session_id: &str) {
        let Some(written) = self
            .requested_commit_of(session_id)
            .and_then(|request| request.suggestion.clone())
        else {
            return;
        };
        let pane = self.commit_pane(session_id);
        // Only ever into an empty box, and only ever once - so a message someone is writing is
        // never argued with, and neither is a box they have emptied on purpose.
        if pane.requested_commit_filled || !pane.message.trim().is_empty() {
            return;
        }
        pane.message = written.as_message();
        pane.requested_commit_filled = true;
    }

    /// The one time the pane asks on its own: something is staged, nothing has been written in
    /// the box, and no message has been asked for since there was last nothing to commit.
    fn auto_ask_for_commit_message(&mut self, session_id: &str) {
        let pane = self.commit_pane(session_id);
        let Some(state) = &pane.state else {
            return;
        };
        if state.staged_files.is_empty() {
            // Nothing staged is a different commit from whatever was staged before it, so the
            // message written for that one goes with it.
            pane.suggestion = None;
            pane.suggestion_error = None;
            pane.suggestion_asked = false;
            return;
        }
        if pane.suggestion_asked || !pane.message.trim().is_empty() || pane.is_running() {
            return;
        }

        // Writing one from the diff is an agent run. A test stages a fixture, so under test that
        // would start a real agent on it - there the pane asks only when pressed.
        if cfg!(test) || !state.opencode_installed {
            return;
        }
        self.ask_for_commit_message(session_id);
    }

    /// Start one action's program, and attach the pane to the pty it runs in.
    fn start_commit_run(&mut self, session_id: &str, action: CommitAction) {
        let kind = kind_of(&action);
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
                            kind,
                            exit_code: None,
                            last_ask: None,
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

    /// Ask how the run is going. What the command printed stays on the shell either way; what
    /// changes is what the pane does next.
    fn poll_commit_run(&mut self, session_id: &str) {
        let key = format!("commit-outcome:{session_id}");
        if self.tasks.is_busy(&key) {
            return;
        }
        let Some(pane) = self.model.commit_panes.get_mut(session_id) else {
            return;
        };
        let Some(run) = &mut pane.run else {
            return;
        };
        if run.exit_code.is_some()
            || run
                .last_ask
                .is_some_and(|last| last.elapsed() < OUTCOME_ASK_INTERVAL)
        {
            return;
        }
        run.last_ask = Some(Instant::now());

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
                match result {
                    Ok(None) => return,
                    Ok(Some(exit_code)) => run.exit_code = Some(exit_code),
                    Err(error) => {
                        // Nothing more is going to answer for this run, and a pane that waited
                        // on it forever would never let the buttons back on.
                        pane.error = Some(format!("{error}"));
                        run.exit_code = Some(OUTCOME_UNREAD);
                    }
                }

                // A run that worked says so beside the buttons, and the staged listing
                // emptying says it louder; a toast on top of both would be a third telling.
                if run.worked() && run.kind == RunKind::Commit {
                    pane.message.clear();
                    // The message it wrote is in the commit that was just made; whatever is
                    // staged next is a different commit, and gets a message of its own.
                    pane.suggestion = None;
                    pane.suggestion_error = None;
                    pane.suggestion_asked = false;
                    pane.closes_review = true;
                }
                let pull_request_is_open = run.worked() && run.kind == RunKind::OpenPr;
                if run.worked() {
                    pane.reached = match run.kind {
                        RunKind::Commit => Reached::Committed,
                        RunKind::Push => Reached::Pushed,
                        // The pull request was opened on what the push sent; nothing moved.
                        RunKind::OpenPr => pane.reached,
                    };
                }
                // Either way the repo has moved on: a refused commit may still have run a
                // hook that changed the tree.
                pane.stale = true;
                model.review(&for_apply).refresh_requested = true;

                // The pull request is the last thing this review is for: it is open in the
                // browser, and what is left on screen is a pane with no button still worth
                // pressing. Closing both is what the user would do next by hand, and the
                // window goes with them when they were the last of it.
                if pull_request_is_open {
                    model.close_review_panes(&for_apply);
                    model.close_commit_pane(&for_apply);
                }
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
            // where they stay put as the staged listing under them grows and shrinks.
            let output = egui::TextEdit::multiline(&mut message)
                .hint_text("what this commit does")
                .desired_width(f32::INFINITY)
                .desired_rows(MESSAGE_ROWS)
                .show(ui);
            if takes_keyboard && !running {
                output.response.request_focus();
            }

            // Under the box, where a message written by the agent reads as an offer of what to
            // put in it rather than as something already in it.
            ui.add_space(4.0);
            if draw_suggested_message(
                ui,
                &palette,
                suggestion.as_ref(),
                suggestion_error.as_deref(),
                writing_message,
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
/// it did not. Answers whether `[use]` was pressed, which is read after the pane is drawn -
/// the row is inside a closure that has the pane borrowed.
pub(super) fn draw_suggested_message(
    ui: &mut Ui,
    palette: &Palette,
    suggestion: Option<&CommitSuggestion>,
    error: Option<&str>,
    writing: bool,
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
            used = widgets::clickable(ui.button("use"))
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
                    label.on_hover_text("pushing sends it to origin under its own name and tracks it there");
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
        let elsewhere =
            asked_branch.filter(|asked| Some(*asked) != state.branch_name.as_deref());
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
            for (directory, files) in widgets::by_directory(state.staged_files.iter(), |file| file.file_path.as_str()) {
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
