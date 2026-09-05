//! The workspace: which panes are open, where they go, and the shells behind them.
//!
//! `egui_frames` draws the arrangement and answers the pointer; `egui_tty` is what a shell pane
//! holds. What is left here is everything only moonreview can decide: where a new pane belongs,
//! what closing one costs, and which shell a tab is showing.

use std::{sync::Arc, time::Duration};

use egui::{RichText, Ui};
use egui_frames::{DropSide, FrameId, FramesEvent, Layout, PaneId};

use crate::{
    api::{AgentKind, OpenSessionRequest},
    cli::Frame,
    native::{
        app::{App, AttachedTerminal, TerminalHolder},
        bindings,
        model::{PendingCard, TabRename},
        panes::{OpenPaneRequest, Pane, PaneKind},
    },
    project::ProjectCommand,
};

/// The narrowest a frame may be left at by opening a shell beside it. Below this, the shell
/// joins a frame's tabs instead of taking a column of its own.
const MIN_COLUMN_WIDTH: f32 = 320.0;

/// A breath between the top of the window and the first frame's border. Frames are separated
/// from each other by their dividers, so only the window's own top edge reads as cramped.
const WORKSPACE_TOP_INSET: f32 = 4.0;

/// Where a new shell's pane lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalPlacement {
    /// Beside the shells already open, or in a new column down the right if there are none.
    WithOtherShells,
    /// A new full-height column down the right of the workspace.
    RightColumn,
    /// A frame of its own against one side of a frame, splitting it in two. This is what the
    /// palette's split commands ask for.
    Beside { frame: FrameId, side: DropSide },
    /// Another tab in this frame, for a workspace with no room left to split.
    Tab(FrameId),
}

/// The arrangement a run starts from: the shape the last one left behind, with this run's
/// review in the frame whose tab strip is the app header.
///
/// A stored arrangement is worth keeping for its shape - where the user put their columns and
/// rows. Its panes are not: a review pane names a session that no longer exists, and a shell
/// pane names a process that died with the last run. Whatever the review and the adopted shells
/// do not fill is dropped on the first frame drawn.
pub(crate) fn arrangement_for(
    stored: Option<Layout<Pane>>,
    session_id: &str,
    frame: Frame,
) -> Layout<Pane> {
    let mut layout = match stored {
        Some(mut stored) if stored.is_coherent() => {
            stored.take_panes();
            stored
        }
        _ => Layout::new(),
    };

    let primary = layout.primary_frame();
    match frame {
        Frame::Review => layout.add_pane(
            primary,
            Pane::Review {
                session_id: session_id.to_string(),
                title: "review".to_string(),
            },
            None,
        ),
        Frame::Tasks => layout.add_pane(primary, Pane::Tasks, None),
        // A shell has to be started before there is a pane to show it, so this window opens
        // with nothing in it and the shell arrives a moment later.
        Frame::Shell => return layout,
    };
    layout
}

impl App {
    /// One frame of the arrangement: drawn, dragged, and whatever the user asked of it done.
    pub(crate) fn draw_workspace(&mut self, ui: &mut Ui) {
        // An empty frame is a leftover - whatever emptied it should have taken it with it - and
        // the only way to see one is the hint its body draws. Dropping them here means no path
        // can leave one on screen, whatever it forgot. A workspace with nothing open at all
        // keeps its single frame: that is a state rather than a leftover.
        self.model.layout.drop_empty_frames();
        self.follow_front_tab(ui.ctx());
        self.follow_task_in_front();
        self.stamp_tab_shortcuts();
        *self.frames.style_mut() = self.palette_of().frames_style();

        ui.add_space(WORKSPACE_TOP_INSET);

        // The workspace draws moonreview's own panes, so the view it needs is this app: both it
        // and the arrangement are lent out for the call and put back straight after.
        let mut frames = std::mem::take(&mut self.frames);
        let mut layout = std::mem::take(&mut self.model.layout);
        let events = frames.show(ui, &mut layout, self);
        self.model.layout = layout;
        self.frames = frames;

        for event in events {
            match event {
                // Deferred: a pane must not be taken out of the tree that is drawing it.
                FramesEvent::PaneCloseRequested(pane) => self.pending_close = Some(pane),
                FramesEvent::NewTabRequested(frame) => self.open_shell_beside(frame),
                FramesEvent::TabDoubleClicked(pane) => self.open_tab_rename(pane),
            }
        }
    }

    /// Open a pane where its kind belongs: reviews with reviews, shells with shells, and a
    /// brand new right-hand column for the first shell.
    pub(crate) fn open_pane(&mut self, request: OpenPaneRequest) {
        let active_frame = self.model.layout.active_frame();

        match request {
            OpenPaneRequest::Review { session_id, title } => {
                // A review that is already open is brought forward instead of duplicated.
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.reviews(&session_id))
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Review, active_frame);
                self.model.review(&session_id);
                self.model
                    .layout
                    .add_pane(frame, Pane::Review { session_id, title }, None);
            }
            OpenPaneRequest::ReviewRepo { repo_path, title } => {
                // The session has to exist before a pane can point at it, and creating one runs
                // git in the repo, so the pane appears once that comes back.
                self.tasks.spawn(
                    move |backend| {
                        backend.open_session(OpenSessionRequest {
                            repo_path,
                            diff_target: None,
                            active_commit: None,
                        })
                    },
                    move |model, result| match result {
                        Ok(opened) => {
                            // A review of a repo already open is brought forward instead of
                            // opened a second time - the same answer opening it by session
                            // gives. The repo names the session: asking for one on a repo that
                            // already has a session answers with that session rather than a
                            // new one, so the two are the same review however it was reached,
                            // whether from a submodule row or a card's `[start]`.
                            if let Some((pane, _)) = model
                                .layout
                                .find_pane(|pane| pane.reviews(&opened.session_id))
                            {
                                model.layout.focus_pane(pane);
                                return;
                            }
                            model.review(&opened.session_id);
                            let frame = model
                                .layout
                                .frame_holding(model.layout.active_frame(), |pane| {
                                    pane.kind() == PaneKind::Review
                                })
                                .unwrap_or_else(|| model.layout.primary_frame());
                            model.layout.add_pane(
                                frame,
                                Pane::Review {
                                    session_id: opened.session_id,
                                    title,
                                },
                                None,
                            );
                        }
                        Err(error) => model.error(format!("could not open that review: {error}")),
                    },
                );
            }
            OpenPaneRequest::Agents => {
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Agents)
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Agents, active_frame);
                self.model.layout.add_pane(frame, Pane::Agents, None);
            }
            OpenPaneRequest::File {
                session_id,
                file_path,
                at,
            } => {
                // The same file twice is the same tab: opening it again brings it forward.
                let pane_id = match self.model.layout.find_pane(
                    |pane| matches!(pane, Pane::File { file_path: open, .. } if *open == file_path),
                ) {
                    Some((pane, _)) => {
                        self.model.layout.focus_pane(pane);
                        pane
                    }
                    None => {
                        let frame = self.frame_for(PaneKind::File, active_frame);
                        self.model.layout.add_pane(
                            frame,
                            Pane::File {
                                session_id: session_id.clone(),
                                file_path: file_path.clone(),
                                // Opened by name or from a search, which is the repo's file
                                // rather than any one task's.
                                task_id: None,
                            },
                            None,
                        )
                    }
                };
                if let Some(at) = at {
                    self.reveal_file_match(pane_id, &session_id, &file_path, at);
                }
            }
            OpenPaneRequest::Terminal { command } => {
                let session_id = self.shell_session_for(active_frame);
                self.spawn_terminal(session_id, command, TerminalPlacement::WithOtherShells);
            }
            OpenPaneRequest::AttachTerminal {
                terminal_id,
                command,
                task_id,
            } => {
                // The start window that was standing in for this shell, if the task had one:
                // the shell opens in its place and it closes behind, because what it was
                // offering has now happened.
                let standing_in = task_id
                    .as_deref()
                    .and_then(|task| self.start_pane_of(task))
                    .filter(|pane| self.model.layout.frame_of(*pane).is_some());

                // The shell is already running on the server; all this opens is a way to see it.
                match self.model.layout.find_pane(
                    |pane| matches!(pane, Pane::Terminal { terminal_id: open, .. } if *open == terminal_id),
                ) {
                    Some((pane, _)) => self.model.layout.focus_pane(pane),
                    None => {
                        // A task's shell is one of that task's tabs, so it goes where the rest
                        // of them go: the column beside the board. Left to `WithOtherShells` it
                        // would only join a frame that already holds a shell, and the column
                        // holding a start window or a task's notes and nothing else would be
                        // split again for every agent started.
                        let column = task_id.is_some().then(|| self.task_column()).flatten();
                        let shell = Pane::Terminal {
                            terminal_id: terminal_id.clone(),
                            command,
                            task_id,
                        };
                        match standing_in.and_then(|pane| self.model.layout.frame_of(pane)) {
                            // In the start window's own frame, and in its place among the
                            // tabs, so the tab the shell arrives in is the one that was there.
                            Some(frame) => {
                                self.model.layout.add_pane(frame, shell, standing_in);
                            }
                            None => match column {
                                Some(frame) => {
                                    self.model.layout.add_pane(frame, shell, None);
                                }
                                None => place_shell(
                                    &mut self.model.layout,
                                    &TerminalPlacement::WithOtherShells,
                                    shell,
                                ),
                            },
                        }
                        self.attach_terminal(&terminal_id);
                    }
                }

                if let Some(pane) = standing_in {
                    self.close_pane(pane);
                }
            }
            OpenPaneRequest::TaskStart { task_id, title } => {
                // One start window a task: asking for it again brings it forward.
                if let Some(pane) = self.start_pane_of(&task_id) {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                self.open_beside_the_board(Pane::Start { task_id, title });
            }
            OpenPaneRequest::NewTask {
                column,
                joins,
                draft_id,
            } => {
                // One new-task pane at a time, and the `+` pressed again brings it forward with
                // what is already written on it: a half-named task is not something to sweep up
                // behind the person writing it.
                if let Some((pane, open)) = self
                    .model
                    .layout
                    .find_pane(|pane| matches!(pane, Pane::NewTask { .. }))
                {
                    // The empty card stays where the pane that is open is writing it, not where
                    // the `+` just pressed would have put one.
                    if let Pane::NewTask { column, joins, .. } = open {
                        self.model.board.card_being_written = Some(PendingCard {
                            column: column.clone(),
                            joins: *joins,
                        });
                    }
                    self.model.board.drafts.remove(&draft_id);
                    self.model.layout.focus_pane(pane);
                    return;
                }
                // The column draws an empty card at that end for as long as the pane is open,
                // so the task has its place on the board while it is being written.
                self.model.board.card_being_written = Some(PendingCard {
                    column: column.clone(),
                    joins,
                });
                self.open_beside_the_board(Pane::NewTask {
                    column,
                    joins,
                    draft_id,
                });
            }
            OpenPaneRequest::Commit { session_id } => {
                // One commit pane a review: opening it again brings it forward, with whatever
                // message was already written still in it.
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.commits(&session_id))
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                // Down the right of the workspace, so the review it is committing stays on
                // screen beside it - among the tabs already there rather than in a column of
                // its own, which would take its width off a review that is being read while
                // the message is written.
                let column = self.column_beside(|open| open.reviews(&session_id));
                let pane = Pane::Commit { session_id };
                match column {
                    Some(frame) => {
                        self.model.layout.add_pane(frame, pane, None);
                    }
                    None => add_right_column(&mut self.model.layout, pane),
                }
            }
            OpenPaneRequest::Project => {
                // Read again on the way in: the file is one a person may also have edited by
                // hand, and the boxes are seeded from what comes back.
                self.model.project_editor = None;
                self.model.project_pending = true;
                self.model.project_focus = true;
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Project)
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Project, active_frame);
                self.model.layout.add_pane(frame, Pane::Project, None);
            }
            OpenPaneRequest::Submodules => {
                self.model.submodule_filter_focus = true;
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Submodules)
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Submodules, active_frame);
                self.model.layout.add_pane(frame, Pane::Submodules, None);
            }
            OpenPaneRequest::Tasks => {
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Tasks)
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Tasks, active_frame);
                self.model.layout.add_pane(frame, Pane::Tasks, None);
                self.model.board.refresh_requested = true;
            }
        }
    }

    /// Where a pane of this kind goes: with the others of its kind, else the frame whose tab
    /// strip is the app header.
    fn frame_for(&self, kind: PaneKind, preferred: FrameId) -> FrameId {
        self.model
            .layout
            .frame_holding(preferred, |pane| pane.kind() == kind)
            .unwrap_or_else(|| self.model.layout.primary_frame())
    }

    /// Start a shell on the given review's repo and open a pane attached to it.
    pub(crate) fn spawn_terminal(
        &mut self,
        session_id: String,
        command: Option<AgentKind>,
        placement: TerminalPlacement,
    ) {
        let started = session_id.clone();
        self.spawn_shell(session_id, command, placement, false, move |backend| {
            backend.create_terminal(&started, command)
        });
    }

    /// The same, with one of the project's commands typed into the shell and sent. The pane
    /// is an ordinary shell pane: the command is over in a moment, and what is left is a
    /// shell in the repo with its output above the prompt.
    ///
    /// `restarts_when_exited` marks the shell as the one whose end restarts the window: the
    /// build-and-run of a project whose run command is the restart word, whose typed line
    /// only exits on a build that came out well.
    pub(crate) fn run_project_command(
        &mut self,
        session_id: String,
        which: ProjectCommand,
        placement: TerminalPlacement,
        restarts_when_exited: bool,
    ) {
        let started = session_id.clone();
        self.spawn_shell(
            session_id,
            None,
            placement,
            restarts_when_exited,
            move |backend| backend.run_project_command(&started, which),
        );
    }

    /// Start a shell whichever way `start` starts it, then open a pane attached to it.
    fn spawn_shell(
        &mut self,
        session_id: String,
        command: Option<AgentKind>,
        placement: TerminalPlacement,
        restarts_when_exited: bool,
        start: impl FnOnce(&dyn crate::backend::Backend) -> anyhow::Result<String> + Send + 'static,
    ) {
        if session_id.is_empty() {
            self.model.error("no review is open yet");
            return;
        }
        self.model.last_shell_session_id = Some(session_id.clone());
        let inbox = Arc::clone(&self.attaching);

        self.tasks.spawn(
            move |backend| {
                let terminal_id = start(backend)?;
                let attachment = backend.attach_terminal(&session_id, &terminal_id);
                Ok((terminal_id, attachment))
            },
            move |model, result| match result {
                Ok((terminal_id, attachment)) => {
                    if restarts_when_exited {
                        model.restart_on_shell_exit = Some(terminal_id.clone());
                    }
                    let pane = Pane::Terminal {
                        terminal_id: terminal_id.clone(),
                        command,
                        task_id: None,
                    };
                    place_shell(&mut model.layout, &placement, pane);
                    if let Ok(mut inbox) = inbox.lock() {
                        inbox.push(AttachedTerminal {
                            terminal_id,
                            attachment,
                            held_by: TerminalHolder::Workspace,
                        });
                    }
                }
                Err(error) => model.error(format!("could not start a shell: {error}")),
            },
        );
    }

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

    /// Adopt shells the server is already running that this window has no tab for.
    ///
    /// A remote server outlives any one window, so a shell started by another window on the
    /// same server is still a shell this one can show.
    pub(crate) fn adopt_existing_shells(&mut self) {
        let session_id = self.model.root_session_id.clone();
        if session_id.is_empty() {
            return;
        }

        self.tasks.spawn_keyed(
            Some("adopt-shells".to_string()),
            move |backend| backend.list_terminals(&session_id),
            |model, result| {
                let known: std::collections::HashSet<String> = model
                    .layout
                    .panes()
                    .filter_map(|(_, pane)| match pane {
                        Pane::Terminal { terminal_id, .. } => Some(terminal_id.clone()),
                        _ => None,
                    })
                    .collect();

                for terminal_id in result.unwrap_or_default() {
                    if known.contains(&terminal_id) {
                        continue;
                    }
                    place_shell(
                        &mut model.layout,
                        &TerminalPlacement::WithOtherShells,
                        Pane::Terminal {
                            terminal_id,
                            command: None,
                            task_id: None,
                        },
                    );
                }
            },
        );
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
    pub(crate) fn close_tabs_of_exited_shells(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn close_pane(&mut self, pane_id: PaneId) {
        self.model.file_editors.remove(&pane_id);
        let closed = self.model.layout.close_pane(pane_id);

        // A task's pane takes its boxes with it, writing whatever was typed into the notes and
        // not yet written: the tab closing is the last chance those words get.
        if let Some(Pane::Start { task_id, .. }) = &closed
            && let Some(editor) = self.model.board.task_editors.remove(task_id)
            && editor.notes_typed_at.is_some()
        {
            crate::native::board::actions::apply(
                self,
                crate::native::board::actions::BoardAction::SaveNotes {
                    task_id: task_id.clone(),
                    notes: editor.notes,
                },
            );
        }

        // A new-task pane takes its writing with it and makes nothing: `[create]` is what makes
        // a task, and closing the tab without pressing it is saying no to the task. The empty
        // card it was standing for goes off the board with it.
        if let Some(Pane::NewTask { draft_id, .. }) = &closed {
            self.model.board.drafts.remove(draft_id);
            self.model.board.card_being_written = None;
        }

        // Closing a shell's tab ends the shell: the tab is the only window it had.
        //
        // A task's shell is the exception. It belongs to the task rather than to the tab, and
        // keeps running with nothing attached until the task reaches DONE, so the user can come
        // back to the agent they closed.
        if let Some(Pane::Terminal {
            terminal_id,
            task_id,
            ..
        }) = closed
        {
            // Closing the build shell by hand is calling its restart off.
            if self.model.restart_on_shell_exit.as_deref() == Some(terminal_id.as_str()) {
                self.model.restart_on_shell_exit = None;
            }
            self.terminals.remove(&terminal_id);
            self.model.terminal_names.remove(&terminal_id);
            self.terminal_errors.remove(&terminal_id);
            if self
                .model
                .renaming_tab
                .as_ref()
                .is_some_and(|rename| rename.pane_id == pane_id)
            {
                self.model.renaming_tab = None;
            }
            if task_id.is_some() {
                return;
            }
            let session_id = self.model.root_session_id.clone();
            self.tasks.spawn(
                move |backend| backend.close_terminal(&session_id, &terminal_id),
                |model, result| model.report(result, "could not close the shell"),
            );
        }
    }

    /// ⌘T and the tab strip's + button both open a shell wherever the workspace has room.
    pub(crate) fn open_shell_tab(&mut self) {
        let frame = self.model.layout.active_frame();
        self.open_shell_beside(frame);
    }

    /// Split the frame the keyboard is in, and start a shell in the half that opens. A frame
    /// with nothing in it is dropped on the next frame drawn, so a split arrives with its
    /// shell rather than empty.
    pub(crate) fn split_frame(&mut self, side: DropSide) {
        let frame = self.model.layout.active_frame();
        let session_id = self.shell_session_for(frame);
        self.spawn_terminal(session_id, None, TerminalPlacement::Beside { frame, side });
    }

    /// The same, for a shell asked for from a particular frame's tab strip.
    pub(crate) fn open_shell_beside(&mut self, frame: FrameId) {
        let placement = self.room_for_a_column(frame);
        let session_id = self.shell_session_for(frame);
        self.spawn_terminal(session_id, None, placement);
    }

    /// The review a shell asked for from this frame starts in: the review the pane in front of
    /// the frame belongs to - a review, a file of it, or its commit pane - else wherever the
    /// last shell was started, else the review the window was launched on.
    pub(crate) fn shell_session_for(&self, frame: FrameId) -> String {
        let showing = self
            .model
            .layout
            .frame(frame)
            .and_then(egui_frames::Frame::active_pane)
            .and_then(|pane| self.model.layout.pane(pane));
        if let Some(
            Pane::Review { session_id, .. }
            | Pane::File { session_id, .. }
            | Pane::Commit { session_id },
        ) = showing
        {
            return session_id.clone();
        }
        self.model
            .last_shell_session_id
            .clone()
            .unwrap_or_else(|| self.model.root_session_id.clone())
    }

    /// A new shell goes in its own column down the right of the workspace, unless that would
    /// squeeze that column - or whatever it takes the room from - below a usable width, in
    /// which case it becomes another tab in the frame it was asked for.
    pub(crate) fn room_for_a_column(&self, frame: FrameId) -> TerminalPlacement {
        // A shell takes a column of its own only in a workspace that is still one frame wide.
        // Once the user has split it, whatever they arranged is theirs, and a new shell joins
        // the tabs of the frame it was asked from.
        let split_already = self.model.layout.frame_count() > 1;
        let width = self.frames.frame_rect(frame).map(|rect| rect.width());

        match width {
            Some(width) if !split_already && fits_another_column(width) => {
                TerminalPlacement::RightColumn
            }
            _ => TerminalPlacement::Tab(frame),
        }
    }

    /// The pane in front of the active frame, which is what the keyboard is talking to.
    pub(crate) fn active_pane(&self) -> Option<(PaneId, &Pane)> {
        self.model.layout.active_pane()
    }

    pub(crate) fn active_pane_kind(&self) -> Option<PaneKind> {
        self.active_pane().map(|(_, pane)| pane.kind())
    }

    pub(crate) fn active_pane_id(&self) -> Option<PaneId> {
        self.active_pane().map(|(pane_id, _)| pane_id)
    }

    /// Where a pane was drawn this frame: the body of the frame holding it, below the tabs.
    /// Anything that floats over a pane - the find bar - is placed against this.
    pub(crate) fn pane_rect(&self, pane_id: PaneId) -> Option<egui::Rect> {
        self.frames.pane_rect(pane_id)
    }

    /// The column down the right to open a pane into, given what that pane is opened to stand
    /// beside - the board for a task's tabs, the review for the pane committing it.
    ///
    /// That is the frame at the right of the workspace, whatever is already in it: a window
    /// that took a column of its own for every tab opened off the one on the left would be a
    /// new column a minute, each of them narrower than the last.
    ///
    /// `None` when the frame at the right is the one holding what the pane stands beside - a
    /// workspace that has not been split yet. A tab landing on top of what it was opened to sit
    /// next to would put that out of sight, so a column is made for it instead.
    fn column_beside(&self, stands_beside: impl Fn(&Pane) -> bool) -> Option<FrameId> {
        let frame = frame_at_the_right(&self.model.layout)?;
        let holds_it = self
            .model
            .layout
            .frame(frame)?
            .panes()
            .iter()
            .filter_map(|pane| self.model.layout.pane(*pane))
            .any(stands_beside);
        (!holds_it).then_some(frame)
    }

    /// The column a task's tabs share: its start window, the shells started from it, the files
    /// opened off its card. They stand beside the board.
    pub(crate) fn task_column(&self) -> Option<FrameId> {
        self.column_beside(|pane| pane.kind() == PaneKind::Tasks)
    }

    /// Put a task's pane down the right of the board, in place of whatever task's pane was
    /// there.
    ///
    /// One in the window at a time: every card clicked leaving its own tab behind would fill
    /// the strip with tasks nobody is looking at any more. A new-task pane is not one of them:
    /// what is on it is being written rather than read, and only its own `[create]` or its own
    /// close mark puts it away.
    ///
    /// It goes into whatever is already down the right of the board, and only into a column of
    /// its own when there is nothing there yet: a window that split itself again for every card
    /// clicked would be a new column a minute. First among that frame's tabs rather than last:
    /// it is opened to be read now and closed in a moment, and a tab that lands at the end of a
    /// long strip is one you have to go looking for.
    fn open_beside_the_board(&mut self, pane: Pane) {
        let others: Vec<PaneId> = self
            .model
            .layout
            .panes()
            .filter(|(_, open)| matches!(open, Pane::Start { .. }))
            .map(|(pane, _)| pane)
            .collect();
        for other in others {
            self.close_pane(other);
        }
        match self.task_column() {
            Some(frame) => {
                let first = self
                    .model
                    .layout
                    .frame(frame)
                    .and_then(|frame| frame.panes().first().copied());
                self.model.layout.add_pane(frame, pane, first);
            }
            None => add_right_column(&mut self.model.layout, pane),
        }
    }

    /// The start window open on this task, if one is.
    fn start_pane_of(&self, task_id: &str) -> Option<PaneId> {
        self.model
            .layout
            .find_pane(|pane| matches!(pane, Pane::Start { task_id: on, .. } if on == task_id))
            .map(|(pane, _)| pane)
    }

    /// Let the board's mark follow the tab in front, before the frame is drawn.
    ///
    /// A task's own tab coming forward - its page, a shell started in it, a file opened off its
    /// card - is that task being worked in, and the board marks its card for it: the same mark
    /// a click on the card makes, because it means the same thing.
    ///
    /// Only when the tab in front changes, so the marks made on the board afterwards are not
    /// undone frame by frame by the tab that opened the first of them.
    ///
    /// The task is read off the pane here rather than by the board, because the arrangement is
    /// lent out to the workspace widget for the length of the draw - see [`Self::draw_workspace`]
    /// - and the board could not ask it anything while it is out.
    fn follow_task_in_front(&mut self) {
        let front = self.active_pane_id();
        if front == self.front_pane {
            return;
        }
        self.front_pane = front;

        // Something that is nobody's task came forward - the board itself, a review. The marks
        // are left alone: they are read on the board, and clicking onto the board to look at
        // them would otherwise be what took them off.
        let Some(task_id) = front
            .and_then(|pane| self.model.layout.pane(pane))
            .and_then(Pane::task_id)
            .map(str::to_string)
        else {
            return;
        };
        crate::native::board::selection::mark_only(&mut self.model.board, task_id);
    }

    /// The review in the frontmost pane of the active frame, if that pane is a review.
    pub(crate) fn focused_review_session(&self) -> Option<String> {
        match self.active_pane()?.1 {
            Pane::Review { session_id, .. } => Some(session_id.clone()),
            _ => None,
        }
    }

    /// The review a command aimed at "this review" means: the one the pane in front belongs to,
    /// whether that is a review, a file of it, or its own commit pane. Else the review the
    /// window was launched on.
    ///
    /// A window can have several reviews open at once: every changed submodule is a review of
    /// its own repo, with its own branch to commit and push. Committing while reading one of
    /// them is committing that repo, not the one the window was launched on.
    pub(crate) fn review_in_front(&self) -> String {
        match self.active_pane().map(|(_, pane)| pane) {
            Some(
                Pane::Review { session_id, .. }
                | Pane::File { session_id, .. }
                | Pane::Commit { session_id },
            ) => session_id.clone(),
            _ => self.model.root_session_id.clone(),
        }
    }

    /// `C-x o`: hand the keyboard to the next frame of the workspace, wrapping round at the end.
    pub(crate) fn focus_next_frame(&mut self) {
        if self.model.layout.frame_count() < 2 {
            return;
        }
        self.model.layout.focus_next_frame();
    }

    /// cmd+1 through cmd+9: bring the active frame's nth tab to the front. A digit past the
    /// last tab does nothing.
    pub(crate) fn select_tab(&mut self, index: usize) {
        let frame = self.model.layout.active_frame();
        let Some(pane) = self
            .model
            .layout
            .frame(frame)
            .and_then(|open| open.panes().get(index).copied())
        else {
            return;
        };
        self.model.layout.focus_pane(pane);
    }

    /// The chords that raise tabs reach the active frame, so its tabs and only its tabs wear
    /// them at the right of their titles. A frame with a single tab wears none: cmd+1 there
    /// would change nothing worth signposting.
    fn stamp_tab_shortcuts(&mut self) {
        self.tab_shortcuts.clear();
        let frame = self.model.layout.active_frame();
        let Some(open) = self.model.layout.frame(frame) else {
            return;
        };
        if open.panes().len() < 2 {
            return;
        }
        for (index, pane) in open.panes().iter().take(9).enumerate() {
            if let Some(label) = bindings::tab_shortcut_label(index) {
                self.tab_shortcuts.insert(*pane, label);
            }
        }
    }

    /// The tab in front of the active frame is the one being worked in, so it is the one with
    /// the keyboard - whatever put it there: a click on its tab or in its frame, cmd+1, `C-x o`,
    /// a tab dragged across, a pane opened or closed. Watching the arrangement rather than
    /// each of those means none of them can be the one that forgets.
    ///
    /// The keyboard itself moves in two steps, because egui's focus can only be taken by the
    /// widget that would hold it: the pane left behind lets go here, and the pane arriving takes
    /// it while it draws - in [`App::draw_terminal`] for a shell, and in `file_pane::draw_editor`
    /// for a file or a task's notes.
    fn follow_front_tab(&mut self, ctx: &egui::Context) {
        let front = self.active_pane_id();
        if front == self.keyboard_pane {
            return;
        }
        self.keyboard_pane = front;
        // A widget drawn in the arriving pane is not being left behind: the click that brought
        // the pane forward may be the very one that put the keyboard in that widget - the third
        // click of a triple on a card's title, say, with the rename box the first two opened
        // already holding it. The keyboard is where it was reached for, so it stays.
        if let Some(focused) = ctx.memory(|memory| memory.focused()) {
            let in_front_pane = front
                .and_then(|pane| self.frames.pane_rect(pane))
                .zip(ctx.read_response(focused))
                .is_some_and(|(pane, widget)| pane.intersects(widget.rect));
            if in_front_pane {
                return;
            }
            // A shell that keeps the keyboard keeps every key sent to the window, and a pane
            // with nothing to type into never takes it off it, so letting go is not the
            // arriving pane's job to do.
            ctx.memory_mut(|memory| memory.surrender_focus(focused));
        }
        self.pane_taking_keyboard = front;
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

/// The frame down the right of the workspace: the last of a row, and the first of a column, so
/// what is answered is the one at the top right rather than whichever frame happens to be last
/// in the arrangement.
///
/// `None` where there is nothing to the right - a workspace of one frame has no right-hand side
/// yet, only a middle.
fn frame_at_the_right(layout: &Layout<Pane>) -> Option<FrameId> {
    let mut node = layout.root();
    loop {
        match node {
            egui_frames::LayoutNode::Frame { frame } => return Some(*frame),
            egui_frames::LayoutNode::Split {
                direction,
                children,
                ..
            } => {
                node = match direction {
                    egui_frames::SplitDirection::Row => children.last()?,
                    egui_frames::SplitDirection::Column => children.first()?,
                };
            }
        }
    }
}

/// Put a pane in a column of its own down the right of the workspace.
fn add_right_column(layout: &mut Layout<Pane>, pane: Pane) {
    layout.add_pane_against_edge(DropSide::Right, egui_frames::DEFAULT_EDGE_SHARE, pane);
}

/// Put a shell's pane where the placement says, falling back to a column of its own.
fn place_shell(layout: &mut Layout<Pane>, placement: &TerminalPlacement, pane: Pane) {
    let column = add_right_column;

    match placement {
        TerminalPlacement::WithOtherShells => {
            let active = layout.active_frame();
            match layout.frame_holding(active, |pane| pane.kind() == PaneKind::Terminal) {
                Some(frame) => {
                    layout.add_pane(frame, pane, None);
                }
                None => column(layout, pane),
            }
        }
        TerminalPlacement::RightColumn => column(layout, pane),
        TerminalPlacement::Beside { frame, side } if layout.frame(*frame).is_some() => {
            layout.add_pane_beside(*frame, *side, pane);
        }
        TerminalPlacement::Beside { .. } => column(layout, pane),
        // The frame is gone if it was closed while the shell was starting.
        TerminalPlacement::Tab(frame) if layout.frame(*frame).is_some() => {
            layout.add_pane(*frame, pane, None);
        }
        TerminalPlacement::Tab(_) => column(layout, pane),
    }
}

/// Whether a frame this wide can give up the share a new right-hand column takes without
/// leaving either side too narrow to work in.
fn fits_another_column(frame_width: f32) -> bool {
    let new_column = frame_width * egui_frames::DEFAULT_EDGE_SHARE;
    let left_behind = frame_width * (1.0 - egui_frames::DEFAULT_EDGE_SHARE);
    new_column >= MIN_COLUMN_WIDTH && left_behind >= MIN_COLUMN_WIDTH
}

/// How often a window with a shell in it redraws. The terminal widget asks for its own frames
/// while its program is alive; this is the floor under everything else.
pub(crate) const SHELL_REPAINT_INTERVAL: Duration = Duration::from_millis(33);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_has_room_for_a_shell_beside_it() {
        assert!(fits_another_column(1440.0));
        assert!(fits_another_column(1000.0));
    }

    #[test]
    fn a_narrow_frame_takes_the_shell_as_a_tab() {
        // At the window's minimum width, a column would leave both sides too thin to read.
        assert!(!fits_another_column(720.0));
        assert!(!fits_another_column(900.0));
    }

    fn shell(id: &str) -> Pane {
        Pane::Terminal {
            terminal_id: id.to_string(),
            command: None,
            task_id: None,
        }
    }

    #[test]
    fn the_first_shell_takes_a_column_and_the_next_one_joins_it() {
        let mut layout = Layout::with_pane(Pane::Review {
            session_id: "session".to_string(),
            title: "review".to_string(),
        });

        place_shell(&mut layout, &TerminalPlacement::WithOtherShells, shell("a"));
        assert_eq!(layout.frame_count(), 2, "the first shell takes a column");

        place_shell(&mut layout, &TerminalPlacement::WithOtherShells, shell("b"));
        assert_eq!(layout.frame_count(), 2, "the second joins its tabs");
        assert_eq!(layout.pane_count(), 3);
        assert!(layout.is_coherent());
    }

    /// What the palette's split commands ask for: the frame in two, the shell in the new half.
    #[test]
    fn a_split_puts_the_shell_in_a_frame_of_its_own_beside_the_one_asked_for() {
        let mut layout = Layout::with_pane(Pane::Review {
            session_id: "session".to_string(),
            title: "review".to_string(),
        });
        let frame = layout.active_frame();

        place_shell(
            &mut layout,
            &TerminalPlacement::Beside {
                frame,
                side: DropSide::Bottom,
            },
            shell("a"),
        );

        assert_eq!(layout.frame_count(), 2, "the frame was split in two");
        assert_eq!(
            layout.frame(frame).map(|frame| frame.panes().len()),
            Some(1),
            "the review kept its half to itself"
        );
        assert!(layout.is_coherent());
    }

    /// A shell takes a moment to start, and the frame it was asked from may be gone by then.
    #[test]
    fn a_shell_asked_for_from_a_frame_that_has_since_closed_still_opens() {
        let mut layout = Layout::with_pane(Pane::Agents);
        let doomed = layout.add_pane_beside(layout.active_frame(), DropSide::Right, shell("gone"));
        let gone = layout.frame_of(doomed).expect("expected a frame");
        layout.close_pane(doomed);
        assert!(
            layout.frame(gone).is_none(),
            "the frame went with its shell"
        );

        place_shell(&mut layout, &TerminalPlacement::Tab(gone), shell("a"));
        place_shell(
            &mut layout,
            &TerminalPlacement::Beside {
                frame: gone,
                side: DropSide::Right,
            },
            shell("b"),
        );

        assert_eq!(layout.pane_count(), 3, "both shells landed somewhere");
        assert!(layout.is_coherent());
    }
}
