//! Committing what the review has staged, and pushing it, without leaving the window.
//!
//! Both actions run as `git` in a pty rather than as a captured process, and the pane shows
//! that pty. That is what makes a signed commit work: gpg asks for the passphrase through
//! pinentry, a terminal pinentry needs a terminal to ask on, and this is it. Anything else
//! git wants typed - a hook's question, a push over ssh - lands in the same place.

pub(super) mod drawing;
mod message;
mod run;

pub(crate) use drawing::draw;

use std::time::{Duration, Instant};

use crate::{
    commit_suggestion::CommitSuggestion,
    committing::{CommitAction, CommitState},
    native::app::App,
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
    /// The commit a board task wrote for this repo that was last put in the box. Each one goes
    /// in once and never again: a box someone has emptied - by hand, or by making that commit -
    /// is not to have it put back on the next frame. Kept as the message rather than as a yes
    /// or no because the pane lives as long as the window, and the repo's next commit is asked
    /// for by another line, with a message of its own.
    requested_commit_put_in: Option<CommitSuggestion>,
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
            requested_commit_put_in: None,
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
}
