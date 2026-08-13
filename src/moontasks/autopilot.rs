//! The window that works the board.

use std::path::Path;

use anyhow::{Context, Result};

use crate::{
    api::AgentKind,
    moontasks::{AUTOPILOT_TAG, store},
};

/// What the autopilot window is told, written into the board folder the first time one opens.
pub(crate) const BRIEF_FILE_NAME: &str = "autopilot.md";

/// The starting text of `.moontasks/autopilot.md`.
const DEFAULT_BRIEF: &str = "\
# Autopilot

You are the manager of this board, not a worker on it. You decide what should happen and start
runs to do it; you do not edit the repo yourself.

Work one card at a time.

1. `list_tasks`. Take the topmost card tagged `autopilot` that has no run going. If there is
   none, say so and stop — do not go looking for other work.
2. `read_task_notes` on it. The notes are the brief; the title is only a name.
3. `create_worktree` for it, so the run works clear of the person's own checkout.
4. `run_task_agent` with a prompt that is the whole job: what to change, what done looks like,
   and to commit on the branch when it is done. The run cannot ask you anything, so leave
   nothing to be asked.
5. `await_task_run` on it, and keep calling it until it says the run has finished. It waits
   for you. Do not end your turn while a run of yours is going — nobody is here to restart you,
   and a card left mid-run is a card nobody is watching.
6. Read what the run actually said, rather than assuming it worked.
7. If it did the work: `review`. That puts the branch in the repo for the person to QA and
   commit, and it is where your part ends — leave the card alone afterwards.
8. If it did not: `append_task_notes` saying plainly what happened, `move_task` the card
   somewhere a person will see it, and go back to step 1.

Then go back to step 1 for the next card, and keep going until there are none left.

Things to know:

- `review` refuses if the run left uncommitted work, or if the repo has local changes. Read the
  refusal and act on it. Do not retry it unchanged.
- You cannot create cards, and you cannot add or remove the `autopilot` tag. Both are the
  person's. Say what happened with the card's column and its notes instead.
- If you are unsure whether a card is really finished, it is not. Write what you saw in the
  notes and leave it for a person.
";

/// The opening message the window is started with, which is what sets the loop going.
pub(crate) fn opening_prompt(repo_path: &Path) -> String {
    format!(
        "You are the autopilot for the moontasks board in {}. Read {} and follow it. \
         The board's tools are attached to this session — start with list_tasks. \
         Work the cards tagged `{AUTOPILOT_TAG}` one at a time, and stop when there are none.",
        repo_path.display(),
        brief_path(repo_path).display(),
    )
}

pub(crate) fn brief_path(repo_path: &Path) -> std::path::PathBuf {
    store::tasks_root(repo_path).join(BRIEF_FILE_NAME)
}

/// Make sure the board has an autopilot brief, without touching one that is already there.
pub(crate) fn ensure_brief(repo_path: &Path) -> Result<()> {
    let path = brief_path(repo_path);
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, DEFAULT_BRIEF)
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Every board tool, by the name the agent sees it under.
const ALLOWED_TOOLS: &str = "\
mcp__moontasks__list_tasks mcp__moontasks__list_columns mcp__moontasks__move_task \
mcp__moontasks__add_task_tag mcp__moontasks__remove_task_tag mcp__moontasks__read_task_notes \
mcp__moontasks__append_task_notes mcp__moontasks__create_worktree \
mcp__moontasks__discard_worktree mcp__moontasks__run_task_agent mcp__moontasks__await_task_run \
mcp__moontasks__read_task_run \
mcp__moontasks__stop_task_agents mcp__moontasks__repo_status mcp__moontasks__review";

/// How the board's tools are attached to an autopilot window, per agent.
struct AutopilotLaunch {
    kind: AgentKind,
    args: &'static [&'static str],
}

const AUTOPILOT_LAUNCHES: &[AutopilotLaunch] = &[
    AutopilotLaunch {
        kind: AgentKind::Claude,
        args: &[
            "{prompt}",
            // Only this board's tools, and only the ones written down here.
            "--strict-mcp-config",
            "--mcp-config",
            "{mcp_config}",
            "--allowedTools",
            ALLOWED_TOOLS,
            // A manager does not touch the repo. The absence of a tool for it is the rule;
            // this is the same rule where the agent itself can see it.
            "--disallowedTools",
            "Edit Write NotebookEdit",
        ],
    },
    AutopilotLaunch {
        kind: AgentKind::Codex,
        args: &["-c", "mcp_servers.moontasks.url={mcp_url}", "{prompt}"],
    },
    AutopilotLaunch {
        kind: AgentKind::OpenCode,
        // OpenCode's interactive mode takes no prompt argument, so its opening message is
        // typed into it and sent — see `TypeAhead::submit`. Its MCP server is added once with
        // `opencode mcp add --url`, because it has no per-run config either.
        args: &[],
    },
];

/// The arguments an autopilot window is started with, and whether its prompt has to be typed
/// into it rather than passed on the command line.
pub(crate) fn launch_args(agent: AgentKind, repo_path: &Path, prompt: &str) -> (Vec<String>, bool) {
    let Some(launch) = AUTOPILOT_LAUNCHES
        .iter()
        .find(|launch| launch.kind == agent)
    else {
        return (Vec::new(), true);
    };

    let mcp_url = mcp_url_for(repo_path);
    let mcp_config = mcp_config_for(&mcp_url);
    let args: Vec<String> = launch
        .args
        .iter()
        .map(|argument| {
            argument
                .replace("{mcp_url}", &mcp_url)
                .replace("{mcp_config}", &mcp_config)
                .replace("{prompt}", prompt)
        })
        .collect();

    let carries_prompt = launch
        .args
        .iter()
        .any(|argument| argument.contains("{prompt}"));
    (args, !carries_prompt)
}

/// This board's tools in the shape claude takes a server in, inline so there is nothing to
/// write to disk and nothing to be approved before it works.
fn mcp_config_for(mcp_url: &str) -> String {
    serde_json::json!({
        "mcpServers": { "moontasks": { "type": "http", "url": mcp_url } }
    })
    .to_string()
}

/// Where an agent finds this board's tools.
pub(crate) fn mcp_url_for(repo_path: &Path) -> String {
    format!(
        "{}/mcp?repo={}",
        crate::api::export_server_url(),
        repo_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brief_is_written_once_and_then_left_alone() {
        let repo = std::env::temp_dir().join(format!(
            "moonreview-autopilot-{}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        std::fs::create_dir_all(&repo).expect("expected a test repo");

        ensure_brief(&repo).expect("expected the brief to be written");
        std::fs::write(brief_path(&repo), "mine now\n").expect("expected to edit the brief");
        ensure_brief(&repo).expect("expected ensure to succeed");

        assert_eq!(
            std::fs::read_to_string(brief_path(&repo)).expect("expected the brief"),
            "mine now\n",
            "an edited brief must not be written back over"
        );

        std::fs::remove_dir_all(repo).expect("failed to remove the test repo");
    }

    /// A manager that could edit the repo would stop being a manager, so the one agent that
    /// can be told is told, and the tools it may use are named rather than blanket-allowed.
    /// `--allowedTools` and `--disallowedTools` take a list, so a prompt written after either
    /// of them is read as another tool name and the window comes up with nothing to do. This
    /// is the one ordering mistake here that is silent, so it gets a test of its own.
    #[test]
    fn a_prompt_is_never_left_where_a_list_flag_would_swallow_it() {
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            let (args, _) = launch_args(agent, Path::new("/repo"), "do the thing");
            let at = args
                .iter()
                .position(|argument| argument == "do the thing")
                .unwrap_or_else(|| panic!("{agent:?} should be given the prompt"));

            let swallowed = args[..at]
                .iter()
                .rev()
                .find(|argument| argument.starts_with("--"))
                .is_some_and(|flag| flag.ends_with("Tools"));
            assert!(
                !swallowed,
                "{agent:?} has its prompt behind a flag that takes a list: {args:?}"
            );
        }
    }

    #[test]
    fn claude_is_given_the_tools_by_name_and_kept_out_of_the_files() {
        let (args, types_its_prompt) =
            launch_args(AgentKind::Claude, Path::new("/repo"), "do the thing");

        assert!(!types_its_prompt, "claude takes its prompt as an argument");
        assert!(args.contains(&"do the thing".to_string()));
        assert!(
            args.iter()
                .any(|argument| argument.contains("\"mcpServers\"")),
            "the board's tools have to be passed in, not waited on"
        );
        let allowed = args
            .iter()
            .position(|argument| argument == "--allowedTools")
            .map(|at| args[at + 1].clone())
            .expect("expected an allow list");
        assert!(allowed.contains("mcp__moontasks__review"));
        assert!(
            !allowed.contains("Edit"),
            "the allow list is the board's tools only"
        );
        let disallowed = args
            .iter()
            .position(|argument| argument == "--disallowedTools")
            .map(|at| args[at + 1].clone())
            .expect("expected a deny list");
        assert!(disallowed.contains("Edit") && disallowed.contains("Write"));
    }

    /// The one agent whose interactive mode takes no prompt argument has to be typed into.
    #[test]
    fn opencode_is_told_its_work_by_being_typed_into() {
        let (args, types_its_prompt) =
            launch_args(AgentKind::OpenCode, Path::new("/repo"), "do the thing");

        assert!(args.is_empty());
        assert!(types_its_prompt);
    }

    #[test]
    fn codex_is_handed_the_board_as_one_config_key() {
        let (args, _) = launch_args(AgentKind::Codex, Path::new("/repo"), "do the thing");

        let url = args
            .iter()
            .find(|argument| argument.starts_with("mcp_servers.moontasks.url="))
            .expect("expected codex to be told where the tools are");
        assert!(url.ends_with("/mcp?repo=/repo"), "got {url}");
    }
}
