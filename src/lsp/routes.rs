//! The HTTP surface a remote window asks language questions through. Each route is a thin
//! wrapper over [`super`], which a window on this machine calls directly.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
};

use crate::api::{
    AppError, AppState, FileQuery, LspCompletionsPayload, LspDocumentRequest, LspFileEditsPayload,
    LspLocationsPayload, LspPositionRequest, LspRenamablePayload, LspRenameRequest,
    LspStatusPayload, LspTriggersPayload, LspWorkPayload,
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

/// What opens a completion list in this file on its own. A read of what the server said as
/// it started, so it is a `GET` beside the status rather than a question about a place.
pub(crate) async fn triggers(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<LspTriggersPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspTriggersPayload {
        triggers: super::trigger_characters(&state, &session_id, &query.file_path),
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

pub(crate) async fn places(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::LspPlacesRequest>,
) -> Result<Json<LspLocationsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspLocationsPayload {
        locations: super::places(
            &state,
            &session_id,
            &request.file_path,
            request.at,
            request.which,
        )?,
    }))
}

pub(crate) async fn prepare_rename(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<LspRenamablePayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspRenamablePayload {
        name: super::prepare_rename(&state, &session_id, &request.file_path, request.at)?,
    }))
}

/// Everything a rename changes. Nothing is written here: which of those files a window has
/// open in a tab is the window's to know - see `crate::native::renaming`.
pub(crate) async fn rename(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspRenameRequest>,
) -> Result<Json<LspFileEditsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(LspFileEditsPayload {
        files: super::rename(
            &state,
            &session_id,
            &request.file_path,
            request.at,
            &request.new_name,
        )?,
    }))
}

pub(crate) async fn format(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::LspFormatRequest>,
) -> Result<Json<crate::api::LspTextEditsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(crate::api::LspTextEditsPayload {
        edits: super::format(&state, &session_id, &request.file_path, request.options)?,
    }))
}

pub(crate) async fn hover(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<crate::api::LspHoverPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(crate::api::LspHoverPayload {
        markdown: super::hover(&state, &session_id, &request.file_path, request.at)?,
    }))
}

/// What a server last said is wrong with a file. A read of what it already published, so a
/// `GET` beside the status rather than a question about a place.
pub(crate) async fn diagnostics(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<FileQuery>,
    State(state): State<AppState>,
) -> Result<Json<crate::api::LspDiagnosticsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(crate::api::LspDiagnosticsPayload {
        diagnostics: super::diagnostics(&state, &session_id, &query.file_path)?,
    }))
}

pub(crate) async fn did_save(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<crate::api::FileRequest>,
) -> Result<&'static str, AppError> {
    crate::api::mark_activity(&state.last_activity);
    super::did_save(&state, &session_id, &request.file_path)?;
    Ok("ok")
}

pub(crate) async fn code_actions(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<crate::api::LspCodeActionsPayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(crate::api::LspCodeActionsPayload {
        actions: super::code_actions(&state, &session_id, &request.file_path, request.at)?,
    }))
}

pub(crate) async fn signature_help(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    Json(request): Json<LspPositionRequest>,
) -> Result<Json<crate::api::LspSignaturePayload>, AppError> {
    crate::api::mark_activity(&state.last_activity);
    Ok(Json(crate::api::LspSignaturePayload {
        signature: super::signature_help(&state, &session_id, &request.file_path, request.at)?,
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
