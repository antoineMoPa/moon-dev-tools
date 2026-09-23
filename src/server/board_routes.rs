//! The routes the task board is read and changed through: its tasks, columns and the shells
//! and agents running on them, and the project's commands.

use anyhow::Result;
use axum::{
    Json,
    extract::{Path as AxumPath, State},
};

use crate::{
    api::{AppError, AppState},
    moontasks::{
        self, AttachResourceRequest, ColumnLabelRequest, ColumnPlacementRequest, CreateTaskRequest,
        LinkFileRequest, NewColumnRequest, StartResourceRequest, TaskNotesPayload,
        TaskPlacementRequest, TaskTagsRequest, TaskTitleRequest, TaskView, TerminalOpened,
        WorkLogPayload,
        store::{BoardColumn, ColumnId},
    },
};

use super::mark_activity;

pub(super) async fn list_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskView>>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::list_tasks(&state, &session_id)?))
}

pub(super) async fn create_task(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<Json<TaskView>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::create_task(
        &state,
        &session_id,
        &request,
    )?))
}

pub(super) async fn delete_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_task(&state, &session_id, &task_id)?;
    Ok("ok")
}

pub(super) async fn project_commands(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<crate::project::ProjectConfig>, AppError> {
    mark_activity(&state);
    Ok(Json(crate::project::session_commands(&state, &session_id)?))
}

pub(super) async fn set_project_config(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::project::ProjectConfig>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    crate::project::set_session_commands(&state, &session_id, &request)?;
    Ok("ok")
}

pub(super) async fn run_project_command(
    AxumPath((session_id, which)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    let terminal_id = crate::project::run(&state, &session_id, which.parse()?)?;
    Ok(Json(TerminalOpened { terminal_id }))
}

pub(super) async fn list_columns(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<BoardColumn>>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::list_columns(&state, &session_id)?))
}

pub(super) async fn add_column(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<NewColumnRequest>,
) -> Result<Json<BoardColumn>, AppError> {
    mark_activity(&state);
    Ok(Json(moontasks::service::add_column(
        &state,
        &session_id,
        &request.label,
        request.at,
    )?))
}

pub(super) async fn rename_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnLabelRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::rename_column(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        &request.label,
    )?;
    Ok("ok")
}

pub(super) async fn set_column_arrivals(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnArrivalsRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::set_column_arrivals(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        request.arrivals,
    )?;
    Ok("ok")
}

pub(super) async fn set_column_sort(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::ColumnSortRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::set_column_sort(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        request.sort,
    )?;
    Ok("ok")
}

pub(super) async fn delete_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_column(&state, &session_id, &ColumnId::new(column_id))?;
    Ok("ok")
}

pub(super) async fn place_column(
    AxumPath((session_id, column_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<ColumnPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::place_column(
        &state,
        &session_id,
        &ColumnId::new(column_id),
        request.position,
    )?;
    Ok("ok")
}

pub(super) async fn place_tasks(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<TaskPlacementRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::place_tasks(
        &state,
        &session_id,
        &request.task_ids,
        request.status,
        request.position,
    )?;
    Ok("ok")
}

pub(super) async fn start_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<StartResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::start_resource(&state, &session_id, &task_id, request)?,
    }))
}

pub(super) async fn explain_task_changes(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<moontasks::explainer::ExplainRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::explainer::start_explanation(
            &state,
            &session_id,
            &task_id,
            request,
        )?,
    }))
}

pub(super) async fn list_agent_sessions(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::agent_sessions::AgentSessionView>>, AppError> {
    mark_activity(&state);
    Ok(Json(crate::agent_sessions::list_for_session(
        &state,
        &session_id,
    )?))
}

pub(super) async fn attach_task_resource(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<AttachResourceRequest>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::attach_resource(&state, &session_id, &task_id, &request)?,
    }))
}

pub(super) async fn resume_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TerminalOpened>, AppError> {
    mark_activity(&state);
    Ok(Json(TerminalOpened {
        terminal_id: moontasks::service::resume_resource(
            &state,
            &session_id,
            &task_id,
            &resource_id,
        )?,
    }))
}

pub(super) async fn stop_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::stop_resource(&state, &session_id, &task_id, &resource_id)?;
    Ok("ok")
}

pub(super) async fn delete_task_resource(
    AxumPath((session_id, task_id, resource_id)): AxumPath<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::delete_resource(&state, &session_id, &task_id, &resource_id)?;
    Ok("ok")
}

pub(super) async fn rename_task(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskTitleRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::rename_task(&state, &session_id, &task_id, &request.title)?;
    Ok("ok")
}

pub(super) async fn set_task_tags(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<TaskTagsRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::set_tags(&state, &session_id, &task_id, &request.tags)?;
    Ok("ok")
}

pub(super) async fn open_task_notes(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<TaskNotesPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(TaskNotesPayload {
        file_path: moontasks::service::open_notes(&state, &session_id, &task_id)?,
    }))
}

pub(super) async fn open_work_log(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<WorkLogPayload>, AppError> {
    mark_activity(&state);
    Ok(Json(WorkLogPayload {
        file_path: moontasks::service::open_work_log(&state, &session_id)?,
    }))
}

pub(super) async fn link_task_file(
    AxumPath((session_id, task_id)): AxumPath<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<LinkFileRequest>,
) -> Result<&'static str, AppError> {
    mark_activity(&state);
    moontasks::service::link_file(&state, &session_id, &task_id, &request.file_path)?;
    Ok("ok")
}
