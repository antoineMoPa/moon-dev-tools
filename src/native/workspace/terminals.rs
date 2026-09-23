//! The shells once they are running: attached, named, drawn, counted, and their tabs closed
//! when they end.

use std::sync::Arc;

use egui::{RichText, Ui};
use egui_frames::PaneId;

use crate::native::{
    app::{App, AttachedTerminal, TerminalHolder},
    model::TabRename,
    panes::Pane,
};

impl App {
    /// Open a shell's tab title for retyping, which is what a double click on the tab asks.
    /// Only a shell's: every other tab is named by what it shows.
    pub(crate) fn open_tab_rename(&mut self, pane_id: PaneId) {
        let Some(pane) = self.model.layout.pane(pane_id) else {
            return;
        };
        if !matches!(pane, Pane::Terminal { .. }) {
            return;
        }
        // The first of the two clicks brought the tab forward and promised its shell the
        // keyboard. The box being opened here is what the keyboard was reached for, so the
        // promise is taken back - a shell still attaching would otherwise take it frames
        // later, out of a box that has been typed into by then.
        self.pane_taking_keyboard = None;
        self.model.renaming_tab = Some(TabRename {
            pane_id,
            name: self.shell_tab_title(pane),
            focus: true,
        });
    }

    /// Ask the server what a shell is called, for a tab drawn before the answer is known -
    /// see [`Model::terminal_names`]. Asked once: what comes back is kept, name or none.
    pub(crate) fn read_terminal_name(&mut self, terminal_id: &str) {
        let session_id = self.model.root_session_id.clone();
        let terminal_id = terminal_id.to_string();
        let for_model = terminal_id.clone();
        self.tasks.spawn_keyed(
            Some(format!("name:{terminal_id}")),
            move |backend| backend.terminal_name(&session_id, &terminal_id),
            move |model, result| {
                // A shell the server no longer has is a shell with no name to read; that it
                // is gone is for the attachment to report.
                model
                    .terminal_names
                    .insert(for_model, result.unwrap_or(None));
            },
        );
    }

    /// Call a shell something else: on its tab now, and on the server behind it, where the
    /// board reads it from.
    pub(crate) fn rename_terminal(&mut self, terminal_id: String, name: String) {
        self.model
            .terminal_names
            .insert(terminal_id.clone(), Some(name.clone()));
        let session_id = self.model.root_session_id.clone();
        self.tasks.spawn(
            move |backend| backend.rename_terminal(&session_id, &terminal_id, &name),
            |model, result| {
                model.report(result, "could not rename the shell");
                model.board.refresh_requested = true;
            },
        );
    }

    /// Reattach a shell whose pane is on screen but whose emulator is not - which happens when
    /// a restored arrangement mentions a terminal this window has not attached yet.
    pub(crate) fn attach_terminal(&mut self, terminal_id: &str) {
        let key = format!("attach:{terminal_id}");
        if self.tasks.is_busy(&key) || self.terminal_errors.contains_key(terminal_id) {
            return;
        }
        let session_id = self.model.root_session_id.clone();
        let inbox = Arc::clone(&self.attaching);
        let terminal_id = terminal_id.to_string();
        let for_inbox = terminal_id.clone();

        self.tasks.spawn_keyed(
            Some(key),
            move |backend| Ok(backend.attach_terminal(&session_id, &terminal_id)),
            move |_model, result| {
                let attachment = result.and_then(|attachment| attachment);
                if let Ok(mut inbox) = inbox.lock() {
                    inbox.push(AttachedTerminal {
                        terminal_id: for_inbox,
                        attachment,
                        held_by: TerminalHolder::Workspace,
                    });
                }
            },
        );
    }

    /// Turn shells that finished attaching into live panes. They arrive from a worker thread,
    /// because a remote one opens a socket; the emulator itself is `!Send`, so it is built here.
    pub(crate) fn drain_attachments(&mut self) {
        let ready = {
            let Ok(mut inbox) = self.attaching.lock() else {
                return;
            };
            std::mem::take(&mut *inbox)
        };

        for attached in ready {
            let AttachedTerminal {
                terminal_id,
                attachment,
                held_by,
            } = attached;
            let opened = attachment.and_then(|stream| {
                egui_tty::Terminal::new(stream)
                    .map(|terminal| terminal.with_label(terminal_id.clone()))
                    .map_err(|error| anyhow::anyhow!("{error}"))
            });

            match opened {
                // A shell the user just opened starts with the keyboard, so they can type into
                // it without clicking first - its tab comes to the front as it opens, and the
                // front tab is the one with the keyboard. See `follow_front_tab`.
                Ok(terminal) => {
                    self.terminal_errors.remove(&terminal_id);
                    match held_by {
                        TerminalHolder::Workspace => {
                            self.terminals.insert(terminal_id, terminal);
                        }
                        TerminalHolder::CommitPane => {
                            self.commit_terminals.insert(terminal_id, terminal);
                        }
                    }
                }
                Err(error) => {
                    let message = format!("{error}");
                    self.model
                        .error(format!("shell {terminal_id} is unavailable: {message}"));
                    self.terminal_errors.insert(terminal_id, message);
                }
            }
        }
    }

    /// A shell that has ended takes its tab with it, and the frame too when it was the last tab
    /// there - logging out of a terminal or an agent finishing should leave the workspace as it
    /// was before the shell was opened.
    ///
    /// An agent that fell over is never among them: the server keeps its shell and does not
    /// mark it exited, so the tab stays open on the error - see `failure_notice` in
    /// `crate::terminal`.
    ///
    /// One a frame: closing a pane rebuilds the tree, and the next frame picks up the next.
    ///
    /// Every shell is read here first, whether or not its tab is the one in front. A terminal
    /// only learns its program has ended from reading it, and drawing is what otherwise reads
    /// it - so a shell that ended behind another tab would go on counting as open: its tab
    /// kept, the restart its end was to set off never started, and the next build typed at a
    /// pty with nothing on the other end.
    pub(crate) fn close_tabs_of_exited_shells(&mut self, ctx: &egui::Context) {
        for terminal in self.terminals.values_mut() {
            terminal.poll();
        }
        let Some(terminal_id) = self
            .terminals
            .iter()
            .find(|(_, terminal)| terminal.has_exited())
            .map(|(terminal_id, _)| terminal_id.clone())
        else {
            return;
        };

        // The build whose end was to restart the window: the line typed into it only exits on
        // a build that came out well, so this shell ending is the rebuilt program being ready
        // to start. The window closing takes the tab with it, so there is nothing left to
        // close here - unless the restart could not start a new window, in which case the
        // toast it left says so and the tab closes on the next frame like any other.
        if self.model.restart_on_shell_exit.as_deref() == Some(terminal_id.as_str()) {
            self.model.restart_on_shell_exit = None;
            self.restart_window(ctx);
            return;
        }

        let pane = self
            .model
            .layout
            .find_pane(|pane| matches!(pane, Pane::Terminal { terminal_id: of_pane, .. } if *of_pane == terminal_id))
            .map(|(pane, _)| pane);

        match pane {
            Some(pane) => self.close_pane(pane),
            // No tab is showing it, so there is nothing to close but the shell itself.
            None => {
                self.terminals.remove(&terminal_id);
                self.model.terminal_names.remove(&terminal_id);
            }
        }
    }

    /// Draw a shell, or say why there isn't one to draw.
    pub(crate) fn draw_terminal(&mut self, ui: &mut Ui, pane_id: PaneId, terminal_id: &str) {
        let palette = self.palette_of();
        if let Some(error) = self.terminal_errors.get(terminal_id) {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new(error.clone()).color(palette.warn));
            });
            return;
        }

        let Some(terminal) = self.terminals.get_mut(terminal_id) else {
            self.attach_terminal(terminal_id);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("attaching…").color(palette.muted));
            });
            return;
        };

        // The shell takes the keyboard it is owed. A tab brought forward while its shell is
        // still attaching keeps the offer until the emulator is there to take it, which is why
        // this is below the wait above rather than at the top.
        if self.pane_taking_keyboard == Some(pane_id) {
            self.pane_taking_keyboard = None;
            terminal.request_focus();
        }

        // The find bar asks for a search only when its query or its place in the matches moved,
        // because a search reads the whole scrollback.
        let searching = self
            .model
            .find
            .as_ref()
            .filter(|find| find.pane_id == pane_id && find.pending)
            .map(|find| (find.query.clone(), find.at));
        let found = searching.map(|(query, at)| terminal.find(&query, at));

        let response = terminal.ui(ui, &palette.terminal_style());
        // Remembered so the review's copy chord can tell "the keyboard is in a shell" from
        // any other focus - see `review::hunks::copy_selected_lines`.
        if response.has_focus() {
            self.model.terminal_with_keyboard = Some(response.id);
        }

        if let Some(total) = found
            && let Some(find) = &mut self.model.find
        {
            find.found(total);
        }
    }

    /// How long until the window is worth drawing again on account of its shells: a live one
    /// asks for itself, so this is only about the review's own polling.
    pub(crate) fn has_live_shell(&self) -> bool {
        self.running_shells() > 0
    }

    /// How many of the window's shells have a command running in them, which is what quitting
    /// would interrupt. A shell waiting at its prompt is not one of them: nothing is lost by
    /// closing it, so there is nothing to ask about.
    ///
    /// Which shells those are is the server's answer, polled - see `App::poll_running_shells`.
    /// A commit or a push in flight counts the same way it does for [`Self::running_shells`].
    pub(crate) fn shells_running_a_command(&self) -> usize {
        let busy_shells = self
            .terminals
            .keys()
            .filter(|terminal_id| self.model.shells_running_a_command.contains(terminal_id))
            .count();
        let running_commands = self
            .model
            .commit_panes
            .values()
            .filter(|pane| pane.is_running())
            .count();
        busy_shells + running_commands
    }

    /// How many shells are open and have not ended, whatever they are doing: this is what
    /// keeps the window repainting, since a live shell can print at any time. What quitting
    /// would interrupt is [`Self::shells_running_a_command`], which is a smaller number.
    ///
    /// A commit pane's shell stays on after its command is done, to carry on working in - so it
    /// counts while its command is going rather than for as long as it is open.
    pub(crate) fn running_shells(&self) -> usize {
        let workspace_shells = self
            .terminals
            .values()
            .filter(|terminal| !terminal.has_exited())
            .count();
        let running_commands = self
            .model
            .commit_panes
            .values()
            .filter(|pane| pane.is_running())
            .count();
        workspace_shells + running_commands
    }
}
