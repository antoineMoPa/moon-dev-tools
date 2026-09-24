//! Opening a pane: where each kind of pane lands, and the one already open that is brought
//! forward instead of a second copy.

use egui_frames::{FrameId, PaneId};

use crate::{
    api::OpenSessionRequest,
    native::{
        app::App,
        model::PendingCard,
        panes::{OpenPaneRequest, Pane, PaneKind},
    },
};

use super::{TerminalPlacement, add_right_column, place_shell};

impl App {
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
                            repo_path: repo_path.clone(),
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
                let pane_id = self.file_pane_for(&session_id, &file_path, active_frame);
                if let Some(at) = at {
                    self.reveal_file_match(pane_id, &session_id, &file_path, at);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            OpenPaneRequest::NewFile {
                session_id,
                file_path,
            } => {
                let pane_id = self.file_pane_for(&session_id, &file_path, active_frame);
                self.begin_new_file(pane_id, &file_path);
            }
            OpenPaneRequest::FileAt {
                session_id,
                file_path,
                revision,
                line,
            } => {
                // The same version of the same file twice is the same tab; another version
                // of it is another tab, beside the file as it is.
                let pane_id = match self.model.layout.find_pane(|pane| {
                    matches!(pane, Pane::File { file_path: open, revision: Some(at), .. }
                        if *open == file_path && *at == revision)
                }) {
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
                                task_id: None,
                                revision: Some(revision.clone()),
                            },
                            None,
                        )
                    }
                };
                self.reveal_file_line(pane_id, &session_id, &file_path, &revision, line);
            }
            OpenPaneRequest::Terminal { command } => {
                let session_id = self.shell_session_for(active_frame);
                self.spawn_terminal(session_id, command, TerminalPlacement::WithOtherShells);
            }
            OpenPaneRequest::TerminalInRepo { repo_path } => self.open_shell_in_repo(repo_path),
            OpenPaneRequest::TerminalBesideReview { session_id } => {
                self.open_shell_beside_review(session_id)
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
                    if let Pane::NewTask {
                        column,
                        joins,
                        draft_id: written_on,
                    } = open
                    {
                        self.model.board.card_being_written = Some(PendingCard {
                            column: column.clone(),
                            joins: *joins,
                        });
                        // Back in the title box, which is what was being asked for: the pane
                        // was asked for again because the naming is not finished. The draft is
                        // the open pane's own - the one made for this request is thrown away
                        // below, and the keyboard must not be promised to it.
                        self.model.board.task_box_focus = Some((
                            written_on.clone(),
                            crate::native::board::actions::TaskPaneBox::Title,
                        ));
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
            OpenPaneRequest::Messages => {
                // One log a window: asking again brings it forward rather than opening a
                // second copy of the same list.
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Messages)
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Messages, active_frame);
                self.model.layout.add_pane(frame, Pane::Messages, None);
            }
            #[cfg(not(target_arch = "wasm32"))]
            OpenPaneRequest::Extension { name } => {
                // One pane an extension: asking again brings it forward.
                if let Some((pane, _)) = self
                    .model
                    .layout
                    .find_pane(|pane| pane.runs_extension(&name))
                {
                    self.model.layout.focus_pane(pane);
                    return;
                }
                let frame = self.frame_for(PaneKind::Extension, active_frame);
                self.model
                    .layout
                    .add_pane(frame, Pane::Extension { name }, None);
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

    /// The tab on a file: brought forward when one is open - the same file twice is the same
    /// tab - and added beside the other file tabs when not.
    fn file_pane_for(
        &mut self,
        session_id: &str,
        file_path: &str,
        active_frame: FrameId,
    ) -> egui_frames::PaneId {
        // Not a tab on an old version of it: that is another thing, and opening the file by
        // name is opening the file as it is.
        match self.model.layout.find_pane(|pane| {
            matches!(pane, Pane::File { file_path: open, revision: None, .. }
                if open.as_str() == file_path)
        }) {
            Some((pane, _)) => {
                self.model.layout.focus_pane(pane);
                pane
            }
            None => {
                let frame = self.frame_for(PaneKind::File, active_frame);
                self.model.layout.add_pane(
                    frame,
                    Pane::File {
                        session_id: session_id.to_string(),
                        file_path: file_path.to_string(),
                        // Opened by name or from a search, which is the repo's file rather
                        // than any one task's.
                        task_id: None,
                        revision: None,
                    },
                    None,
                )
            }
        }
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
}
