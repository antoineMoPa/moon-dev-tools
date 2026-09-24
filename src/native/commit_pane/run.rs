//! A commit or a push running in its shell: started, watched for how it went, and drawn.

use egui::{RichText, Ui};
use web_time::Instant;

use crate::{
    committing::CommitAction,
    native::app::{App, AttachedTerminal, TerminalHolder},
};

use super::{CommitRun, OUTCOME_ASK_INTERVAL, OUTCOME_UNREAD, Reached, RunKind, kind_of};

impl App {
    /// Start one action's program, and attach the pane to the pty it runs in.
    pub(super) fn start_commit_run(&mut self, session_id: &str, action: CommitAction) {
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
    pub(super) fn poll_commit_run(&mut self, session_id: &str) {
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
    pub(super) fn draw_commit_terminal(
        &mut self,
        ui: &mut Ui,
        terminal_id: &str,
        takes_keyboard: bool,
    ) {
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
