//! The routes the window's own file picker is served by: a folder of this machine's disk
//! listed, and a file picked anywhere on it given a session to be read through.

use axum::{
    Json,
    extract::{Query, State},
};

use crate::{
    api::{
        AppError, AppState, FolderQuery,
        folders::{FilePlaced, FolderListing, PlaceFileRequest},
    },
    service,
};

use super::mark_activity;

/// `GET /api/folder?path=`: the names in one folder, and which of them are folders.
///
/// Any folder the server's account can read, not only the ones under a repo: a picker is for
/// getting to a file wherever it is. That shows a logged-in window nothing its shell tab does
/// not - `ls` there lists the same folders - and it is all this route does: names and kinds,
/// no sizes, no dates, no contents.
pub(super) async fn list_folder(
    State(state): State<AppState>,
    Query(query): Query<FolderQuery>,
) -> Result<Json<FolderListing>, AppError> {
    mark_activity(&state);
    // A folder on a slow disk, or one with a great many entries, is read off the async
    // workers, which also carry every shell's socket.
    let listing = tokio::task::spawn_blocking(move || service::list_folder(&query.path)).await??;
    Ok(Json(listing))
}

/// `POST /api/session/place-file`: a session on the project holding a file, and the file's
/// path inside it.
///
/// Nothing a window could not already do in two steps: `/api/session/open` takes any folder,
/// and a session reads and writes the files under its folder. This finds the folder for it.
pub(super) async fn place_file(
    State(state): State<AppState>,
    Json(request): Json<PlaceFileRequest>,
) -> Result<Json<FilePlaced>, AppError> {
    mark_activity(&state);
    Ok(Json(service::place_file(&state, &request.path)?))
}
