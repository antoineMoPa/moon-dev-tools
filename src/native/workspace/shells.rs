//! Starting shells: on a review's repo, on another repo, with one of the project's commands
//! typed in, and the ones the server already runs that this window has no tab for yet.

use std::sync::Arc;

use egui_frames::{DropSide, FrameId};

use crate::{
    api::{AgentKind, OpenSessionRequest},
    native::{
        app::{App, AttachedTerminal, TerminalHolder},
        panes::Pane,
    },
    project::ProjectCommand,
};

use super::{ShellMarks, TerminalPlacement, place_shell};

impl App {
    /// Start a shell on the given review's repo and open a pane attached to it.
    pub(crate) fn spawn_terminal(
        &mut self,
        session_id: String,
        command: Option<AgentKind>,
        placement: TerminalPlacement,
    ) {
        let started = session_id.clone();
        self.spawn_shell(
            session_id,
            command,
            placement,
            ShellMarks::default(),
            move |backend| backend.create_terminal(&started, command),
        );
    }

    /// The same, with one of the project's commands typed into the shell and sent. The pane
    /// is an ordinary shell pane: the command is over in a moment, and what is left is a
    /// shell in the repo with its output above the prompt.
    ///
    /// That shell is where the project's commands go from then on - see
    /// [`Self::shell_for_project_commands`] - so a second build is typed into the tab the
    /// first one ran in rather than opening another beside it.
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
        if let Some(terminal_id) = self.shell_for_project_commands() {
            self.type_project_command(&terminal_id, which, restarts_when_exited);
            return;
        }
        let started = session_id.clone();
        self.spawn_shell(
            session_id,
            None,
            placement,
            ShellMarks {
                takes_project_commands: true,
                restarts_window: restarts_when_exited,
            },
            move |backend| backend.run_project_command(&started, which),
        );
    }

    /// The shell a project's command goes back into: the one the last command ran in, still
    /// open and waiting at its prompt.
    ///
    /// A shell with something running in it is not one to type a build into - the line would
    /// sit in its input until whatever is running there is done - so that one is left alone
    /// and the command opens a shell of its own. Which shells those are is the server's
    /// answer, polled - see `App::poll_running_shells`.
    fn shell_for_project_commands(&self) -> Option<String> {
        let terminal_id = self.model.project_shell.clone()?;
        let waiting = self
            .terminals
            .get(&terminal_id)
            .is_some_and(|terminal| !terminal.has_exited())
            && !self.model.shells_running_a_command.contains(&terminal_id);
        waiting.then_some(terminal_id)
    }

    /// Type one of the project's commands into a shell that is already open, as a person at
    /// that prompt would, and bring its tab forward to watch it run.
    ///
    /// The line is the window's own copy of the project's commands - the one the Project menu
    /// offers - rather than the file read again: the command that runs is the command that was
    /// picked.
    fn type_project_command(
        &mut self,
        terminal_id: &str,
        which: ProjectCommand,
        restarts_when_exited: bool,
    ) {
        let Some(line) = self.model.project.line(which) else {
            self.model
                .error(format!("this project has no {} command", which.label()));
            return;
        };
        let Some(terminal) = self.terminals.get(terminal_id) else {
            self.model
                .error("the shell the project's commands run in is gone".to_string());
            return;
        };
        if let Err(error) = terminal.send(format!("{line}\r").as_bytes()) {
            self.model
                .error(format!("could not run {line} in its shell: {error}"));
            return;
        }
        if restarts_when_exited {
            self.model.restart_on_shell_exit = Some(terminal_id.to_string());
        }

        if let Some((pane_id, _)) = self.model.layout.find_pane(
            |pane| matches!(pane, Pane::Terminal { terminal_id: of_pane, .. } if of_pane == terminal_id),
        ) {
            self.model.layout.focus_pane(pane_id);
        }
    }

    /// A shell on the window's repo with a command line typed into it and sent: what an
    /// extension's `open_shell` asks for. It goes where shells go, beside the others.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn run_in_shell(&mut self, command: String) {
        let session_id = self.model.root_session_id.clone();
        let started = session_id.clone();
        self.spawn_shell(
            session_id,
            None,
            TerminalPlacement::WithOtherShells,
            ShellMarks::default(),
            move |backend| backend.run_in_shell(&started, &command),
        );
    }

    /// A shell started in a repo other than the window's, beside the other shells. Opening a
    /// session on a repo that already has one answers with that session, so the shell lands in
    /// the same session a review of the repo is in.
    ///
    /// The shell is attached through the window's own session: which session names a shell
    /// only matters for where it starts, and that is settled by the time it is attached.
    pub(crate) fn open_shell_in_repo(&mut self, repo_path: String) {
        let session_id = self.model.root_session_id.clone();
        self.spawn_shell(
            session_id,
            None,
            TerminalPlacement::WithOtherShells,
            ShellMarks::default(),
            move |backend| {
                let opened = backend.open_session(OpenSessionRequest {
                    repo_path: repo_path.clone(),
                    diff_target: None,
                    active_commit: None,
                })?;
                backend.create_terminal(&opened.session_id, None)
            },
        );
    }

    /// Start a shell whichever way `start` starts it, then open a pane attached to it.
    fn spawn_shell(
        &mut self,
        session_id: String,
        command: Option<AgentKind>,
        placement: TerminalPlacement,
        marks: ShellMarks,
        start: impl Fn(&dyn crate::backend::Backend) -> anyhow::Result<String> + Send + 'static,
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
                    if marks.restarts_window {
                        model.restart_on_shell_exit = Some(terminal_id.clone());
                    }
                    if marks.takes_project_commands {
                        model.project_shell = Some(terminal_id.clone());
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

    /// A shell at the root of the repo a review is on, beside that review: what clicking the
    /// repo's name over the review opens. It joins the tabs of the frame at the right, the way
    /// a task's tabs join the column beside the board, and takes a column of its own only when
    /// that frame is the review's own - a tab there would hide the review it was opened from.
    ///
    /// Asked for through [`OpenPaneRequest::TerminalBesideReview`], never from inside a pane:
    /// while the panes are being drawn the layout is lent out, and what is left in its place
    /// has no frame at the right to find.
    pub(super) fn open_shell_beside_review(&mut self, session_id: String) {
        let placement = match self.column_beside(
            |pane| matches!(pane, Pane::Review { session_id: of_pane, .. } if *of_pane == session_id),
        ) {
            Some(frame) => TerminalPlacement::Tab(frame),
            None => TerminalPlacement::RightColumn,
        };
        self.spawn_terminal(session_id, None, placement);
    }
}
