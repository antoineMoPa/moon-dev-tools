//! What the board does once a card is clicked, dragged or typed into.

use crate::{
    api::AgentKind,
    moontasks::{ColumnEnd, ColumnId, CreateTaskRequest, StartResourceRequest},
    native::{
        app::App,
        model::OpenedShell,
        palette::CommandAction,
        panes::OpenPaneRequest,
    },
};

/// What a click on the board asked for. Collected while drawing and acted on afterwards, so
/// nothing changes the pane tree or the task list while either is being read.
pub(crate) enum BoardAction {
    /// Open the new-task box at one end of this column - the end whose `+` was pressed.
    OpenComposer(ColumnId, ColumnEnd),
    CloseComposer,
    /// Create the typed task at the end of this column the composer is standing at, and start
    /// the picked agent on it.
    Create(ColumnId, ColumnEnd, AgentKind),
    /// A card let go of in a column, at the place among its cards it was dropped.
    Place(String, ColumnId, usize),
    /// A column let go of on the board, at the place among the others it was dropped.
    PlaceColumn(ColumnId, usize),
    AddColumn(String),
    RenameColumn(ColumnId, String),
    CancelColumnRename,
    DeleteColumn(ColumnId),
    CloseColumnComposer,
    Delete(String),
    Rename(String, String),
    CancelRename,
    /// Open the task's `notes.md` in a pane down the right, making the file first if the
    /// task has none yet.
    OpenNotes(String),
    /// Turn the palette into the file finder, picking a file to put on this task's card.
    PickFile(String),
    /// Open a file linked to a card, in a pane down the right.
    OpenFile(String),
    Start(String, StartResourceRequest),
    Resume(String, String),
    /// Open the modal that lists the agents' own sessions, for this task.
    OpenAttachPicker {
        task_id: String,
        task_title: String,
    },
    CloseAttachPicker,
    /// Put the picked session on the task and open a shell resumed on it.
    Attach {
        task_id: String,
        agent: AgentKind,
        agent_session_id: String,
    },
    Stop(String, String),
    /// Take a run off the task for good, rather than leaving it to be resumed.
    DeleteResource(String, String),
    ArmResourceDelete(String),
    CancelResourceDelete,
    /// Bring a task's shell on screen, in a tab of its own.
    OpenShell {
        terminal_id: String,
        command: Option<AgentKind>,
        task_id: String,
    },
    /// Open the review of what the task has changed.
    OpenReview(String, String),
}

pub(super) fn apply(app: &mut App, action: BoardAction) {
    let session_id = app.model.root_session_id.clone();

    match action {
        BoardAction::OpenComposer(column_id, joins) => {
            app.model.board.composer_in = Some(column_id);
            app.model.board.composer_at = joins;
            app.model.board.composer_focus = true;
            // Each column's box starts from that column's own remembered agent.
            app.model.board.composer_agent = None;
        }
        BoardAction::CloseComposer => {
            app.model.board.composer_in = None;
            app.model.board.new_title.clear();
            app.model.board.composer_agent = None;
        }
        BoardAction::Create(column_id, joins, agent) => {
            let request = CreateTaskRequest {
                title: app.model.board.new_title.trim().to_string(),
                agent,
                status: column_id,
                joins,
            };
            if request.title.is_empty() {
                return;
            }
            // The box closes on the way out: the card it was standing in for is on its way.
            app.model.board.new_title.clear();
            app.model.board.composer_in = None;
            app.model.board.composer_agent = None;
            // And the filter goes with it, so the new card is on the board rather than behind
            // a query that was asked before it existed and says nothing about it.
            app.model.board.filter.clear();
            act(app, "could not create the task", move |backend| {
                backend.create_task(&session_id, &request).map(|_| ())
            });
        }
        BoardAction::Place(task_id, status, position) => {
            app.tasks.spawn(
                move |backend| backend.place_task(&session_id, &task_id, status, position),
                |model, result| {
                    // A move the server would not make is not one to keep drawing.
                    if result.is_err() {
                        model.board.pending_place = None;
                    }
                    model.report(result, "could not move the task");
                    model.board.refresh_requested = true;
                },
            );
        }
        BoardAction::Delete(task_id) => {
            app.model.board.pending_delete = None;
            act(app, "could not delete the task", move |backend| {
                backend.delete_task(&session_id, &task_id)
            });
        }
        BoardAction::Start(task_id, request) => {
            // The shell it starts in is what the user wants to look at, so it opens with it.
            let for_pane = task_id.clone();
            let command = (request.agent != AgentKind::None).then_some(request.agent);
            app.tasks.spawn(
                move |backend| backend.start_task_resource(&session_id, &task_id, request),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command,
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not start it: {error}")),
                    }
                },
            );
        }
        BoardAction::Resume(task_id, resource_id) => {
            let for_pane = task_id.clone();
            let command = app
                .model
                .board
                .tasks
                .iter()
                .find(|task| task.id == task_id)
                .and_then(|task| {
                    task.resources
                        .iter()
                        .find(|resource| resource.id == resource_id)
                })
                .map(|resource| resource.agent);
            app.tasks.spawn(
                move |backend| backend.resume_task_resource(&session_id, &task_id, &resource_id),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command,
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not resume it: {error}")),
                    }
                },
            );
        }
        BoardAction::OpenAttachPicker {
            task_id,
            task_title,
        } => {
            app.model.board.attach_picker = Some(crate::native::model::AttachPicker {
                task_id,
                task_title,
                sessions: None,
                error: None,
                manual_id: String::new(),
                manual_agent: None,
            });
            // Keyed, so holding the menu item down cannot queue a listing per frame. The
            // listing is the repo's rather than the task's, so whichever picker is open when
            // it lands is the one it answers - including one reopened on another card while
            // an earlier read was still on its way, whose own read the key swallowed.
            app.tasks.spawn_keyed(
                Some("agent-sessions".to_string()),
                move |backend| backend.list_agent_sessions(&session_id),
                move |model, result| {
                    let Some(picker) = model.board.attach_picker.as_mut() else {
                        return;
                    };
                    match result {
                        Ok(sessions) => picker.sessions = Some(sessions),
                        Err(error) => picker.error = Some(error.to_string()),
                    }
                },
            );
        }
        BoardAction::CloseAttachPicker => app.model.board.attach_picker = None,
        BoardAction::Attach {
            task_id,
            agent,
            agent_session_id,
        } => {
            app.model.board.attach_picker = None;
            let for_pane = task_id.clone();
            let request = crate::moontasks::AttachResourceRequest {
                agent,
                agent_session_id,
            };
            app.tasks.spawn(
                move |backend| backend.attach_task_resource(&session_id, &task_id, &request),
                move |model, result| {
                    model.board.refresh_requested = true;
                    match result {
                        Ok(terminal_id) => {
                            model.board.opened_shell = Some(OpenedShell {
                                terminal_id,
                                command: Some(agent),
                                task_id: for_pane,
                            })
                        }
                        Err(error) => model.error(format!("could not attach it: {error}")),
                    }
                },
            );
        }
        BoardAction::Stop(task_id, resource_id) => {
            act(app, "could not stop it", move |backend| {
                backend.stop_task_resource(&session_id, &task_id, &resource_id)
            });
        }
        BoardAction::ArmResourceDelete(resource_id) => {
            app.model.board.pending_resource_delete = Some(resource_id);
        }
        BoardAction::CancelResourceDelete => app.model.board.pending_resource_delete = None,
        BoardAction::DeleteResource(task_id, resource_id) => {
            app.model.board.pending_resource_delete = None;
            act(app, "could not remove it", move |backend| {
                backend.delete_task_resource(&session_id, &task_id, &resource_id)
            });
        }
        BoardAction::Rename(task_id, title) => {
            app.model.board.renaming = None;
            act(app, "could not rename the task", move |backend| {
                backend.rename_task(&session_id, &task_id, &title)
            });
        }
        BoardAction::CancelRename => app.model.board.renaming = None,
        BoardAction::OpenNotes(task_id) => {
            app.tasks.spawn(
                move |backend| backend.open_task_notes(&session_id, &task_id),
                |model, result| match result {
                    Ok(file_path) => model.board.opened_file = Some(file_path),
                    Err(error) => model.error(format!("could not open the notes: {error}")),
                },
            );
        }
        BoardAction::PickFile(task_id) => app.model.palette.show_files_for_task(task_id),
        BoardAction::OpenFile(file_path) => app.model.board.opened_file = Some(file_path),
        BoardAction::AddColumn(label) => {
            // The box closes on the way out: the column it was standing in for is on its way.
            app.model.board.new_column_label.clear();
            app.model.board.column_composer_open = false;
            act(app, "could not add the column", move |backend| {
                backend.add_column(&session_id, &label).map(|_| ())
            });
        }
        BoardAction::CloseColumnComposer => {
            app.model.board.column_composer_open = false;
            app.model.board.new_column_label.clear();
        }
        BoardAction::RenameColumn(column_id, label) => {
            app.model.board.renaming_column = None;
            act(app, "could not rename the column", move |backend| {
                backend.rename_column(&session_id, &column_id, &label)
            });
        }
        BoardAction::CancelColumnRename => app.model.board.renaming_column = None,
        BoardAction::DeleteColumn(column_id) => {
            app.model.board.pending_column_delete = None;
            act(app, "could not remove the column", move |backend| {
                backend.delete_column(&session_id, &column_id)
            });
        }
        BoardAction::PlaceColumn(column_id, position) => {
            app.tasks.spawn(
                move |backend| backend.place_column(&session_id, &column_id, position),
                |model, result| {
                    // A move the server would not make is not one to keep drawing.
                    if result.is_err() {
                        model.board.pending_column_place = None;
                    }
                    model.report(result, "could not move the column");
                    model.board.refresh_requested = true;
                },
            );
        }
        BoardAction::OpenShell {
            terminal_id,
            command,
            task_id,
        } => {
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::AttachTerminal {
                terminal_id,
                command,
                task_id: Some(task_id),
            }));
        }
        BoardAction::OpenReview(repo_path, title) => {
            app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::ReviewRepo {
                repo_path,
                title,
            }));
        }
    }
}

/// Run a board action, and read the board again once it is done.
fn act<W>(app: &App, context: &'static str, work: W)
where
    W: FnOnce(&dyn crate::backend::Backend) -> anyhow::Result<()> + Send + 'static,
{
    app.tasks.spawn(work, move |model, result| {
        model.report(result, context);
        model.board.refresh_requested = true;
    });
}
