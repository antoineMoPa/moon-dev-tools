//! Starting the agent that writes the explanation, in a shell of the task.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::ExplainRequest;
use crate::{
    agent::agent_is_available,
    api::{AgentKind, AppState},
    moontasks::{service, store, store::TaskResource, store::TaskResourceKind},
    terminal::{TerminalProgram, TerminalSpec},
};

/// What the agent is asked for, written where it can be read and run again by hand.
pub(crate) const PROMPT_FILE_NAME: &str = "change_explanation.prompt.md";

/// What the run's shell is called, after the task's title: `write the parser explain - 1`.
const SHELL_LABEL: &str = "explain";

/// How each agent is run for an answer rather than a conversation: on `{model}`, its
/// permission prompts off, and the prompt read from `{prompt}`, the file it was written to.
/// Two read it from stdin; OpenCode takes it as an argument.
///
/// The model is named rather than left to the agent's default, which is whatever the person
/// has picked for their own conversations - a slow, careful one, as often as not - where this
/// is a page of bullets that a fast one writes as well.
///
/// The run is still a conversation afterwards: `resume` on the card opens the agent on it,
/// which is the way to tell it what the explanation got wrong. Claude is given `{session}`,
/// a session id of moontasks' choosing, so the run can be resumed exactly; the other two are
/// resumed by their own reckoning of the last session, the way a run of theirs from `[start]`
/// is - see [`crate::moontasks::AGENT_LAUNCHES`].
const HEADLESS_COMMANDS: &[HeadlessCommand] = &[
    HeadlessCommand {
        kind: AgentKind::Claude,
        model: "sonnet",
        template: "claude -p --model {model} --permission-mode bypassPermissions \
                   --session-id {session} < {prompt}",
    },
    HeadlessCommand {
        kind: AgentKind::Codex,
        model: "gpt-5.6-terra",
        template: "codex exec -m {model} --full-auto - < {prompt}",
    },
    HeadlessCommand {
        kind: AgentKind::OpenCode,
        model: "openai/gpt-5.6-terra",
        template: "opencode run --model {model} --dangerously-skip-permissions \"$(cat {prompt})\"",
    },
];

struct HeadlessCommand {
    kind: AgentKind,
    model: &'static str,
    template: &'static str,
}

/// The command, and the session id it was given if its template names one.
fn headless_command(agent: AgentKind, prompt_path: &Path) -> Result<(String, Option<String>)> {
    let headless = HEADLESS_COMMANDS
        .iter()
        .find(|command| command.kind == agent)
        .with_context(|| format!("{} cannot explain a change", agent.label()))?;
    let session = headless
        .template
        .contains("{session}")
        .then(store::new_uuid);
    let mut command = headless
        .template
        .replace("{model}", headless.model)
        .replace(
            "{prompt}",
            &crate::committing::single_quoted(&prompt_path.display().to_string()),
        );
    if let Some(session) = &session {
        command = command.replace("{session}", session);
    }
    Ok((command, session))
}

/// Start explaining what is changed, for this task, in a shell of the task, and answer with
/// that shell. The run is written on the task the way any agent run is.
pub(crate) fn start_explanation(
    state: &AppState,
    session_id: &str,
    task_id: &str,
    request: ExplainRequest,
) -> Result<String> {
    if request.agent == AgentKind::None {
        bail!("no agent is picked to explain it: pick one in the review's selector");
    }
    if !agent_is_available(state.agent_availability, request.agent) {
        bail!("{} is not installed here", request.agent.label());
    }
    let repo_path = service::repo_of(state, session_id)?;
    let mut metadata = store::read_task(&repo_path, task_id)?;
    let task_dir = store::task_dir(&repo_path, task_id)?;
    // The prompt sends the agent to the task folder for the task, which is its notes.
    store::ensure_notes_file(&repo_path, task_id)?;
    let prompt_path = task_dir.join(PROMPT_FILE_NAME);
    std::fs::write(&prompt_path, prompt(&repo_path, &task_dir))
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;

    let (command, agent_session_id) = headless_command(request.agent, &prompt_path)?;
    let name = crate::terminal::name_for_new_shell_called(
        state,
        &repo_path,
        Some(&metadata.title),
        SHELL_LABEL,
    )?;
    let terminal_id = state.terminals.spawn(TerminalSpec {
        // In the task folder, which the prompt says is where it is: what it writes lands
        // there, and a `git diff` run where it stands is run on the task folder rather than
        // the repo - which is what the prompt's naming of the repo is there to head off.
        cwd: task_dir,
        program: TerminalProgram::LoginShell,
        args: vec!["-c".to_string(), run_script(request.agent, &command)],
        env: service::task_env(session_id, task_id, &repo_path),
        owner: Some(task_id.to_string()),
        name: Some(name.clone()),
        type_ahead: None,
    })?;

    metadata.resources.push(TaskResource {
        id: store::new_uuid(),
        kind: TaskResourceKind::Agent,
        agent: request.agent,
        file_path: None,
        terminal_id: Some(terminal_id.clone()),
        terminal_owner: Some(std::process::id()),
        agent_session_id,
        name: Some(name),
        started_at_unix: store::now_unix(),
    });
    store::write_task(&repo_path, task_id, &metadata)?;

    Ok(terminal_id)
}

/// The script the run's shell is started with, rather than typed into - for the same reasons
/// as a commit run's: nothing of it lands in the history of the person's own shell, and the
/// output stays on screen above a shell to carry on in.
fn run_script(agent: AgentKind, command: &str) -> String {
    let shell = crate::committing::single_quoted(&crate::shell_path::login_shell());
    [
        format!(
            "printf '%s\\n' 'explaining the change with {} - the PDF opens when it is done'",
            agent.label()
        ),
        command.to_string(),
        format!("exec {shell} -l"),
    ]
    .join("\n")
}

/// What the agent is asked for. The repo and the task folder are both named by their whole
/// paths: an agent told only where it stands takes the task folder for the project and runs
/// `git diff` there, and one told "the current folder" answers from wherever it takes its
/// project to be. What the files are called and how to read the diff are the agent's to work
/// out.
fn prompt(repo_path: &Path, task_dir: &Path) -> String {
    format!(
        "Write a typst file summarizing the current task + diff. Root repo is at {repo} - \
         check it and any relevant submodules. Task path is at {task_dir}, where you are \
         currently. Compile the pdf and pop it open using `open` command. Keep the text \
         concise, bullet-point style, short sentences for a busy engineer. Include code \
         samples of important changes.\n",
        repo = repo_path.display(),
        task_dir = task_dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_is_told_the_repo_and_the_task_folder_by_their_whole_paths() {
        let asked = prompt(Path::new("/repo"), Path::new("/repo/.moontasks/a-task"));

        assert!(asked.contains("Root repo is at /repo "), "{asked}");
        assert!(
            asked.contains("Task path is at /repo/.moontasks/a-task,"),
            "{asked}"
        );
        assert!(asked.contains("`open`"), "{asked}");
        assert_eq!(
            asked.lines().count(),
            1,
            "a few sentences on one line:\n{asked}"
        );
    }

    #[test]
    fn every_agent_a_task_can_start_can_explain_and_is_asked_headless() {
        let prompt_path = Path::new("/repo/.moontasks/a task/change_explanation.prompt.md");
        for launch in crate::moontasks::AGENT_LAUNCHES {
            let (command, session) =
                headless_command(launch.kind, prompt_path).expect("a headless command");
            assert!(
                command.contains("'/repo/.moontasks/a task/change_explanation.prompt.md'"),
                "{command}"
            );
            assert!(!command.contains(['{', '}']), "{command}");
            assert!(
                command.contains(" --model ") || command.contains(" -m "),
                "{command}"
            );
            // The same rule as a run from `[start]`: an agent whose start names a session id
            // is given one here too, so `resume` opens this very run.
            let starts_with_a_session = launch.start.iter().any(|arg| arg.contains("{session}"));
            assert_eq!(
                session.is_some(),
                starts_with_a_session,
                "{}",
                launch.kind.label()
            );
            if let Some(session) = session {
                assert!(command.contains(&session), "{command}");
            }
        }
        assert!(headless_command(AgentKind::None, prompt_path).is_err());
    }

    #[test]
    fn the_run_says_what_it_is_and_leaves_a_shell_behind() {
        let script = run_script(AgentKind::Claude, "claude -p < 'prompt'");
        let lines: Vec<&str> = script.lines().collect();
        assert!(
            lines[0].contains("explaining the change with Claude"),
            "{script}"
        );
        assert_eq!(lines[1], "claude -p < 'prompt'");
        assert!(lines[2].starts_with("exec "), "{script}");
        assert!(lines[2].ends_with(" -l"), "{script}");
    }

    /// The run on the card: written on the task with its shell, named after what it does, and
    /// there to resume - and the prompt is in the task folder for a person to read.
    ///
    /// Ignored because it starts a real agent, if only for a moment - run it with `--ignored`
    /// on a machine that has one.
    #[test]
    #[ignore]
    fn an_explanation_is_a_run_of_the_task_in_a_shell_named_for_it() {
        let repo =
            std::env::temp_dir().join(format!("moonreview-explainer-{}-run", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).expect("failed to make the scratch repo");
        crate::git::run_git_no_output(&repo, &["init"]).expect("failed to init the scratch repo");
        let state = crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(
            std::time::Instant::now(),
        )));
        let session_id = crate::service::open_session(
            &state,
            crate::api::OpenSessionRequest {
                repo_path: repo.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
        )
        .expect("failed to open the session")
        .session_id;
        let task_id = store::create_task(
            &repo,
            "Write the parser",
            &crate::moontasks::ColumnId::new("todo"),
            crate::moontasks::ColumnEnd::Top,
        )
        .expect("failed to create the task");

        let refused = start_explanation(
            &state,
            &session_id,
            &task_id,
            ExplainRequest {
                agent: AgentKind::None,
            },
        )
        .expect_err("no agent, no explanation");
        assert!(refused.to_string().contains("selector"), "{refused}");

        // Whichever agent this machine has; the shell is a login shell either way, and what
        // it runs is not waited for here.
        let Some(agent) = [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode]
            .into_iter()
            .find(|agent| agent_is_available(state.agent_availability, *agent))
        else {
            eprintln!("no agent installed here, so the run itself is not checked");
            let _ = std::fs::remove_dir_all(&repo);
            return;
        };
        let terminal_id =
            start_explanation(&state, &session_id, &task_id, ExplainRequest { agent })
                .expect("the run starts");

        let task = service::list_tasks(&state, &session_id)
            .expect("the board reads")
            .into_iter()
            .find(|task| task.id == task_id)
            .expect("the task is on the board");
        let run = task
            .resources
            .iter()
            .find(|resource| resource.terminal_id.as_deref() == Some(&terminal_id))
            .expect("the run is on the card");
        assert_eq!(run.kind, TaskResourceKind::Agent);
        assert_eq!(run.agent, agent);
        assert_eq!(run.label, "Write the parser explain - 1");
        assert!(run.running);
        assert!(run.resumable, "the run is a conversation to come back to");
        let prompt = std::fs::read_to_string(
            store::task_dir(&repo, &task_id)
                .unwrap()
                .join(PROMPT_FILE_NAME),
        )
        .expect("the prompt is in the task folder");
        assert!(prompt.contains(&task_id), "{prompt}");

        state.terminals.remove(&terminal_id);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
