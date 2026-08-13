//! Everything both frontends do to the board.
//!
//! Like [`crate::service`], this is synchronous and takes `&AppState`, so the native window
//! calls it directly and the axum routes are a thin skin over the same functions.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    api::{AgentKind, AppState},
    git::agent_is_available,
    moontasks::{
        AgentLaunch, AttachResourceRequest, CreateTaskRequest, StartResourceRequest,
        TaskResourceView, TaskView, agent_launch, autopilot,
        store::{
            self, BoardColumn, BoardConfig, ColumnId, TaskMetadata, TaskResource, TaskResourceKind,
        },
        worktrees,
    },
    terminal::TerminalSpec,
};

/// The repo a session's board belongs to.
pub(crate) fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
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

/// What a card is sorted by inside its column: where it was put, and — for cards off a board
/// written before they had a place, which all read as zero — the order they were created in.
fn place_of(metadata: &TaskMetadata) -> (u32, u64) {
    (metadata.position, metadata.created_at_unix)
}

/// Move a task to a column and to a place in it, renumbering that column from the top.
///
/// `position` is counted among the column's other cards, so it is where the card lands rather
/// than where it was aimed: a number past the end puts it at the bottom.
pub(crate) fn place_task(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    status: ColumnId,
    position: usize,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let board = store::read_board(&repo_path);
    if !board.has(&status) {
        bail!("{status} is not a column of this board");
    }

    let mut moved = store::read_task(&repo_path, task_id)?;
    release_a_finished_task(state, &board, &repo_path, task_id, &mut moved, &status);
    moved.status = status.clone();

    // The cards already in that column, in the order the board draws them.
    let mut column: Vec<(String, TaskMetadata)> = store::list_task_ids(&repo_path)?
        .into_iter()
        .filter(|other| other != task_id)
        .filter_map(|other| {
            let metadata = store::read_task(&repo_path, &other).ok()?;
            (metadata.status == status).then_some((other, metadata))
        })
        .collect();
    column.sort_by_key(|(_, metadata)| place_of(metadata));
    column.insert(position.min(column.len()), (task_id.to_string(), moved));

    for (index, (id, mut metadata)) in column.into_iter().enumerate() {
        let position = index as u32;
        // The card that moved is written whatever its number came out as — it is the one
        // whose column may have changed.
        if metadata.position == position && id != task_id {
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
/// readable — and made unique, because two columns sharing an id would be one column with two
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
            "there {} still {holding} card{} in this column — move {} out first",
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

fn view_of(state: &AppState, repo_path: &Path, task_id: &str, metadata: &TaskMetadata) -> TaskView {
    // Agent runs are the task's record and outlive the process; its shells are only ever the
    // ones open right now, so the two are listed from different places and merged by age.
    let mut resources: Vec<TaskResourceView> = metadata
        .resources
        .iter()
        .map(|resource| TaskResourceView {
            id: resource.id.clone(),
            kind: resource.kind,
            agent: resource.agent,
            label: resource.agent.label().to_lowercase(),
            running: resource
                .terminal_id
                .as_ref()
                .is_some_and(|terminal_id| state.terminals.is_live(terminal_id)),
            terminal_id: resource.terminal_id.clone(),
            resumable: agent_launch(resource.agent).is_some(),
            started_at_unix: resource.started_at_unix,
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
                label: "shell".to_string(),
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
        // Where the task's work happens, which is its own checkout once it has one. This is
        // what the card hands to a new shell and to `[start review]`.
        repo_path: worktrees::work_dir_of(repo_path, metadata)
            .display()
            .to_string(),
        worktree: worktrees::view_of(metadata),
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

pub(crate) fn create_task(
    state: &AppState,
    session_id: &str,
    request: &CreateTaskRequest,
) -> Result<TaskView> {
    let repo_path = repo_of(state, session_id)?;
    let task_id = store::create_task(&repo_path, &request.title, &request.status, request.joins)?;

    // The column remembers the agent this task was created with — including "none" — so the
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
                prompt: None,
            },
        )?;
    }

    let metadata = store::read_task(&repo_path, &task_id)?;
    Ok(view_of(state, &repo_path, &task_id, &metadata))
}

/// A finished task lets go of its shells, and of the checkout they were working in. Until then
/// the shells keep running with no tab open, which is what makes closing an agent's tab safe.
fn release_a_finished_task(
    state: &AppState,
    board: &BoardConfig,
    repo_path: &Path,
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
    worktrees::discard_quietly(repo_path, metadata);
}

pub(crate) fn delete_task(state: &AppState, session_id: &str, task_id: &str) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    state.terminals.remove_owned_by(task_id);
    // A deleted card has nowhere left to record its worktree, so the directory would be
    // unreachable rather than merely unused.
    if let Ok(mut metadata) = store::read_task(&repo_path, task_id) {
        worktrees::discard_quietly(&repo_path, &mut metadata);
    }
    store::delete_task(&repo_path, task_id)
}

/// What a card is marked with, set whole.
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
    };
    let launch = agent_launch(agent);
    // The work up front means the one-shot argv; nothing up front means a conversation opened
    // on the task and left waiting. Everything past this point is the same either way — a
    // finished one-shot run is resumed and attached to exactly like any other.
    let template = |launch: &AgentLaunch| match request.prompt {
        Some(_) => launch.one_shot,
        None => launch.start,
    };
    // Only an agent whose args name a session id has a run that can be resumed exactly.
    let agent_session_id = launch
        .filter(|launch| template(launch).iter().any(|arg| arg.contains("{session}")))
        .map(|_| store::new_uuid());

    // Named before the run starts, because the run writes its transcript under this name.
    let resource_id = store::new_uuid();
    let fillings = write_task_files(task_id, &repo_path, &metadata)?
        .with_session(agent_session_id.as_deref())
        .with_prompt(request.prompt.as_deref());
    let args = match launch {
        Some(launch) => fillings.fill_all(template(launch).iter()),
        None => Vec::new(),
    };
    let env = task_env(session_id, task_id, &repo_path);

    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: worktrees::work_dir_of(&repo_path, &metadata),
        program: (agent != AgentKind::None).then_some(agent),
        args,
        env,
        owner: Some(task_id.to_string()),
        type_ahead: type_ahead_for(&request, &metadata),
        // Only a run given its work up front keeps one: an interactive run is a conversation
        // someone is having, and its scrollback is where that lives.
        transcript: request
            .prompt
            .is_some()
            .then(|| store::run_log_path(&repo_path, task_id, &resource_id)),
    })?;

    // A shell is not written down: nothing survives its pty, so a record of one from a run of
    // moonreview that has ended is a card entry with nowhere to go. The registry lists the
    // ones that are open, and that is the whole life of a shell.
    if request.kind == TaskResourceKind::Agent {
        metadata.resources.push(TaskResource {
            id: resource_id,
            kind: request.kind,
            agent,
            terminal_id: Some(terminal_id.clone()),
            agent_session_id,
            started_at_unix: store::now_unix(),
        });
        store::write_task(&repo_path, task_id, &metadata)?;
    }

    Ok(terminal_id)
}

/// Open a window that works the board, running the given agent.
pub(crate) fn start_autopilot(
    state: &AppState,
    session_id: &str,
    agent: AgentKind,
) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    if !agent_is_available(state.agent_availability, agent) {
        bail!("{} is not installed here", agent.label());
    }
    autopilot::ensure_brief(&repo_path)?;

    let prompt = autopilot::opening_prompt(&repo_path);
    let (args, types_its_prompt) = autopilot::launch_args(agent, &repo_path, &prompt);

    state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program: Some(agent),
        args,
        env: Vec::new(),
        owner: None,
        // Only for the agent that cannot be handed its opening message on the command line.
        // It is sent, unlike everywhere else, because a window nobody is going to type into is
        // one a person opened in order for it to start — the Enter was theirs, further up.
        type_ahead: types_its_prompt.then(|| crate::terminal::TypeAhead {
            text: prompt,
            submit: true,
        }),
        transcript: None,
    })
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
        bail!("a shell cannot be resumed — open a new one");
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
    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: worktrees::work_dir_of(&repo_path, &metadata),
        program: Some(resource.agent),
        args: fillings.fill_all(template.iter()),
        env: task_env(session_id, task_id, &repo_path),
        owner: Some(task_id.to_string()),
        // A resumed run is being picked up where it left off, and it was told the title when
        // it started; typing it again would be typing over whatever it is in the middle of.
        type_ahead: None,
        transcript: None,
    })?;

    metadata.resources[at].terminal_id = Some(terminal_id.clone());
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
}

/// Put a session an agent already has on a task, and open a shell resumed on it.
///
/// This is how a task is pointed back at real work when its recorded session id stopped
/// meaning anything — the id here was read off the agent's own records, so opening it is
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
    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: worktrees::work_dir_of(&repo_path, &metadata),
        program: Some(request.agent),
        args: fillings.fill_all(launch.attach.iter()),
        env: task_env(session_id, task_id, &repo_path),
        owner: Some(task_id.to_string()),
        // The session being attached is already under way; typing the title at it would be
        // typing over whatever it is in the middle of.
        type_ahead: None,
        transcript: None,
    })?;

    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: TaskResourceKind::Agent,
        agent: request.agent,
        terminal_id: Some(terminal_id.clone()),
        agent_session_id: Some(agent_session_id.to_string()),
        started_at_unix: store::now_unix(),
    });
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
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
/// is what everything else — shells, agent sessions, whatever an agent wrote — points at.
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

    /// The work, for a run that was given it up front. A run that was not has no value for
    /// `{prompt}`, which is what makes the one-shot arguments unfillable for it.
    fn with_prompt(mut self, prompt: Option<&str>) -> Self {
        if let Some(prompt) = prompt {
            self.values.push(("{prompt}", prompt.to_string()));
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

    /// Fill an argument list in, dropping any argument that cannot be filled — along with the
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

/// What is written into a run's box a moment after it starts, if anything.
fn type_ahead_for(
    request: &StartResourceRequest,
    metadata: &TaskMetadata,
) -> Option<crate::terminal::TypeAhead> {
    if request.kind != TaskResourceKind::Agent || request.prompt.is_some() {
        return None;
    }
    Some(crate::terminal::TypeAhead {
        text: metadata.title.clone(),
        submit: false,
    })
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
    // Only made, never rewritten — it is the task's own record, unlike the brief.
    store::ensure_notes_file(repo_path, task_id)?;

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

    /// Claude is given the session id and the brief — and no prompt, so it comes up knowing
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

        assert!(args.is_empty(), "expected no dangling --resume: {args:?}");
    }

    /// Attaching opens the exact session that was picked, whichever agent it belongs to.
    #[test]
    fn an_attached_session_is_opened_by_its_own_id() {
        let session = "11111111-2222-4333-8444-555555555555";
        let expected: &[(AgentKind, &[&str])] = &[
            (AgentKind::Claude, &["--resume", session]),
            (AgentKind::Codex, &["resume", session]),
            (AgentKind::OpenCode, &["--session", session]),
        ];

        for (kind, args) in expected {
            let launch = agent_launch(*kind).expect("expected the agent to be launchable");

            assert_eq!(
                fillings()
                    .with_session(Some(session))
                    .fill_all(launch.attach.iter()),
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
            tags: Vec::new(),
            worktree: None,
            resources: vec![TaskResource {
                id: "resource".to_string(),
                kind: TaskResourceKind::Agent,
                agent: AgentKind::Claude,
                // No server in this test, so no shell of this name is live.
                terminal_id: Some("terminal-gone".to_string()),
                agent_session_id: None,
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
            tags: Vec::new(),
            worktree: None,
            resources: vec![TaskResource {
                id: "resource".to_string(),
                kind: TaskResourceKind::Agent,
                agent: AgentKind::Claude,
                terminal_id: Some("terminal-gone".to_string()),
                agent_session_id: None,
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
