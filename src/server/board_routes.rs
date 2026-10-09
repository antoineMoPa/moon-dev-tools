//! The routes the task board is read and changed through: its tasks, columns and the shells
//! and agents running on them, the project's commands, and the settings every window shares.
//!
//! The board and the project file are the project's, so each of their routes does its work
//! through [`session_work`], as the person the session belongs to. The settings are the
//! server's own file, read and changed as the server.

use anyhow::Result;
use axum::{
    Json,
    extract::{Path as AxumPath, State},
};

use crate::{
    api::{AppError, AppState},
    moontasks::{
        self, AttachResourceRequest, BoardTaskView, ColumnLabelRequest, ColumnPlacementRequest,
        CreateTaskRequest, LinkFileRequest, NewColumnRequest, ReviewRequestView,
        StartResourceRequest, TaskNotesPayload, TaskPlacementRequest, TaskRemoteTrackerRequest,
        TaskTagsRequest, TaskTitleRequest, TaskView, TerminalOpened, WirePayload, WorkLogPayload,
        review_request::Amend,
        store::{BoardColumn, ColumnId},
    },
};

use super::{mark_activity, session_work};

pub(super) async fn list_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskView>>, AppError> {
    mark_activity(&state);
    let tasks = session_work(state, session_id, moontasks::service::list_tasks).await?;
    Ok(Json(tasks))
}

pub(super) async fn board_task(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<BoardTaskView>, AppError> {
    mark_activity(&state);
    let task = session_work(state, session_id, moontasks::service::board_task).await?;
    Ok(Json(task))
}

pub(super) async fn create_task(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<Json<TaskView>, AppError> {
    mark_activity(&state);
    let task = session_work(state, session_id, move |state, session_id| {
        moontasks::service::create_task(state, session_id, &request)
    })
    .await?;
    Ok(Json(task))
}

pub(super) async fn delete_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::delete_task(state, session_id, &task_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn list_review_requests(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ReviewRequestView>>, AppError> {
    mark_activity(&state);
    let requests =
        session_work(state, session_id, moontasks::service::list_review_requests).await?;
    Ok(Json(requests))
}

pub(super) async fn amend_review_request(
    AxumPath((session_id, task_id, index)): AxumPath<(String, String, usize)>,
    State(state): State<AppState>,
    Json(amend): Json<Amend>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::amend_review_request(state, session_id, &task_id, index, amend)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn settings(State(state): State<AppState>) -> Json<crate::settings::Settings> {
    mark_activity(&state);
    Json(crate::settings::served(&state))
}

pub(super) async fn change_settings(
    State(state): State<AppState>,
    Json(change): Json<crate::settings::SettingsChange>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::settings::change(&state, change)?;
    Ok("ok")
}

pub(super) async fn project_commands(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<crate::project::ProjectConfig>, AppError> {
    mark_activity(&state);
    let commands = session_work(state, session_id, crate::project::session_commands).await?;
    Ok(Json(commands))
}

pub(super) async fn set_project_config(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::project::ProjectConfig>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        crate::project::set_session_commands(state, session_id, &request)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn run_project_command(
    AxumPath((session_id, which)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let which: crate::project::ProjectCommand = which.parse()?;
    let terminal_id = session_work(state, session_id, move |state, session_id| {
        crate::project::run(state, session_id, which)
    })
    .await?;
    Ok(Json(TerminalOpened { terminal_id }))
}

pub(super) async fn list_columns(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<BoardColumn>>, AppError> {
    mark_activity(&state);
    let columns = session_work(state, session_id, moontasks::service::list_columns).await?;
    Ok(Json(columns))
}

pub(super) async fn add_column(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<NewColumnRequest>,
) -> Result<Json<BoardColumn>, AppError> {
    mark_activity(&state);
    let column = session_work(state, session_id, move |state, session_id| {
        moontasks::service::add_column(state, session_id, &request.label, request.at)
    })
    .await?;
    Ok(Json(column))
}

pub(super) async fn rename_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnLabelRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::rename_column(
            state,
            session_id,
            &ColumnId::new(column_id),
            &request.label,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn set_column_arrivals(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnArrivalsRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::set_column_arrivals(
            state,
            session_id,
            &ColumnId::new(column_id),
            request.arrivals,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn set_column_sort(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnSortRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::set_column_sort(
            state,
            session_id,
            &ColumnId::new(column_id),
            request.sort,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn set_column_marks_a_days_work(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnDaysWorkRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::set_column_marks_a_days_work(
            state,
            session_id,
            &ColumnId::new(column_id),
            request.marks_a_days_work,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn delete_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::delete_column(state, session_id, &ColumnId::new(column_id))
    })
    .await?;
    Ok("ok")
}

pub(super) async fn place_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::place_column(
            state,
            session_id,
            &ColumnId::new(column_id),
            request.position,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn place_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<TaskPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::place_tasks(
            state,
            session_id,
            &request.task_ids,
            request.status,
            request.position,
        )
    })
    .await?;
    Ok("ok")
}

pub(super) async fn start_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<StartResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let terminal_id = session_work(state, session_id, move |state, session_id| {
        moontasks::service::start_resource(state, session_id, &task_id, request)
    })
    .await?;
    Ok(Json(TerminalOpened { terminal_id }))
}

pub(super) async fn list_agent_sessions(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::agent_sessions::AgentSessionView>>, AppError> {
    mark_activity(&state);
    // The sessions are the ones the person's own agents keep, in the person's own home.
    let sessions = session_work(state, session_id, crate::agent_sessions::list_for_session).await?;
    Ok(Json(sessions))
}

pub(super) async fn attach_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<AttachResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let terminal_id = session_work(state, session_id, move |state, session_id| {
        moontasks::service::attach_resource(state, session_id, &task_id, &request)
    })
    .await?;
    Ok(Json(TerminalOpened { terminal_id }))
}

pub(super) async fn resume_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let terminal_id = session_work(state, session_id, move |state, session_id| {
        moontasks::service::resume_resource(state, session_id, &task_id, &resource_id)
    })
    .await?;
    Ok(Json(TerminalOpened { terminal_id }))
}

pub(super) async fn stop_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::stop_resource(state, session_id, &task_id, &resource_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn delete_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::delete_resource(state, session_id, &task_id, &resource_id)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn rename_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskTitleRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::rename_task(state, session_id, &task_id, &request.title)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn set_task_tags(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskTagsRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::set_tags(state, session_id, &task_id, &request.tags)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn set_task_remote_tracker_url(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskRemoteTrackerRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::set_remote_tracker_url(state, session_id, &task_id, &request.url)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn open_task_notes(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TaskNotesPayload>, AppError> {
    mark_activity(&state);
    let file_path = session_work(state, session_id, move |state, session_id| {
        moontasks::service::open_notes(state, session_id, &task_id)
    })
    .await?;
    Ok(Json(TaskNotesPayload { file_path }))
}

pub(super) async fn open_work_log(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<WorkLogPayload>, AppError> {
    mark_activity(&state);
    let file_path = session_work(state, session_id, moontasks::service::open_work_log).await?;
    Ok(Json(WorkLogPayload { file_path }))
}

pub(super) async fn open_wire(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<WirePayload>, AppError> {
    mark_activity(&state);
    let file_path = session_work(state, session_id, moontasks::service::open_wire).await?;
    Ok(Json(WirePayload { file_path }))
}

pub(super) async fn link_task_file(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<LinkFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::link_file(state, session_id, &task_id, &request.file_path)
    })
    .await?;
    Ok("ok")
}

pub(super) async fn remove_task_attachment(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::RemoveAttachmentRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    session_work(state, session_id, move |state, session_id| {
        moontasks::service::remove_attachment(state, session_id, &task_id, &request.listed)
    })
    .await?;
    Ok("ok")
}
