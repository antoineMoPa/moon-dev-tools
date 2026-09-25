//! Everything the board does to the tasks on disk.
//!
//! Like [`crate::service`], this is synchronous and takes `&AppState`, so the native window
//! calls it directly and the axum routes are a thin skin over the same functions.

mod columns;
mod resources;
#[cfg(test)]
mod tests;

pub(crate) use columns::{
    add_column, delete_column, list_columns, place_column, rename_column, set_column_arrivals,
    set_column_sort,
};
pub(super) use resources::task_env;
pub(crate) use resources::{
    attach_resource, delete_resource, record_run_name, resume_resource, start_resource,
    stop_resource,
};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    api::{AgentKind, AppState},
    moontasks::{
        CreateTaskRequest, ReviewRequestView, TaskResourceView, TaskView, agent_launch,
        column_sort,
        review_request::{self, Amend},
        store::{
            self, BoardConfig, ColumnEnd, ColumnId, TaskMetadata, TaskResource, TaskResourceKind,
        },
    },
};

/// The repo a session's board belongs to.
pub(super) fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

/// Every task on the board, with what each one has running right now.
///
/// Reading the board is also when it catches up with reality by clearing resources whose
/// shells have exited, including shells lost with a previous run of the server.
pub(crate) fn list_tasks(state: &AppState, session_id: &str) -> Result<Vec<TaskView>> {
    let repo_path = repo_of(state, session_id)?;
    let mut tasks = Vec::new();

    for task_id in store::list_task_ids(&repo_path)? {
        let Ok(mut metadata) = store::read_task(&repo_path, &task_id) else {
            // A half-written or hand-edited `metadata.json` is skipped rather than fatal: the
            // rest of the board is still worth showing.
            continue;
        };
        if reconcile(state, &mut metadata) {
            store::write_task(&repo_path, &task_id, &metadata)?;
        }
        tasks.push((
            place_of(&metadata),
            view_of(state, &repo_path, &task_id, &metadata),
        ));
    }

    // One order for the whole board, which each column reads its own cards out of.
    tasks.sort_by_key(|(place, _)| *place);
    let mut tasks: Vec<TaskView> = tasks.into_iter().map(|(_, task)| task).collect();
    // And a column that keeps an order of its own is read out in that one instead - see
    // [`column_sort`]. The places are left as they are underneath, for when it no longer does.
    column_sort::arrange(&store::read_board(&repo_path).columns, &mut tasks);
    Ok(tasks)
}

/// What a card is sorted by inside its column: where it was put, and - for cards off a board
/// written before they had a place, which all read as zero - the order they were created in.
fn place_of(metadata: &TaskMetadata) -> (u32, u64) {
    (metadata.position, metadata.created_at_unix)
}

/// Move tasks to a column and to a place in it, renumbering that column from the top.
///
/// `position` is counted among the column's other cards, so it is where the first of them
/// lands rather than where it was aimed: a number past the end puts them at the bottom. More
/// than one task is a card dragged with others selected - they land as a run, in the order
/// the board already had them.
pub(crate) fn place_tasks(
    state: &AppState,
    session_id: &str,
    task_ids: &[String],
    status: ColumnId,
    position: usize,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let board = store::read_board(&repo_path);
    if !board.has(&status) {
        bail!("{status} is not a column of this board");
    }

    let mut moving: Vec<(String, TaskMetadata)> = Vec::new();
    // Whether any of them is arriving from another column, which is what the column's own
    // `arrivals` end is about: cards shuffled about within a column go where they were put.
    let mut arriving = false;
    for task_id in task_ids {
        let mut metadata = store::read_task(&repo_path, task_id)?;
        release_a_finished_task(state, &board, task_id, &mut metadata, &status);
        if metadata.status != status {
            arriving = true;
            metadata.status = status.clone();
            metadata.entered_column_at_unix = Some(store::now_unix());
        }
        moving.push((task_id.clone(), metadata));
    }
    // The order they were in before the drag, which is the order they keep: the board hands
    // them over that way, and a board read another way round would otherwise reorder them.
    moving.sort_by_key(|(_, metadata)| place_of(metadata));

    // The cards already in that column, in the order the board draws them.
    let mut column: Vec<(String, TaskMetadata)> = store::list_task_ids(&repo_path)?
        .into_iter()
        .filter(|other| !task_ids.contains(other))
        .filter_map(|other| {
            let metadata = store::read_task(&repo_path, &other).ok()?;
            (metadata.status == status).then_some((other, metadata))
        })
        .collect();
    column.sort_by_key(|(_, metadata)| place_of(metadata));
    // A column that says which end arrivals go to takes them there instead of where the drop
    // landed - see [`store::BoardColumn::arrivals`].
    let at = match board.arrivals_end(&status).filter(|_| arriving) {
        Some(ColumnEnd::Top) => 0,
        Some(ColumnEnd::Bottom) => column.len(),
        None => position.min(column.len()),
    };
    for (offset, moved) in moving.into_iter().enumerate() {
        column.insert(at + offset, moved);
    }

    for (index, (id, mut metadata)) in column.into_iter().enumerate() {
        let position = index as u32;
        // A card that moved is written whatever its number came out as - it is one whose
        // column may have changed.
        if metadata.position == position && !task_ids.contains(&id) {
            continue;
        }
        metadata.position = position;
        store::write_task(&repo_path, &id, &metadata)?;
    }
    Ok(())
}

/// Bring a task's record in line with the shells there are, and return whether anything
/// changed: a run whose shell is gone is written down as ended, which is what offers it to be
/// resumed.
///
/// Gone is not the same as not here. Every moon on the machine reads the same record, and a
/// shell belongs to the one that started it - a `moon serve` or a second window reading the
/// board must not end the runs of the window beside it. So a shell another moon holds is let
/// be while that moon is running, and only one whose moon has exited is taken for ended.
fn reconcile(state: &AppState, metadata: &mut TaskMetadata) -> bool {
    let mut changed = false;

    for resource in &mut metadata.resources {
        let Some(terminal_id) = resource.terminal_id.clone() else {
            continue;
        };
        if state.terminals.is_live(&terminal_id) {
            continue;
        }
        let held_elsewhere = resource
            .terminal_owner
            .is_some_and(|owner| owner != std::process::id() && process_is_running(owner));
        if held_elsewhere {
            continue;
        }
        resource.terminal_id = None;
        resource.terminal_owner = None;
        changed = true;
    }
    changed
}

/// Whether a process of this id is running. `kill` with no signal only asks: it answers
/// `EPERM` for a process that is there but not this user's, which still counts as running.
fn process_is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 delivers nothing; it only checks that `pid` names a process.
    let asked = unsafe { libc::kill(pid, 0) };
    asked == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn view_of(state: &AppState, repo_path: &Path, task_id: &str, metadata: &TaskMetadata) -> TaskView {
    // Agent runs and linked files are the task's record and outlive the process; its shells
    // are only ever the ones open right now, so the two are listed from different places and
    // merged by age.
    let mut resources: Vec<TaskResourceView> = metadata
        .resources
        .iter()
        .map(|resource| match resource.kind {
            TaskResourceKind::File | TaskResourceKind::Visualization => {
                let Some(file_path) = resource.file_path.clone() else {
                    panic!("{:?} {} has no file path", resource.kind, resource.id);
                };
                TaskResourceView {
                    id: resource.id.clone(),
                    kind: resource.kind,
                    agent: resource.agent,
                    // A file by its path; a visualization by the name it was given as it was
                    // kept, which is what its pane's tab reads.
                    label: resource.name.clone().unwrap_or_else(|| file_path.clone()),
                    file_path: Some(file_path),
                    running: false,
                    quiet_for_secs: None,
                    attention: None,
                    terminal_id: None,
                    resumable: false,
                    started_at_unix: resource.started_at_unix,
                }
            }
            TaskResourceKind::Shell | TaskResourceKind::Agent => TaskResourceView {
                id: resource.id.clone(),
                kind: resource.kind,
                agent: resource.agent,
                // What the run's shell is called - `write the parser claude - 2`, or whatever
                // it was renamed to - which the run keeps once the shell is gone. The name in
                // full, so the row on the card reads as the tab the run is open in. The agent
                // alone for a run written down before runs had names.
                label: resource
                    .name
                    .clone()
                    .unwrap_or_else(|| resource.agent.label().to_lowercase()),
                file_path: None,
                running: resource
                    .terminal_id
                    .as_ref()
                    .is_some_and(|terminal_id| state.terminals.is_live(terminal_id)),
                // Only a run's: a plain shell sits at its prompt printing nothing, and that
                // is not a shell to look at.
                quiet_for_secs: resource
                    .terminal_id
                    .as_ref()
                    .filter(|_| resource.kind == TaskResourceKind::Agent)
                    .and_then(|terminal_id| state.terminals.quiet_for(terminal_id))
                    .map(|quiet| quiet.as_secs()),
                attention: resource
                    .terminal_id
                    .as_ref()
                    .and_then(|terminal_id| state.terminals.attention(terminal_id)),
                terminal_id: resource.terminal_id.clone(),
                resumable: agent_launch(resource.agent).is_some(),
                started_at_unix: resource.started_at_unix,
            },
        })
        .collect();
    resources.extend(
        state
            .terminals
            .owned_shells(task_id)
            .into_iter()
            .map(|shell| TaskResourceView {
                // A shell is its terminal, so that is the name the board takes it off the task by.
                id: shell.terminal_id.clone(),
                kind: TaskResourceKind::Shell,
                agent: AgentKind::None,
                label: shell.name.unwrap_or_else(|| "shell".to_string()),
                file_path: None,
                running: true,
                quiet_for_secs: None,
                attention: state.terminals.attention(&shell.terminal_id),
                terminal_id: Some(shell.terminal_id),
                resumable: false,
                started_at_unix: shell.started_at_unix,
            }),
    );
    resources.sort_by_key(|resource| resource.started_at_unix);

    TaskView {
        id: task_id.to_string(),
        title: metadata.title.clone(),
        status: metadata.status.clone(),
        created_at_unix: metadata.created_at_unix,
        entered_column_at_unix: metadata.entered_column_at_unix,
        dir_path: store::tasks_root(repo_path)
            .join(task_id)
            .display()
            .to_string(),
        repo_path: repo_path.display().to_string(),
        tags: metadata.tags.clone(),
        notes: store::read_notes(repo_path, task_id),
        resources,
    }
}

/// Make sure the task's notes file exists, and answer with where the file pane finds it.
///
/// The file has to be real before the pane opens it: the repo-file pipeline the pane reads and
/// saves through refuses a path that is not a file in the working tree, and a task made before
/// notes existed has none yet.
pub(crate) fn open_notes(state: &AppState, session_id: &str, task_id: &str) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    store::read_task(&repo_path, task_id)?;
    store::ensure_notes_file(&repo_path, task_id)?;
    Ok(super::notes_repo_path(task_id))
}

/// Where the project's work log is, made if the project has none yet - see
/// [`crate::native::work_log`]. Made here for the reason the notes are: the file pane reads
/// and saves through the repo-file pipeline, which wants a file in the working tree.
pub(crate) fn open_work_log(state: &AppState, session_id: &str) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    store::ensure_work_log_file(&repo_path)?;
    Ok(super::work_log_repo_path())
}

/// Put a file of the repo on the task's card.
///
/// The path is kept as the file pane addresses it - relative to the repo root - and has to be
/// a file in the working tree right now: a card is a way back to the file, and one pointing
/// at nothing is worse than none. The same file twice is refused rather than listed twice.
pub(crate) fn link_file(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    file_path: &str,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let file_path = file_path.trim();
    if file_path.is_empty() {
        bail!("a linked file needs a path");
    }
    if Path::new(file_path).is_absolute() {
        bail!("a linked file is named relative to the repo");
    }
    // Both sides are resolved before they are compared: on macOS the repo may be reached
    // through a symlink (`/var` for `/private/var`), and comparing a resolved path against an
    // unresolved root would refuse a file that is plainly inside it.
    let repo_root = repo_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", repo_path.display()))?;
    let resolved = repo_root
        .join(file_path)
        .canonicalize()
        .with_context(|| format!("{file_path} is not a file of the repo"))?;
    if !resolved.starts_with(&repo_root) {
        bail!("{file_path} is outside the repo");
    }
    if !resolved.is_file() {
        bail!("{file_path} is not a file");
    }
    if metadata
        .resources
        .iter()
        .any(|resource| resource.file_path.as_deref() == Some(file_path))
    {
        bail!("{file_path} is already on this task");
    }

    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: TaskResourceKind::File,
        agent: AgentKind::None,
        file_path: Some(file_path.to_string()),
        terminal_id: None,
        terminal_owner: None,
        agent_session_id: None,
        name: None,
        started_at_unix: store::now_unix(),
    });
    store::write_task(&repo_path, task_id, &metadata)
}

pub(crate) fn create_task(
    state: &AppState,
    session_id: &str,
    request: &CreateTaskRequest,
) -> Result<TaskView> {
    let repo_path = repo_of(state, session_id)?;
    let task_id = store::create_task(&repo_path, &request.title, &request.status, request.joins)?;
    let metadata = store::read_task(&repo_path, &task_id)?;
    Ok(view_of(state, &repo_path, &task_id, &metadata))
}

/// A finished task lets go of its shells. Until then they keep running with no tab open,
/// which is what makes closing an agent's tab safe.
fn release_a_finished_task(
    state: &AppState,
    board: &BoardConfig,
    task_id: &str,
    metadata: &mut TaskMetadata,
    status: &ColumnId,
) {
    if board.role(store::RELEASES_SHELLS_IN).as_ref() != Some(status) {
        return;
    }
    state.terminals.remove_owned_by(task_id);
    for resource in &mut metadata.resources {
        resource.terminal_id = None;
        resource.terminal_owner = None;
    }
}

/// Every repo the board's tasks ask to have looked at, and how each of them stands - see
/// [`review_request::list_for_repo`].
pub(crate) fn list_review_requests(
    state: &AppState,
    session_id: &str,
) -> Result<Vec<ReviewRequestView>> {
    Ok(review_request::list_for_repo(&repo_of(state, session_id)?))
}

/// Dismiss one line of a task's `request_for_review.txt`, or cross it off or back on.
pub(crate) fn amend_review_request(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    index: usize,
    amend: Amend,
) -> Result<()> {
    review_request::amend(&repo_of(state, session_id)?, task_id, index, amend)
}

pub(crate) fn delete_task(state: &AppState, session_id: &str, task_id: &str) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    state.terminals.remove_owned_by(task_id);
    store::delete_task(&repo_path, task_id)
}

/// Give a task a different title. The folder keeps the name it was created with, because it
/// is what everything else - shells, agent sessions, whatever an agent wrote - points at.
pub(crate) fn rename_task(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    title: &str,
) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a task needs a title");
    }
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    metadata.title = title.to_string();
    store::write_task(&repo_path, task_id, &metadata)
}

/// What a card is marked with, set whole. Spelled the way the store keeps tags, so a tag typed
/// twice in two spellings lands as one.
pub(crate) fn set_tags(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    tags: &[String],
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    metadata.tags = store::tags_of(tags.iter().map(String::as_str));
    store::write_task(&repo_path, task_id, &metadata)
}
