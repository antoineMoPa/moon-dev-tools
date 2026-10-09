//! The routes the window's own file picker is served by: a folder of this machine's disk
//! listed, and a file picked anywhere on it given a session to be read through.
//!
//! Neither has a session to say whose the work is, so each does it as whoever the request was
//! let in as - see [`for_person`].

use axum::{
    Extension, Json,
    extract::{Query, State},
};

use crate::{
    api::{
        AppError, AppState, FolderQuery,
        folders::{FilePlaced, FolderListing, PlaceFileRequest},
    },
    service,
};

use super::{for_person, mark_activity};

/// `GET /api/folder?path=`: the names in one folder, and which of them are folders.
///
/// Any folder the account the shells run as can read, not only the ones under a repo: a
/// picker is for getting to a file wherever it is. That shows a logged-in window nothing its
/// shell tab does not - `ls` there lists the same folders - and it is all this route does:
/// names and kinds, no sizes, no dates, no contents.
pub(super) async fn list_folder(
    State(state): State<AppState>,
    State(profiles): State<super::profiles::Profiles>,
    Extension(user): Extension<super::users::UserId>,
    Query(query): Query<FolderQuery>,
) -> Result<Json<FolderListing>, AppError> {
    mark_activity(&state);
    // A folder on a slow disk, or one with a great many entries, is read off the async
    // workers, which also carry every shell's socket.
    let person = profiles.session_owner(&user)?.person;
    let listing = for_person(person, move || service::list_folder(&query.path)).await?;
    Ok(Json(listing))
}

/// `POST /api/session/place-file`: a session on the project holding a file, and the file's
/// path inside it.
///
/// Nothing a window could not already do in two steps: `/api/session/open` takes any folder,
/// and a session reads and writes the files under its folder. This finds the folder for it.
pub(super) async fn place_file(
    State(state): State<AppState>,
    State(profiles): State<super::profiles::Profiles>,
    Extension(user): Extension<super::users::UserId>,
    Json(request): Json<PlaceFileRequest>,
) -> Result<Json<FilePlaced>, AppError> {
    mark_activity(&state);
    let owner = profiles.session_owner(&user)?;
    let placed = for_person(owner.person.clone(), move || {
        service::place_file_for_profile(&state, &request.path, Some(owner))
    })
    .await?;
    Ok(Json(placed))
}
