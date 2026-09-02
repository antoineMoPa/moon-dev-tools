//! Everything the board does to the tasks on disk.
//!
//! Like [`crate::service`], this is synchronous and takes `&AppState`, so the native window
//! calls it directly and the axum routes are a thin skin over the same functions.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    api::{AgentKind, AppState},
    agent::agent_is_available,
    moontasks::{
        AttachResourceRequest, CreateTaskRequest, StartResourceRequest, TaskResourceView,
        TaskView, agent_launch,
        store::{
            self, BoardColumn, BoardConfig, ColumnId, TaskMetadata, TaskResource, TaskResourceKind,
        },
    },
    terminal::{TerminalProgram, TerminalSpec},
};

/// The repo a session's board belongs to.
fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
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
    Ok(tasks.into_iter().map(|(_, task)| task).collect())
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
    for task_id in task_ids {
        let mut metadata = store::read_task(&repo_path, task_id)?;
        release_a_finished_task(state, &board, task_id, &mut metadata, &status);
        metadata.status = status.clone();
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
    let at = position.min(column.len());
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

/// Bring a task's record in line with the shells the server actually has, and return whether
/// anything changed.
fn reconcile(state: &AppState, metadata: &mut TaskMetadata) -> bool {
    let mut changed = false;

    for resource in &mut metadata.resources {
        let Some(terminal_id) = resource.terminal_id.clone() else {
            continue;
        };
        if state.terminals.is_live(&terminal_id) {
            continue;
        }
        resource.terminal_id = None;
        changed = true;
    }
    changed
}

/// The board's columns, left to right.
pub(crate) fn list_columns(state: &AppState, session_id: &str) -> Result<Vec<BoardColumn>> {
    let repo_path = repo_of(state, session_id)?;
    Ok(store::read_board(&repo_path).columns)
}

/// Add a column at the right-hand end of the board.
///
/// Its id is made from its name the same way a task folder's is, so the board file stays
/// readable - and made unique, because two columns sharing an id would be one column with two
/// headings and every card in either would be in both.
pub(crate) fn add_column(state: &AppState, session_id: &str, label: &str) -> Result<BoardColumn> {
    let label = label.trim();
    if label.is_empty() {
        bail!("a column needs a name");
    }

    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);

    let base = store::slug_of(label);
    let mut id = ColumnId::new(base.clone());
    for suffix in 2.. {
        if !board.has(&id) {
            break;
        }
        id = ColumnId::new(format!("{base}-{suffix}"));
    }

    let column = BoardColumn {
        id,
        label: label.to_string(),
        default_agent: None,
    };
    board.columns.push(column.clone());
    store::write_board(&repo_path, &board)?;
    Ok(column)
}

/// Rename a column. The id is left alone, so every card in it stays in it.
pub(crate) fn rename_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    label: &str,
) -> Result<()> {
    let label = label.trim();
    if label.is_empty() {
        bail!("a column needs a name");
    }

    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    let Some(column) = board
        .columns
        .iter_mut()
        .find(|column| column.id == *column_id)
    else {
        bail!("{column_id} is not a column of this board");
    };
    column.label = label.to_string();
    store::write_board(&repo_path, &board)
}

/// Take a column off the board.
///
/// Only an empty one: a column holding cards is the only record of where those cards are, and
/// deleting it would either lose them or move them somewhere nobody asked for. The board says
/// as much rather than choosing on the user's behalf.
pub(crate) fn delete_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    if !board.has(column_id) {
        bail!("{column_id} is not a column of this board");
    }
    if board.columns.len() == 1 {
        bail!("a board needs a column to put its cards in");
    }

    let holding = store::list_task_ids(&repo_path)?
        .iter()
        .filter_map(|task_id| store::read_task(&repo_path, task_id).ok())
        .filter(|metadata| metadata.status == *column_id)
        .count();
    if holding > 0 {
        bail!(
            "there {} still {holding} card{} in this column - move {} out first",
            if holding == 1 { "is" } else { "are" },
            if holding == 1 { "" } else { "s" },
            if holding == 1 { "it" } else { "them" }
        );
    }

    board.columns.retain(|column| column.id != *column_id);
    store::write_board(&repo_path, &board)
}

/// Put a column at a place among the others, which is what dragging its heading does.
///
/// The cards go with it: a card names its column, so where the column is drawn is where its
/// cards are drawn, and nothing about any of them has to be rewritten.
pub(crate) fn place_column(
    state: &AppState,
    session_id: &str,
    column_id: &ColumnId,
    position: usize,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut board = store::read_board(&repo_path);
    let Some(at) = board.position_of(column_id) else {
        bail!("{column_id} is not a column of this board");
    };

    let column = board.columns.remove(at);
    board
        .columns
        .insert(position.min(board.columns.len()), column);
    store::write_board(&repo_path, &board)
}

/// A run's name as its own card writes it: the card is already the task, so the task's title
/// comes off the front - `claude - 1` on the card for the `write the parser claude - 1` tab.
/// A name that does not start with the title is written as it is, which is what a name
/// someone retyped, and one written down before runs carried the title, look like.
fn label_on_the_card(title: &str, name: String) -> String {
    let Some(front) = crate::terminal::title_in_name(title) else {
        return name;
    };
    match name.strip_prefix(&format!("{front} ")) {
        Some(rest) => rest.to_string(),
        None => name,
    }
}

fn view_of(state: &AppState, repo_path: &Path, task_id: &str, metadata: &TaskMetadata) -> TaskView {
    // Agent runs and linked files are the task's record and outlive the process; its shells
    // are only ever the ones open right now, so the two are listed from different places and
    // merged by age.
    let mut resources: Vec<TaskResourceView> = metadata
        .resources
        .iter()
        .map(|resource| match resource.kind {
            TaskResourceKind::File => {
                let Some(file_path) = resource.file_path.clone() else {
                    panic!("linked file {} has no file path", resource.id);
                };
                TaskResourceView {
                    id: resource.id.clone(),
                    kind: TaskResourceKind::File,
                    agent: AgentKind::None,
                    label: file_path.clone(),
                    file_path: Some(file_path),
                    running: false,
                    terminal_id: None,
                    resumable: false,
                    started_at_unix: resource.started_at_unix,
                }
            }
            TaskResourceKind::Shell | TaskResourceKind::Agent => TaskResourceView {
                id: resource.id.clone(),
                kind: resource.kind,
                agent: resource.agent,
                // What the run's shell is called - `claude - 2`, or whatever it was renamed
                // to - which the run keeps once the shell is gone. The agent alone for a run
                // written down before runs had names.
                label: resource
                    .name
                    .clone()
                    .map(|name| label_on_the_card(&metadata.title, name))
                    .unwrap_or_else(|| resource.agent.label().to_lowercase()),
                file_path: None,
                running: resource
                    .terminal_id
                    .as_ref()
                    .is_some_and(|terminal_id| state.terminals.is_live(terminal_id)),
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
                label: shell
                    .name
                    .map(|name| label_on_the_card(&metadata.title, name))
                    .unwrap_or_else(|| "shell".to_string()),
                file_path: None,
                running: true,
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
        dir_path: store::tasks_root(repo_path)
            .join(task_id)
            .display()
            .to_string(),
        repo_path: repo_path.display().to_string(),
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

    // The column remembers the agent this task was created with - including "none" - so the
    // next task created in it starts from the same choice.
    let mut board = store::read_board(&repo_path);
    if let Some(column) = board
        .columns
        .iter_mut()
        .find(|column| column.id == request.status)
        && column.default_agent != Some(request.agent)
    {
        column.default_agent = Some(request.agent);
        store::write_board(&repo_path, &board)?;
    }

    // A task created with an agent starts working straight away, which is the whole point of
    // creating it here rather than making the folder by hand.
    if request.agent != AgentKind::None {
        start_resource(
            state,
            session_id,
            &task_id,
            StartResourceRequest {
                kind: TaskResourceKind::Agent,
                agent: request.agent,
            },
        )?;
    }

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
    }
}

pub(crate) fn delete_task(state: &AppState, session_id: &str, task_id: &str) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    state.terminals.remove_owned_by(task_id);
    store::delete_task(&repo_path, task_id)
}

/// Start a shell or an agent in a task, and record it on the task.
pub(crate) fn start_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    request: StartResourceRequest,
) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let agent = match request.kind {
        TaskResourceKind::Shell => AgentKind::None,
        TaskResourceKind::Agent => {
            if !agent_is_available(state.agent_availability, request.agent) {
                bail!("{} is not installed here", request.agent.label());
            }
            request.agent
        }
        TaskResourceKind::File => bail!("a file is linked to a task, not started"),
    };
    let launch = agent_launch(agent);
    // Only an agent whose start args name a session id has a run that can be resumed exactly.
    let agent_session_id = launch
        .filter(|launch| launch.start.iter().any(|arg| arg.contains("{session}")))
        .map(|_| store::new_uuid());

    let fillings =
        write_task_files(task_id, &repo_path, &metadata)?.with_session(agent_session_id.as_deref());
    let args = match launch {
        Some(launch) => fillings.fill_all(launch.start.iter()),
        None => Vec::new(),
    };
    let env = task_env(session_id, task_id, &repo_path);
    let program = TerminalProgram::of_agent(Some(agent));
    let name =
        crate::terminal::name_for_new_shell(state, &repo_path, Some(&metadata.title), &program)?;

    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program,
        args,
        env,
        owner: Some(task_id.to_string()),
        name: Some(name.clone()),
        // An agent comes up with the card's title already written in its box, waiting on the
        // Enter that sends it. It is still the person who starts the work - the title is a
        // card's name and rarely the whole of what is wanted - but the common case, where it
        // is, is one keystroke away. A task's plain shell gets nothing typed at it.
        type_ahead: (request.kind == TaskResourceKind::Agent).then(|| metadata.title.clone()),
    })?;

    // A shell is not written down: nothing survives its pty, so a record of one from a run of
    // moonreview that has ended is a card entry with nowhere to go. The registry lists the
    // ones that are open, and that is the whole life of a shell.
    if request.kind == TaskResourceKind::Agent {
        metadata.resources.push(TaskResource {
            id: store::new_uuid(),
            kind: request.kind,
            agent,
            file_path: None,
            terminal_id: Some(terminal_id.clone()),
            agent_session_id,
            name: Some(name),
            started_at_unix: store::now_unix(),
        });
        store::write_task(&repo_path, task_id, &metadata)?;
    }

    Ok(terminal_id)
}

/// Start a past agent run again where it left off.
pub(crate) fn resume_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    resource_id: &str,
) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let Some(at) = metadata
        .resources
        .iter()
        .position(|resource| resource.id == resource_id)
    else {
        bail!("that run is not on this task any more");
    };
    let resource = metadata.resources[at].clone();
    if let Some(terminal_id) = &resource.terminal_id
        && state.terminals.is_live(terminal_id)
    {
        return Ok(terminal_id.clone());
    }
    let Some(launch) = agent_launch(resource.agent) else {
        bail!("a shell cannot be resumed - open a new one");
    };
    if !agent_is_available(state.agent_availability, resource.agent) {
        bail!("{} is not installed here", resource.agent.label());
    }

    let fillings = write_task_files(task_id, &repo_path, &metadata)?
        .with_session(resource.agent_session_id.as_deref());
    // A run that recorded its session id is picked up by that exact session; one that could
    // not is left to the agent's own reckoning of what its last run was.
    let template = match resource.agent_session_id {
        Some(_) => launch.attach,
        None => launch.resume,
    };
    let program = TerminalProgram::Agent(resource.agent);
    // The run keeps the name it had; one written down before runs had names is numbered now.
    let name = match resource.name {
        Some(name) => name,
        None => crate::terminal::name_for_new_shell(
            state,
            &repo_path,
            Some(&metadata.title),
            &program,
        )?,
    };
    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program,
        args: fillings.fill_all(template.iter()),
        env: task_env(session_id, task_id, &repo_path),
        owner: Some(task_id.to_string()),
        name: Some(name.clone()),
        // A resumed run is being picked up where it left off, and it was told the title when
        // it started; typing it again would be typing over whatever it is in the middle of.
        type_ahead: None,
    })?;

    metadata.resources[at].terminal_id = Some(terminal_id.clone());
    metadata.resources[at].name = Some(name);
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
}

/// Put a session an agent already has on a task, and open a shell resumed on it.
///
/// This is how a task is pointed back at real work when its recorded session id stopped
/// meaning anything - the id here was read off the agent's own records, so opening it is
/// the same as resuming a run the task had recorded itself.
pub(crate) fn attach_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    request: &AttachResourceRequest,
) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let agent_session_id = request.agent_session_id.trim();
    if agent_session_id.is_empty() {
        bail!("attaching needs the id of the session to attach");
    }
    let Some(launch) = agent_launch(request.agent) else {
        bail!("only an agent's session can be attached");
    };
    if !agent_is_available(state.agent_availability, request.agent) {
        bail!("{} is not installed here", request.agent.label());
    }

    let fillings =
        write_task_files(task_id, &repo_path, &metadata)?.with_session(Some(agent_session_id));
    let program = TerminalProgram::Agent(request.agent);
    let name =
        crate::terminal::name_for_new_shell(state, &repo_path, Some(&metadata.title), &program)?;
    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program,
        args: fillings.fill_all(launch.attach.iter()),
        env: task_env(session_id, task_id, &repo_path),
        owner: Some(task_id.to_string()),
        name: Some(name.clone()),
        // The session being attached is already under way; typing the title at it would be
        // typing over whatever it is in the middle of.
        type_ahead: None,
    })?;

    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: TaskResourceKind::Agent,
        agent: request.agent,
        file_path: None,
        terminal_id: Some(terminal_id.clone()),
        agent_session_id: Some(agent_session_id.to_string()),
        name: Some(name),
        started_at_unix: store::now_unix(),
    });
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
}

/// Write down what a task's run is now called, for the run whose shell this is. A task's
/// plain shell is not written down at all, so renaming one changes nothing here.
pub(crate) fn record_run_name(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    terminal_id: &str,
    name: &str,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    let Some(run) = metadata
        .resources
        .iter_mut()
        .find(|resource| resource.terminal_id.as_deref() == Some(terminal_id))
    else {
        return Ok(());
    };
    run.name = Some(name.trim().to_string());
    store::write_task(&repo_path, task_id, &metadata)
}

/// Close one of a task's shells, if that is what the id names.
///
/// A shell goes by its terminal id, because the registry is the only place it is listed.
/// Ending it is all there is to do with it, so `stop` and `delete` both come through here.
fn close_shell(state: &AppState, task_id: &str, resource_id: &str) -> bool {
    let owned = state
        .terminals
        .owned_shells(task_id)
        .into_iter()
        .any(|shell| shell.terminal_id == resource_id);
    if owned {
        state.terminals.remove(resource_id);
    }
    owned
}

/// Take a run off a task for good, ending its shell if it is still running.
///
/// `stop` keeps the run so it can be resumed; this is for the ones that are finished with.
pub(crate) fn delete_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    resource_id: &str,
) -> Result<()> {
    if close_shell(state, task_id, resource_id) {
        return Ok(());
    }
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let Some(at) = metadata
        .resources
        .iter()
        .position(|resource| resource.id == resource_id)
    else {
        bail!("that run is not on this task any more");
    };
    if let Some(terminal_id) = metadata.resources.remove(at).terminal_id {
        state.terminals.remove(&terminal_id);
    }
    store::write_task(&repo_path, task_id, &metadata)
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

/// End one of a task's shells, leaving the run recorded so it can be resumed.
pub(crate) fn stop_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    resource_id: &str,
) -> Result<()> {
    if close_shell(state, task_id, resource_id) {
        return Ok(());
    }
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;

    let Some(resource) = metadata
        .resources
        .iter_mut()
        .find(|resource| resource.id == resource_id)
    else {
        bail!("that run is not on this task any more");
    };
    if let Some(terminal_id) = resource.terminal_id.take() {
        state.terminals.remove(&terminal_id);
    }
    store::write_task(&repo_path, task_id, &metadata)
}

/// Everything an agent's command line and environment can be filled in with, for one run.
///
/// Built once per start, because most of it is a file that has just been written into the
/// task folder and the rest is where this run of moonreview is answering.
struct Fillings {
    values: Vec<(&'static str, String)>,
}

impl Fillings {
    /// The session id is only known once it has been decided there will be one, which is
    /// after everything else here has been worked out.
    fn with_session(mut self, agent_session_id: Option<&str>) -> Self {
        if let Some(session) = agent_session_id {
            self.values.push(("{session}", session.to_string()));
        }
        self
    }

    /// Fill one template string in. `None` means a placeholder in it had nothing to fill it.
    ///
    /// Whether it can be filled is decided from the template, before anything is substituted:
    /// a value may itself contain braces, so the result cannot be read for leftovers.
    fn fill(&self, template: &str) -> Option<String> {
        let missing = crate::moontasks::LAUNCH_PLACEHOLDERS.iter().any(|name| {
            template.contains(name) && !self.values.iter().any(|(known, _)| known == name)
        });
        if missing {
            return None;
        }

        let mut filled = template.to_string();
        for (name, value) in &self.values {
            filled = filled.replace(name, value);
        }
        Some(filled)
    }

    /// Fill an argument list in, dropping any argument that cannot be filled - along with the
    /// flag in front of it, which would otherwise be left dangling.
    fn fill_all<'a>(&self, template: impl Iterator<Item = &'a &'static str>) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        for argument in template {
            match self.fill(argument) {
                Some(filled) => args.push(filled),
                None => {
                    args.pop();
                }
            }
        }
        args
    }
}

/// What every process started for a task is told about itself.
fn task_env(session_id: &str, task_id: &str, repo_path: &Path) -> Vec<(String, String)> {
    vec![
        (super::TASK_ID_ENV_VAR.to_string(), task_id.to_string()),
        (
            super::TASK_DIR_ENV_VAR.to_string(),
            store::tasks_root(repo_path)
                .join(task_id)
                .display()
                .to_string(),
        ),
        (
            super::SESSION_ID_ENV_VAR.to_string(),
            session_id.to_string(),
        ),
        (
            super::SERVER_URL_ENV_VAR.to_string(),
            crate::api::export_server_url(),
        ),
    ]
}

/// Write what an agent working in this task needs to read, and answer with everything its
/// command line can be filled in from.
///
/// The brief is rewritten on every start rather than once at creation, so a card that has been
/// renamed starts its next agent on the name it has now.
fn write_task_files(task_id: &str, repo_path: &Path, metadata: &TaskMetadata) -> Result<Fillings> {
    let dir = store::task_dir(repo_path, task_id)?;

    let brief = super::brief_for(&metadata.title, &dir.display().to_string());
    let path = dir.join(super::BRIEF_FILE_NAME);
    std::fs::write(&path, format!("{brief}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;

    // The brief points the agent at notes.md, so by the time one reads it the file is there.
    // Only made, never rewritten - it is the task's own record, unlike the brief.
    store::ensure_notes_file(repo_path, task_id)?;

    // The format the brief sends the agent to read, written beside it for the same reason and
    // rewritten for the same reason: it is ours, not the task's, and a task started today should
    // be reading today's.
    let path = dir.join(super::REVIEW_REQUEST_BRIEF_FILE_NAME);
    std::fs::write(&path, crate::moontasks::review_request::REVIEW_REQUEST_BRIEF)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(Fillings {
        values: vec![("{brief}", brief)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's fillings, without the brief file a real one writes.
    fn fillings() -> Fillings {
        Fillings {
            values: vec![("{brief}", "the brief".to_string())],
        }
    }

    /// Claude is given the session id and the brief - and no prompt, so it comes up knowing
    /// the task and waits to be told what to do about it.
    #[test]
    fn an_agent_is_told_the_task_but_not_set_to_work() {
        let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

        let args = fillings()
            .with_session(Some("11111111-2222-4333-8444-555555555555"))
            .fill_all(launch.start.iter());

        assert_eq!(
            args,
            [
                "--session-id",
                "11111111-2222-4333-8444-555555555555",
                "--append-system-prompt",
                "the brief",
            ]
        );
    }

    #[test]
    fn a_missing_session_id_takes_the_flag_in_front_of_it_with_it() {
        let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

        let args = fillings().fill_all(launch.attach.iter());

        assert!(
            !args.iter().any(|arg| arg == "--resume"),
            "expected no dangling --resume: {args:?}"
        );
        // What is left is a fresh run that still knows the task, which is the best that can be
        // done with a session that turned out not to be there.
        assert_eq!(args, ["--append-system-prompt", "the brief"]);
    }

    /// Attaching opens the exact session that was picked, whichever agent it belongs to.
    ///
    /// Claude is handed the brief with it: a session resumed comes back with the system prompt it
    /// was opened on, so without this a run started before the board asked for anything would
    /// never hear that it can.
    #[test]
    fn an_attached_session_is_opened_by_its_own_id() {
        let session = "11111111-2222-4333-8444-555555555555";
        let expected: &[(AgentKind, &[&str])] = &[
            (
                AgentKind::Claude,
                &["--resume", session, "--append-system-prompt", "the brief"],
            ),
            (AgentKind::Codex, &["resume", session]),
            (AgentKind::OpenCode, &["--session", session]),
        ];

        for (kind, args) in expected {
            let launch = agent_launch(*kind).expect("expected the agent to be launchable");

            assert_eq!(
                fillings().with_session(Some(session)).fill_all(launch.attach.iter()),
                *args,
                "{kind:?} did not open the picked session"
            );
        }
    }

    #[test]
    fn an_agent_resumed_by_its_own_reckoning_needs_no_session_id() {
        let launch = agent_launch(AgentKind::Codex).expect("expected codex to be launchable");

        let args = fillings().fill_all(launch.resume.iter());

        assert_eq!(args, ["resume", "--last"]);
    }

    /// OpenCode and Codex are handed nothing at all on a fresh run: they come up in the repo
    /// waiting, with the brief in the task folder for whoever needs it.
    #[test]
    fn an_agent_with_no_start_args_comes_up_waiting_rather_than_working() {
        for kind in [AgentKind::Codex, AgentKind::OpenCode] {
            let launch = agent_launch(kind).expect("expected the agent to be launchable");

            assert!(
                fillings().fill_all(launch.start.iter()).is_empty(),
                "{kind:?} should come up waiting rather than working"
            );
        }
    }

    /// Starting an agent opens a conversation rather than firing a job off, so none of the
    /// three is handed the work.
    #[test]
    fn no_agent_is_given_the_task_as_a_prompt() {
        for launch in crate::moontasks::AGENT_LAUNCHES {
            let args = fillings()
                .with_session(Some("11111111-2222-4333-8444-555555555555"))
                .fill_all(launch.start.iter());

            assert!(
                !args.iter().any(|arg| arg.contains("Fix the login page")),
                "{:?} should not be started on the work: {args:?}",
                launch.kind
            );
        }
    }

    /// The brief names the task and says where its notes go, which is all an agent needs to
    /// know beyond the work itself.
    #[test]
    fn the_brief_names_the_task_and_its_folder() {
        let brief = crate::moontasks::brief_for("Fix the login page", "/repo/.moontasks/task");

        assert!(brief.contains("Fix the login page"));
        assert!(brief.contains("/repo/.moontasks/task"));
    }

    #[test]
    fn a_finished_agent_is_cleared_without_moving_its_task() {
        let mut metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: ColumnId::new("in_progress"),
            created_at_unix: 0,
            position: 0,
            resources: vec![TaskResource {
                id: "resource".to_string(),
                kind: TaskResourceKind::Agent,
                agent: AgentKind::Claude,
                file_path: None,
                // No server in this test, so no shell of this name is live.
                terminal_id: Some("terminal-gone".to_string()),
                agent_session_id: None,
                name: None,
                started_at_unix: 0,
            }],
        };
        let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
            std::time::Instant::now(),
        )));

        assert!(reconcile(&state, &mut metadata));

        assert_eq!(metadata.status, ColumnId::new("in_progress"));
        assert_eq!(metadata.resources[0].terminal_id, None);
        assert!(
            !reconcile(&state, &mut metadata),
            "a settled task changes nothing on the next read"
        );
    }

    #[test]
    fn a_finished_agent_leaves_a_task_where_the_user_put_it() {
        let mut metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: ColumnId::new("done"),
            created_at_unix: 0,
            position: 0,
            resources: vec![TaskResource {
                id: "resource".to_string(),
                kind: TaskResourceKind::Agent,
                agent: AgentKind::Claude,
                file_path: None,
                terminal_id: Some("terminal-gone".to_string()),
                agent_session_id: None,
                name: None,
                started_at_unix: 0,
            }],
        };
        let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
            std::time::Instant::now(),
        )));

        assert!(
            reconcile(&state, &mut metadata),
            "the shell is still recorded"
        );

        assert_eq!(metadata.status, ColumnId::new("done"));
    }
}
