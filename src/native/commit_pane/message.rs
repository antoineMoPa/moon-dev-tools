//! The message a commit goes out with: the one a review request wrote for this branch, or one
//! asked of the agent.

use crate::{moontasks::ReviewRequestView, native::app::App};

impl App {
    /// The key one review's message-writing run goes under, which is also how the pane knows
    /// one is in flight.
    pub(super) fn suggestion_key(session_id: &str) -> String {
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
    /// Several tasks can name the same repo, and their lines all resolve to the same working
    /// copy when the branches they name are checked out nowhere - so the repo alone does not
    /// pick one. The branch the repo is actually on does: that is the work sitting in the pane,
    /// and the line that named that branch is the one that wrote the commit for it. Only when
    /// no line named it does the first line for the repo stand, which is the single-line case
    /// and the one where there is nothing better to offer.
    ///
    /// A line on a task that has been finished is not among them at all, not even for the
    /// header's `asked for` pill: the card being in that column is the person saying the work is
    /// behind them, and a commit pane opened afterwards has nothing to hear from it.
    pub(super) fn requested_review_of(&self, session_id: &str) -> Option<&ReviewRequestView> {
        let repo_path = &self
            .model
            .review_ref(session_id)?
            .payload
            .as_ref()?
            .repo_path;
        let mut for_this_repo = self
            .model
            .review_requests
            .iter()
            .filter(|request| &request.repo_path == repo_path && !request.task_finished);
        let first = for_this_repo.next()?;
        let Some(branch) = self.branch_of_commit_pane(session_id) else {
            return Some(first);
        };
        std::iter::once(first)
            .chain(for_this_repo)
            .find(|request| request.branch.as_deref() == Some(branch))
            .or(Some(first))
    }

    /// The line whose commit this pane is about to make, which is the one whose message goes in
    /// the box.
    ///
    /// Narrower than [`Self::requested_review_of`], which answers for the header: a message is
    /// put in someone's box, so the line has to be about this commit and not merely about this
    /// repo. Where no line named the branch, that one answers with the first line for the repo
    /// and this one answers with nothing.
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
            Some(branch) => {
                (Some(branch.as_str()) == self.branch_of_commit_pane(session_id)).then_some(request)
            }
            None => Some(request),
        }
    }

    /// The branch the repo a pane is committing is on, once git has been asked. `None` until
    /// then, and on a detached HEAD.
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
    /// come from the diff. It is in the box as soon as git has said which branch the repo is on,
    /// so what is about to be committed is readable while the hunks are still being picked next
    /// door. Not before that: which line wrote this commit is answered by the branch, and a box
    /// that has been filled is not filled over - so filling it a frame early would be filling it
    /// with another task's message and keeping it there.
    pub(super) fn fill_in_the_requested_commit(&mut self, session_id: &str) {
        if self.commit_pane(session_id).state.is_none() {
            return;
        }
        let Some(written) = self
            .requested_commit_of(session_id)
            .and_then(|request| request.suggestion.clone())
        else {
            return;
        };
        let pane = self.commit_pane(session_id);
        // Only ever into an empty box, and each message only ever once - so a message someone
        // is writing is never argued with, and neither is a box they have emptied on purpose.
        if pane.requested_commit_put_in.as_ref() == Some(&written)
            || !pane.message.trim().is_empty()
        {
            return;
        }
        pane.message = written.as_message();
        pane.requested_commit_put_in = Some(written);
    }

    /// The one time the pane asks on its own: something is staged, nothing has been written in
    /// the box, and no message has been asked for since there was last nothing to commit.
    pub(super) fn auto_ask_for_commit_message(&mut self, session_id: &str) {
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
}
