//! Everything both frontends do to the board.
//!
//! Like [`crate::service`], this is synchronous and takes `&AppState`, so the native window
//! calls it directly and the axum routes are a thin skin over the same functions.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    api::{AgentKind, AppState},
    git::agent_is_available,
    moontasks::{
        CreateTaskRequest, OPENCODE_CONFIG_FILE_NAME, StartResourceRequest, TaskResourceView,
        TaskView, agent_launch,
        store::{
            self, MCP_CONFIG_FILE_NAME, TaskMetadata, TaskResource, TaskResourceKind, TaskStatus,
        },
    },
    terminal::TerminalSpec,
};

/// The repo a session's board belongs to.
fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

/// Every task on the board, with what each one has running right now.
///
/// Reading the board is also when it catches up with reality: an agent that has finished, or
/// one whose shell died with a previous run of the server, moves its task on to local review.
/// That is what makes the board behave the same whether the window was open the whole time or
/// is being opened again after a restart.
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
        tasks.push(view_of(state, &repo_path, &task_id, &metadata));
    }

    tasks.sort_by_key(|task| task.created_at_unix);
    Ok(tasks)
}

/// Bring a task's record in line with the shells the server actually has, and return whether
/// anything changed.
fn reconcile(state: &AppState, metadata: &mut TaskMetadata) -> bool {
    let mut changed = false;
    let mut an_agent_finished = false;

    for resource in &mut metadata.resources {
        let Some(terminal_id) = resource.terminal_id.clone() else {
            continue;
        };
        if state.terminals.is_live(&terminal_id) {
            continue;
        }
        resource.terminal_id = None;
        changed = true;
        if resource.kind == TaskResourceKind::Agent {
            an_agent_finished = true;
        }
    }

    // An agent that has stopped working has, as far as the board is concerned, produced
    // something to look at.
    if an_agent_finished && metadata.status == TaskStatus::InProgress {
        metadata.status = TaskStatus::InLocalReview;
        changed = true;
    }
    changed
}

fn view_of(
    state: &AppState,
    repo_path: &Path,
    task_id: &str,
    metadata: &TaskMetadata,
) -> TaskView {
    let resources = metadata
        .resources
        .iter()
        .map(|resource| TaskResourceView {
            id: resource.id.clone(),
            kind: resource.kind,
            agent: resource.agent,
            label: label_of(resource),
            running: resource
                .terminal_id
                .as_ref()
                .is_some_and(|terminal_id| state.terminals.is_live(terminal_id)),
            terminal_id: resource.terminal_id.clone(),
            resumable: resource.kind == TaskResourceKind::Agent
                && agent_launch(resource.agent).is_some(),
            started_at_unix: resource.started_at_unix,
        })
        .collect();

    TaskView {
        id: task_id.to_string(),
        title: metadata.title.clone(),
        status: metadata.status,
        created_at_unix: metadata.created_at_unix,
        dir_path: store::tasks_root(repo_path)
            .join(task_id)
            .display()
            .to_string(),
        repo_path: repo_path.display().to_string(),
        resources,
    }
}

fn label_of(resource: &TaskResource) -> String {
    match resource.kind {
        TaskResourceKind::Shell => "shell".to_string(),
        TaskResourceKind::Agent => resource.agent.label().to_lowercase(),
    }
}

pub(crate) fn create_task(
    state: &AppState,
    session_id: &str,
    request: &CreateTaskRequest,
) -> Result<TaskView> {
    let repo_path = repo_of(state, session_id)?;
    let task_id = store::create_task(&repo_path, &request.title)?;

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

pub(crate) fn set_task_status(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    status: TaskStatus,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    metadata.status = status;

    // A finished task lets go of its shells. Until then they keep running with no tab open,
    // which is what makes closing an agent's tab safe.
    if status == TaskStatus::Done {
        state.terminals.remove_owned_by(task_id);
        for resource in &mut metadata.resources {
            resource.terminal_id = None;
        }
    }

    store::write_task(&repo_path, task_id, &metadata)
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
    };
    let launch = agent_launch(agent);
    // Only an agent whose start args name a session id has a run that can be resumed exactly.
    let agent_session_id = launch
        .filter(|launch| launch.start.iter().any(|arg| arg.contains("{session}")))
        .map(|_| store::new_uuid());

    let fillings = write_task_files(session_id, task_id, &repo_path, &metadata)?
        .with_session(agent_session_id.as_deref());
    let (args, mut env) = match launch {
        Some(launch) => (
            fillings.fill_all(launch.mcp.iter().chain(launch.start.iter())),
            fillings.fill_env(launch.env),
        ),
        None => (Vec::new(), Vec::new()),
    };
    env.extend(task_env(session_id, task_id, &repo_path));

    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program: (agent != AgentKind::None).then_some(agent),
        args,
        env,
        owner: Some(task_id.to_string()),
    })?;

    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: request.kind,
        agent,
        terminal_id: Some(terminal_id.clone()),
        agent_session_id,
        started_at_unix: store::now_unix(),
    });
    // Work has started, so the task is no longer waiting to be picked up.
    if request.kind == TaskResourceKind::Agent && metadata.status == TaskStatus::Todo {
        metadata.status = TaskStatus::InProgress;
    }
    store::write_task(&repo_path, task_id, &metadata)?;

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
        bail!("a shell cannot be resumed — open a new one");
    };
    if !agent_is_available(state.agent_availability, resource.agent) {
        bail!("{} is not installed here", resource.agent.label());
    }

    let fillings = write_task_files(session_id, task_id, &repo_path, &metadata)?
        .with_session(resource.agent_session_id.as_deref());
    let mut env = fillings.fill_env(launch.env);
    env.extend(task_env(session_id, task_id, &repo_path));

    let terminal_id = state.terminals.spawn(TerminalSpec {
        cwd: repo_path.clone(),
        program: Some(resource.agent),
        args: fillings.fill_all(launch.mcp.iter().chain(launch.resume.iter())),
        env,
        owner: Some(task_id.to_string()),
    })?;

    metadata.resources[at].terminal_id = Some(terminal_id.clone());
    if metadata.status == TaskStatus::Todo {
        metadata.status = TaskStatus::InProgress;
    }
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
}

/// End one of a task's shells, leaving the run recorded so it can be resumed.
pub(crate) fn stop_resource(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    resource_id: &str,
) -> Result<()> {
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

    fn fill_env(&self, template: &[(&'static str, &'static str)]) -> Vec<(String, String)> {
        template
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), self.fill(value)?)))
            .collect()
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
/// The files are rewritten on every start rather than once at creation: they carry the review
/// session and the address this run of moonreview is answering on, and neither survives a
/// restart.
fn write_task_files(
    session_id: &str,
    task_id: &str,
    repo_path: &Path,
    metadata: &TaskMetadata,
) -> Result<Fillings> {
    let executable = std::env::current_exe().context("failed to locate moonreview itself")?;
    let executable = executable.display().to_string();
    let dir = store::task_dir(repo_path, task_id)?;
    let env = task_env(session_id, task_id, repo_path);

    // Claude's shape, which is also the one a person would recognise.
    let mcp_json = write_file(
        &dir,
        MCP_CONFIG_FILE_NAME,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "mcpServers": {
                    "moontasks": {
                        "command": executable,
                        "args": ["mcp"],
                        "env": env.iter().cloned().collect::<BTreeMap<_, _>>(),
                    }
                }
            }))?
        ),
    )?;

    // OpenCode's, which it reads whole rather than merging, so this is a config in its own
    // right rather than an MCP fragment.
    let mcp_opencode = write_file(
        &dir,
        OPENCODE_CONFIG_FILE_NAME,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    "moontasks": {
                        "type": "local",
                        "command": [executable, "mcp"],
                        "enabled": true,
                        "environment": env.iter().cloned().collect::<BTreeMap<_, _>>(),
                    }
                }
            }))?
        ),
    )?;

    let brief = super::brief_for(&metadata.title, &dir.display().to_string());
    write_file(&dir, super::BRIEF_FILE_NAME, &format!("{brief}\n"))?;

    Ok(Fillings {
        values: vec![
            ("{exe}", toml_string(&executable)),
            ("{mcp_json}", mcp_json.display().to_string()),
            ("{mcp_opencode}", mcp_opencode.display().to_string()),
            ("{mcp_env_toml}", toml_inline_table(&env)),
            ("{prompt}", metadata.title.clone()),
            (
                "{briefed_prompt}",
                format!("{brief}\n\nStart on this task now."),
            ),
            ("{brief}", brief),
        ],
    })
}

fn write_file(dir: &Path, name: &str, contents: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    std::fs::write(&path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// A TOML string, for a config value passed on a command line.
fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A TOML inline table of the same, which is how one `-c` override carries a whole map.
fn toml_inline_table(entries: &[(String, String)]) -> String {
    let pairs: Vec<String> = entries
        .iter()
        .map(|(name, value)| format!("{name} = {}", toml_string(value)))
        .collect();
    format!("{{ {} }}", pairs.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's fillings, without the files a real one writes.
    fn fillings() -> Fillings {
        Fillings {
            values: vec![
                ("{exe}", "\"/bin/moonreview\"".to_string()),
                ("{mcp_json}", "/repo/.moontasks/task/mcp.json".to_string()),
                (
                    "{mcp_opencode}",
                    "/repo/.moontasks/task/opencode.json".to_string(),
                ),
                ("{mcp_env_toml}", "{ MOONREVIEW_TASK_ID = \"task\" }".to_string()),
                ("{prompt}", "Fix the login page".to_string()),
                ("{briefed_prompt}", "the brief, then the work".to_string()),
                ("{brief}", "the brief".to_string()),
            ],
        }
    }

    /// Claude takes the session id, the brief and the work, all on the way in.
    #[test]
    fn an_agent_is_told_the_task_the_brief_and_where_its_mcp_server_is() {
        let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

        let args = fillings()
            .with_session(Some("11111111-2222-4333-8444-555555555555"))
            .fill_all(launch.mcp.iter().chain(launch.start.iter()));

        assert_eq!(
            args,
            [
                "--mcp-config",
                "/repo/.moontasks/task/mcp.json",
                "--session-id",
                "11111111-2222-4333-8444-555555555555",
                "--append-system-prompt",
                "the brief",
                "Fix the login page",
            ]
        );
    }

    #[test]
    fn a_missing_session_id_takes_the_flag_in_front_of_it_with_it() {
        let launch = agent_launch(AgentKind::Claude).expect("expected claude to be launchable");

        let args = fillings().fill_all(launch.resume.iter());

        assert!(args.is_empty(), "expected no dangling --resume: {args:?}");
    }

    #[test]
    fn an_agent_resumed_by_its_own_reckoning_needs_no_session_id() {
        let launch = agent_launch(AgentKind::Codex).expect("expected codex to be launchable");

        let args = fillings().fill_all(launch.resume.iter());

        assert_eq!(args, ["resume", "--last"]);
    }

    /// Codex has no per-run config file, so its MCP server arrives as three `-c` overrides,
    /// each one argument of `key=value`.
    #[test]
    fn codex_is_given_its_mcp_server_as_config_overrides() {
        let launch = agent_launch(AgentKind::Codex).expect("expected codex to be launchable");

        let args = fillings().fill_all(launch.mcp.iter().chain(launch.start.iter()));

        assert_eq!(
            args,
            [
                "-c",
                "mcp_servers.moontasks.command=\"/bin/moonreview\"",
                "-c",
                "mcp_servers.moontasks.args=[\"mcp\"]",
                "-c",
                "mcp_servers.moontasks.env={ MOONREVIEW_TASK_ID = \"task\" }",
                "the brief, then the work",
            ]
        );
    }

    /// OpenCode reads its whole config from the file its environment names, so that is where
    /// its MCP server comes from rather than from an argument.
    #[test]
    fn opencode_is_pointed_at_the_config_in_its_task_folder() {
        let launch = agent_launch(AgentKind::OpenCode).expect("expected opencode to launch");
        let fillings = fillings();

        assert_eq!(
            fillings.fill_env(launch.env),
            [(
                "OPENCODE_CONFIG".to_string(),
                "/repo/.moontasks/task/opencode.json".to_string()
            )]
        );
        assert_eq!(
            fillings.fill_all(launch.start.iter()),
            ["--prompt", "the brief, then the work"]
        );
    }

    /// The brief is the whole of why an agent reaches for the MCP server at all.
    #[test]
    fn the_brief_names_the_task_and_the_tool_that_reports_it_finished() {
        let brief = crate::moontasks::brief_for("Fix the login page", "/repo/.moontasks/task");

        assert!(brief.contains("Fix the login page"));
        assert!(brief.contains("/repo/.moontasks/task"));
        assert!(brief.contains("moontasks_set_status"));
        assert!(brief.contains("in_local_review"));
    }

    #[test]
    fn a_path_with_a_quote_in_it_stays_one_toml_string() {
        assert_eq!(toml_string(r#"/a "b"/c"#), r#""/a \"b\"/c""#);
        assert_eq!(toml_string(r"C:\tools"), r#""C:\\tools""#);
    }

    #[test]
    fn a_finished_agent_moves_its_task_to_local_review() {
        let mut metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: TaskStatus::InProgress,
            created_at_unix: 0,
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

        assert_eq!(metadata.status, TaskStatus::InLocalReview);
        assert_eq!(metadata.resources[0].terminal_id, None);
        assert!(
            !reconcile(&state, &mut metadata),
            "a settled task changes nothing on the next read"
        );
    }

    #[test]
    fn a_task_already_in_review_is_left_where_the_user_put_it() {
        let mut metadata = TaskMetadata {
            title: "Fix the login page".to_string(),
            status: TaskStatus::InRemoteReview,
            created_at_unix: 0,
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

        assert!(reconcile(&state, &mut metadata), "the shell is still recorded");

        assert_eq!(metadata.status, TaskStatus::InRemoteReview);
    }
}
