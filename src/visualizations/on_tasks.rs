//! The visualizations a task's agent announces, kept with the task.
//!
//! A fragment lives in Codex's home, in a folder of the thread that wrote it, which is nowhere a
//! task can point at for long: it is not the repo's, and the thread is not the task's. So the
//! first time a run of a task announces one, the fragment is copied into the task's folder -
//! `.moontasks/<task>/visualizations/<name>.html` - and written down on the task as a resource,
//! which is how the card and the task's pane list it and open it again after the run is gone.
//!
//! The copy is what is shown from then on, the pane beside the running agent included, so a
//! visualization has one pane whether it was opened by the agent or from the task. A fragment
//! the agent rewrites is copied again.

use std::{fs, path::Path};

use anyhow::{Context, Result};

use super::VisualizationView;
use crate::{
    api::{AgentKind, AppState},
    moontasks::{
        TaskResourceKind,
        store::{self, TaskResource},
    },
};

/// The folder of a task a visualization is kept in.
pub(crate) const TASK_VISUALIZATIONS_DIR: &str = "visualizations";

/// Every visualization the server's terminals have announced, with the ones a task's run
/// announced kept on that task and answered by their copy.
pub(crate) fn announced(state: &AppState, session_id: &str) -> Result<Vec<VisualizationView>> {
    let repo_path =
        crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))?;
    state
        .terminals
        .visualizations()
        .into_iter()
        .map(|view| keep_on_task(state, &repo_path, view))
        .collect()
}

fn keep_on_task(
    state: &AppState,
    repo_path: &Path,
    view: VisualizationView,
) -> Result<VisualizationView> {
    let Some(task_id) = state.terminals.owner(&view.terminal_id) else {
        return Ok(view);
    };
    // A server holds the shells of every repo it reviews; a task of another one is kept when
    // that repo's window asks.
    if !store::task_dir(repo_path, &task_id)?.is_dir() {
        return Ok(view);
    }
    keep_in_task_folder(repo_path, &task_id, view)
}

/// Copy the fragment into the task's folder when the copy is missing or older, put it on the
/// task, and answer the view as the copy.
fn keep_in_task_folder(
    repo_path: &Path,
    task_id: &str,
    view: VisualizationView,
) -> Result<VisualizationView> {
    let task_dir = store::task_dir(repo_path, task_id)?;
    let fragment_path = Path::new(&view.fragment_path);
    let file_name = fragment_path
        .file_name()
        .expect("an announced fragment is a file in its thread's folder");
    let copy_path = task_dir.join(TASK_VISUALIZATIONS_DIR).join(file_name);

    if is_stale(fragment_path, &copy_path)? {
        fs::create_dir_all(
            copy_path
                .parent()
                .expect("the copy is inside the task folder"),
        )
        .with_context(|| format!("failed to make the folder for {}", copy_path.display()))?;
        fs::copy(fragment_path, &copy_path)
            .with_context(|| format!("failed to keep {} on the task", view.fragment_path))?;
        record_on_task(repo_path, task_id, &copy_path)?;
    }

    Ok(VisualizationView {
        fragment_path: copy_path.display().to_string(),
        modified_unix_ms: modified_unix_ms(&copy_path)?,
        ..view
    })
}

/// Whether the copy is missing, or older than the fragment the agent last wrote.
///
/// A copy is never older than the fragment it was made from - whether the copy keeps the
/// fragment's time or takes its own - so only a rewrite makes one stale.
fn is_stale(fragment_path: &Path, copy_path: &Path) -> Result<bool> {
    if !copy_path.exists() {
        return Ok(true);
    }
    Ok(modified_unix_ms(fragment_path)? > modified_unix_ms(copy_path)?)
}

/// Put the copy on the task's list, unless it is already there.
///
/// Called for every copy made, so a visualization taken off the task comes back when the agent
/// rewrites it - that is the agent showing it again.
fn record_on_task(repo_path: &Path, task_id: &str, copy_path: &Path) -> Result<()> {
    let file_path = copy_path
        .strip_prefix(repo_path)
        .expect("a task folder is inside its repo")
        .display()
        .to_string();
    let mut metadata = store::read_task(repo_path, task_id)?;
    if metadata.resources.iter().any(|resource| {
        resource.kind == TaskResourceKind::Visualization
            && resource.file_path.as_deref() == Some(file_path.as_str())
    }) {
        return Ok(());
    }
    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: TaskResourceKind::Visualization,
        // Codex is the one agent that announces visualizations so far.
        agent: AgentKind::Codex,
        name: Some(super::page::title_of(copy_path)),
        file_path: Some(file_path),
        // Not the run's terminal: taking a resource off a task ends the terminal it names.
        terminal_id: None,
        agent_session_id: None,
        started_at_unix: store::now_unix(),
    });
    store::write_task(repo_path, task_id, &metadata)
}

/// Whether a path is a visualization kept in one of the repo's task folders - the other kind of
/// path a page is built from, besides a Codex thread's fragment.
pub(crate) fn is_task_copy(repo_path: &Path, fragment_path: &Path) -> bool {
    let (Ok(tasks_root), Ok(fragment_path)) = (
        fs::canonicalize(store::tasks_root(repo_path)),
        fs::canonicalize(fragment_path),
    ) else {
        return false;
    };
    let Ok(relative) = fragment_path.strip_prefix(&tasks_root) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    matches!(
        components.as_slice(),
        [
            std::path::Component::Normal(_),
            std::path::Component::Normal(dir),
            std::path::Component::Normal(_),
        ] if *dir == TASK_VISUALIZATIONS_DIR
    ) && fragment_path
        .extension()
        .is_some_and(|extension| extension == "html")
        && fragment_path.is_file()
}

fn modified_unix_ms(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("failed to read when {} was written", path.display()))?
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a file is written after the epoch")
        .as_millis() as u64)
}

#[cfg(test)]
#[path = "on_tasks_tests.rs"]
mod tests;
