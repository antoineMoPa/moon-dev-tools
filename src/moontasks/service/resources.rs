//! What runs on a task: agents and shells started, resumed, attached, stopped and deleted, and
//! the files and environment a run is started with.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::{
    agent::agent_is_available,
    api::{AgentKind, AppState},
    moontasks::{
        AttachResourceRequest, StartFolder, StartResourceRequest, agent_launch,
        store::{self, TaskMetadata, TaskResource, TaskResourceKind},
    },
    terminal::{TerminalProgram, TerminalSpec},
};

use super::repo_of;

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
        TaskResourceKind::Visualization => {
            bail!("a visualization is kept on a task by the run that showed it, not started")
        }
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
    let mut env = task_env(session_id, task_id, &repo_path);
    if let Some(launch) = launch {
        env.extend(fillings.fill_env(launch.env));
    }
    let program = TerminalProgram::of_agent(Some(agent));
    let name =
        crate::terminal::name_for_new_shell(state, &repo_path, Some(&metadata.title), &program)?;
    let task_dir = store::task_dir(&repo_path, task_id)?;
    let cwd = match request.opens_in {
        StartFolder::Repo => repo_path.clone(),
        StartFolder::TaskFolder => task_dir.clone(),
    };

    // An agent comes up with the card's title already written in its box, waiting on the
    // Enter that sends it. It is still the person who starts the work - the title is a card's
    // name and rarely the whole of what is wanted - but the common case, where it is, is one
    // keystroke away. A task's plain shell gets nothing typed at it.
    let type_ahead = (request.kind == TaskResourceKind::Agent).then(|| metadata.title.clone());

    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd,
        program,
        args,
        env,
        owner: Some(task_id.to_string()),
        name: Some(name.clone()),
        type_ahead,
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
        None => {
            crate::terminal::name_for_new_shell(state, &repo_path, Some(&metadata.title), &program)?
        }
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
pub(super) struct Fillings {
    pub(super) values: Vec<(&'static str, String)>,
}

impl Fillings {
    /// The session id is only known once it has been decided there will be one, which is
    /// after everything else here has been worked out.
    pub(super) fn with_session(mut self, agent_session_id: Option<&str>) -> Self {
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

    /// Fill an agent's environment in, leaving unset any variable whose value has a
    /// placeholder with nothing to fill it.
    pub(super) fn fill_env(
        &self,
        template: &'static [(&'static str, &'static str)],
    ) -> Vec<(String, String)> {
        template
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), self.fill(value)?)))
            .collect()
    }

    /// Fill an argument list in, dropping any argument that cannot be filled - along with the
    /// flag in front of it, which would otherwise be left dangling.
    pub(super) fn fill_all<'a>(
        &self,
        template: impl Iterator<Item = &'a &'static str>,
    ) -> Vec<String> {
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
pub(in crate::moontasks) fn task_env(
    session_id: &str,
    task_id: &str,
    repo_path: &Path,
) -> Vec<(String, String)> {
    vec![
        (
            crate::moontasks::TASK_ID_ENV_VAR.to_string(),
            task_id.to_string(),
        ),
        (
            crate::moontasks::TASK_DIR_ENV_VAR.to_string(),
            store::tasks_root(repo_path)
                .join(task_id)
                .display()
                .to_string(),
        ),
        (
            crate::moontasks::SESSION_ID_ENV_VAR.to_string(),
            session_id.to_string(),
        ),
        (
            crate::moontasks::SERVER_URL_ENV_VAR.to_string(),
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

    let brief = crate::moontasks::brief_for(&metadata.title, &dir.display().to_string());
    let brief_path = dir.join(crate::moontasks::BRIEF_FILE_NAME);
    std::fs::write(&brief_path, format!("{brief}\n"))
        .with_context(|| format!("failed to write {}", brief_path.display()))?;

    // The brief points the agent at notes.md, so by the time one reads it the file is there.
    // Only made, never rewritten - it is the task's own record, unlike the brief.
    store::ensure_notes_file(repo_path, task_id)?;

    // The format the brief sends the agent to read, written beside it for the same reason and
    // rewritten for the same reason: it is ours, not the task's, and a task started today should
    // be reading today's.
    let path = dir.join(crate::moontasks::REVIEW_REQUEST_BRIEF_FILE_NAME);
    std::fs::write(
        &path,
        crate::moontasks::review_request::REVIEW_REQUEST_BRIEF,
    )
    .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(Fillings {
        values: vec![
            ("{brief}", brief),
            ("{brief_file}", brief_path.display().to_string()),
        ],
    })
}
