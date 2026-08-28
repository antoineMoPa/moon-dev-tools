//! `<repo>/.moonreview.json`: the two commands a project is built and run with.
//!
//! Which command builds a repo is a fact about the repo, not about whoever opened it, so it is
//! kept with the repo rather than in `~/.moonreview/settings.json` alongside the choices that
//! belong to a person. It is one small file at the root, in a format anyone can open and edit,
//! and it can be committed so everyone working on the repo gets the same two commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

const PROJECT_FILE_NAME: &str = ".moonreview.json";

/// One of the two commands a project configures. The Project menu has an item per variant,
/// and running one is asking the server for this, not for a line of shell to run: the command
/// text lives in the repo's file and never travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectCommand {
    Build,
    Run,
}

impl ProjectCommand {
    /// What the menu item and the shell's tab are called.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Run => "run",
        }
    }
}

impl std::str::FromStr for ProjectCommand {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "build" => Ok(Self::Build),
            "run" => Ok(Self::Run),
            other => bail!("{other} is not a project command"),
        }
    }
}

/// What the Project menu runs. A command that is not set is one the menu does not offer:
/// there is no sensible guess at how a repo is built, and an item that runs nothing is worse
/// than no item.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct ProjectCommands {
    #[serde(default)]
    pub(crate) build: Option<String>,
    #[serde(default)]
    pub(crate) run: Option<String>,
}

impl ProjectCommands {
    /// The command line one of the menu's items runs, if the project has set it.
    pub(crate) fn line(&self, which: ProjectCommand) -> Option<&str> {
        match which {
            ProjectCommand::Build => self.build.as_deref(),
            ProjectCommand::Run => self.run.as_deref(),
        }
    }

    /// What two boxes of typed text mean, which is the one place a blank box becomes an unset
    /// command - so a file written by the native pane and one written through the web say the
    /// same thing about a command nobody filled in.
    pub(crate) fn typed(build: &str, run: &str) -> Self {
        Self {
            build: typed_command(build),
            run: typed_command(run),
        }
    }
}

fn typed_command(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn project_path(repo_path: &Path) -> PathBuf {
    repo_path.join(PROJECT_FILE_NAME)
}

/// The commands the repo's file has, or none at all.
///
/// A file that cannot be read or makes no sense leaves both commands unset, the way
/// [`crate::moontasks::store::read_board`] falls back to its defaults: a window that opens
/// with an empty Project menu is worth more than one that refuses to open, and this file is
/// hand-editable, so a half-typed one is an ordinary thing to find.
pub(crate) fn read_project(repo_path: &Path) -> ProjectCommands {
    let path = project_path(repo_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ProjectCommands::default();
    };
    match serde_json::from_str(&text) {
        Ok(commands) => commands,
        Err(error) => {
            eprintln!("[moonreview] ignoring {}: {error}", path.display());
            ProjectCommands::default()
        }
    }
}

pub(crate) fn write_project(repo_path: &Path, commands: &ProjectCommands) -> Result<()> {
    let path = project_path(repo_path);
    let text = serde_json::to_string_pretty(commands).context("failed to encode the commands")?;
    std::fs::write(&path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// The commands of the repo one review is open on. Both frontends ask through this, so the
/// window and the browser read the same file.
pub(crate) fn session_commands(state: &AppState, session_id: &str) -> Result<ProjectCommands> {
    let repo_path = repo_of(state, session_id)?;
    Ok(read_project(&repo_path))
}

pub(crate) fn set_session_commands(
    state: &AppState,
    session_id: &str,
    commands: &ProjectCommands,
) -> Result<()> {
    let repo_path = repo_of(state, session_id)?;
    write_project(&repo_path, commands)
}

/// Start a shell on the repo with one of the project's commands typed into it and sent.
///
/// A command the project has not set is an error rather than an empty shell: the menu only
/// offers the ones that are set, so asking for one that is not means the file changed under
/// whoever asked, and they should be told so.
pub(crate) fn run(state: &AppState, session_id: &str, which: ProjectCommand) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    let commands = read_project(&repo_path);
    let Some(line) = commands.line(which) else {
        bail!("this project has no {} command", which.label());
    };
    state
        .terminals
        .spawn(crate::terminal::TerminalSpec::running(repo_path, line))
}

fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moonreview-project-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("failed to make the scratch repo");
        dir
    }

    #[test]
    fn a_repo_with_no_file_has_neither_command() {
        let commands = read_project(&scratch_repo("empty"));

        assert_eq!(commands, ProjectCommands::default());
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        let repo = scratch_repo("round-trip");
        let commands = ProjectCommands::typed("cargo build", "cargo run -- .");

        write_project(&repo, &commands).expect("failed to write the commands");

        assert_eq!(read_project(&repo), commands);
    }

    #[test]
    fn a_blank_box_is_a_command_that_is_not_set() {
        let commands = ProjectCommands::typed("  ", "cargo run");

        assert_eq!(commands.build, None);
        assert_eq!(commands.run, Some("cargo run".to_string()));
    }

    #[test]
    fn a_file_that_makes_no_sense_leaves_both_commands_unset() {
        let repo = scratch_repo("broken");
        std::fs::write(project_path(&repo), "{ not json").expect("failed to write the file");

        assert_eq!(read_project(&repo), ProjectCommands::default());
    }
}
