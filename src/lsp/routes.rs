//! The HTTP surface a remote window asks language questions through. Each route is a thin
//! wrapper over [`super`], which a window on this machine calls directly.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};

use crate::api::{
    AppError, AppState, FileQuery, LspCompletionsPayload, LspDocumentRequest, LspLocationsPayload,
    LspPositionRequest, LspStatusPayload, LspWorkPayload,
};

pub(crate) async fn status(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<LspStatusPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspStatusPayload {
        status: super::status(&state, &session_id, &query.file_path)?,
    }))
}

/// What every server running for this session is doing. Answered without touching a server:
/// the progress notifications have already been folded into what each one is doing, so this
/// is a read of what is there rather than a question anything has to wait on.
pub(crate) async fn working(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<Json<LspWorkPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspWorkPayload {
        working: super::working(&state, &session_id),
    }))
}

pub(crate) async fn did_open(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspDocumentRequest>,
) -> Result<&'static str, AppError> {
    crate::api::mark_activity(&state.last_activity);
    super::did_open(&state, &session_id, &request.file_path, &request.text)?;
    Ok("ok")
}

pub(crate) async fn did_change(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspDocumentRequest>,
) -> Result<&'static str, AppError> {
    crate::api::mark_activity(&state.last_activity);
    super::did_change(&state, &session_id, &request.file_path, &request.text)?;
    Ok("ok")
}

pub(crate) async fn did_close(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    crate::api::mark_activity(&state.last_activity);
    super::did_close(&state, &session_id, &request.file_path)?;
    Ok("ok")
}

pub(crate) async fn definition(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<LspLocationsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspLocationsPayload {
        locations: super::definition(&state, &session_id, &request.file_path, request.at)?,
    }))
}

pub(crate) async fn completion(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<LspCompletionsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspCompletionsPayload {
        completions: super::completion(&state, &session_id, &request.file_path, request.at)?,
    }))
}
