//! The server's side of visualizations: which ones its terminals have announced, and the page
//! for one of them.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use super::{VisualizationList, VisualizationPage};
use crate::{
    agent_sessions::AgentHome,
    api::{AppError, AppState},
};

#[derive(Serialize, Deserialize)]
pub(crate) struct PageQuery {
    pub(crate) fragment_path: String,
}

/// Every visualization the server's terminals have announced - see [`super::on_tasks`].
pub(crate) async fn announced(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    // Reading the rollouts and copying fragments is file work, kept off the async workers -
    // and done for whoever is asking, who the copies in the project are then written as.
    let visualizations =
        crate::server::session_work(state, session_id, super::on_tasks::announced).await?;
    Ok(Json(VisualizationList { visualizations }))
}

/// The page a fragment is shown on - see [`super::page`].
pub(crate) async fn page(
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<PageQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let html = crate::server::session_work(state, session_id, move |state, session_id| {
        page_of(state, session_id, &query.fragment_path)
    })
    .await?;
    Ok(Json(VisualizationPage { html }))
}

/// The page for a fragment one of Codex's threads holds, or for one kept in a task folder of the
/// session's repo, and nothing else: the path comes from whoever is asking.
///
/// The Codex is the asking person's own: what somebody else's shows is kept on a task, or is
/// not theirs to see - see [`super::on_tasks::announced`].
pub(crate) fn page_of(
    state: &AppState,
    session_id: &str,
    fragment_path: &str,
) -> anyhow::Result<String> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    let fragment_path = std::path::Path::new(fragment_path);
    let codex_home = super::codex_home_of(&AgentHome::of_session(state, session_id)?);
    anyhow::ensure!(
        super::directives::is_thread_fragment(&codex_home, fragment_path)
            || super::on_tasks::is_task_copy(&repo_path, fragment_path),
        "{} is not a visualization of a Codex thread or of a task",
        fragment_path.display()
    );
    super::page::page_for(fragment_path)
}
