//! `<repo>/.moonreview.json`: the commands a project is built and run with, and how its
//! files are indented.
//!
//! Which command builds a repo, and whether its code is written in tabs or in spaces, are
//! facts about the repo rather than about whoever opened it, so they are kept with the repo
//! rather than in `~/.moonreview/settings.json` alongside the choices that belong to a person.
//! It is one small file at the root, in a format anyone can open and edit, and it can be
//! committed so everyone working on the repo gets the same commands and the same
//! indentation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::api::AppState;

const PROJECT_FILE_NAME: &str = ".moonreview.json";

/// The run command that is not a line of shell: it asks the window to start this program
/// again, which is how a project that builds this very program is "run" - moonreview cannot
/// run a new moonreview from a shell and cleanly close the one showing that shell. Only the
/// window can restart itself, so the server refuses to type this into a terminal.
pub(crate) const RESTART_RUN_COMMAND: &str = "@restart";

/// What the Project menu runs. The first two are the commands a project configures; the third
/// is the two chained, built out of them rather than stored. Running one is asking the server
/// for this, not for a line of shell to run: the command text lives in the repo's file and
/// never travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectCommand {
    Build,
    Run,
    #[serde(rename = "build-and-run")]
    BuildAndRun,
}

impl ProjectCommand {
    /// What the menu item and the shell's tab are called.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Run => "run",
            Self::BuildAndRun => "build and run",
        }
    }

    /// The word the command goes by in a URL, where the label's spaces cannot. What
    /// [`std::str::FromStr`] below parses.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Run => "run",
            Self::BuildAndRun => "build-and-run",
        }
    }
}

impl std::str::FromStr for ProjectCommand {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "build" => Ok(Self::Build),
            "run" => Ok(Self::Run),
            "build-and-run" => Ok(Self::BuildAndRun),
            other => bail!("{other} is not a project command"),
        }
    }
}

/// What the repo's file says about it.
///
/// A command that is not set is one the Project menu does not offer: there is no sensible
/// guess at how a repo is built, and an item that runs nothing is worse than no item. An
/// indentation that is not set is four spaces, which is a guess worth making - every file has
/// to be indented in something the moment a tab is pressed.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct ProjectConfig {
    #[serde(default)]
    pub(crate) build: Option<String>,
    #[serde(default)]
    pub(crate) run: Option<String>,
    /// What a Tab press puts into a file of this repo: `"tab"`, or `{ "spaces": 2 }`. Picked
    /// in the configuration pane, or typed into the file by hand, and read by the editor
    /// through [`ProjectConfig::indent`].
    #[serde(default)]
    pub(crate) indent: Option<egui_moon_editor::Indent>,
}

impl ProjectConfig {
    /// The command line one of the menu's items runs, if the project has set what it needs.
    /// Build and run is the two commands chained on the first's success. For a project whose
    /// run command is [`RESTART_RUN_COMMAND`] the run half is `exit` instead: the build shell
    /// ending is then the signal that the build came out well and the window can start again
    /// on it - the window watches for that exit, see
    /// `crate::native::workspace` (`close_tabs_of_exited_shells`). A failed build keeps the
    /// shell open on its errors, and nothing restarts.
    pub(crate) fn line(&self, which: ProjectCommand) -> Option<String> {
        match which {
            ProjectCommand::Build => self.build.clone(),
            ProjectCommand::Run => self.run.clone(),
            ProjectCommand::BuildAndRun => {
                let build = self.build.as_deref()?;
                let run = self.run.as_deref()?;
                if run == RESTART_RUN_COMMAND {
                    Some(format!("{build} && exit"))
                } else {
                    Some(format!("{build} && {run}"))
                }
            }
        }
    }

    /// Whether running this project means restarting the window rather than typing a line
    /// into a shell - see [`RESTART_RUN_COMMAND`].
    pub(crate) fn run_restarts_window(&self) -> bool {
        self.run.as_deref() == Some(RESTART_RUN_COMMAND)
    }

    /// How this repo's files are indented, which is four spaces until the file says otherwise.
    pub(crate) fn indent(&self) -> egui_moon_editor::Indent {
        self.indent.unwrap_or_default()
    }

    /// What the configuration pane is holding, as a file to write. This is the one place a
    /// blank box becomes an unset command - so a file written by the native pane and one
    /// written through the web say the same thing about a command nobody filled in.
    pub(crate) fn typed(build: &str, run: &str, indent: egui_moon_editor::Indent) -> Self {
        Self {
            build: typed_command(build),
            run: typed_command(run),
            indent: Some(indent),
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
pub(crate) fn read_project(repo_path: &Path) -> ProjectConfig {
    let path = project_path(repo_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ProjectConfig::default();
    };
    match serde_json::from_str(&text) {
        Ok(commands) => commands,
        Err(error) => {
            eprintln!("[moonreview] ignoring {}: {error}", path.display());
            ProjectConfig::default()
        }
    }
}

pub(crate) fn write_project(repo_path: &Path, commands: &ProjectConfig) -> Result<()> {
    let path = project_path(repo_path);
    let text = serde_json::to_string_pretty(commands).context("failed to encode the commands")?;
    std::fs::write(&path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// The commands of the repo one review is open on. Both frontends ask through this, so the
/// window and the browser read the same file.
pub(crate) fn session_commands(state: &AppState, session_id: &str) -> Result<ProjectConfig> {
    let repo_path = repo_of(state, session_id)?;
    Ok(read_project(&repo_path))
}

pub(crate) fn set_session_commands(
    state: &AppState,
    session_id: &str,
    commands: &ProjectConfig,
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
    // Restarting is the window's own act, not a line of shell - a window that asks for this
    // anyway read the file before it said so.
    if which == ProjectCommand::Run && commands.run_restarts_window() {
        bail!("this project's run command restarts the window, which only the window itself does");
    }
    let Some(line) = commands.line(which) else {
        bail!("this project has no {} command", which.label());
    };
    state
        .terminals
        .spawn(crate::terminal::TerminalSpec::running(repo_path, &line))
}

fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

#[cfg(test)]
mod tests {
    use egui_moon_editor::Indent;

    use super::*;

    fn scratch_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moonreview-project-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("failed to make the scratch repo");
        dir
    }

    #[test]
    fn a_repo_with_no_file_has_neither_command() {
        let commands = read_project(&scratch_repo("empty"));

        assert_eq!(commands, ProjectConfig::default());
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        let repo = scratch_repo("round-trip");
        let commands = ProjectConfig::typed("cargo build", "cargo run -- .", Indent::default());

        write_project(&repo, &commands).expect("failed to write the commands");

        assert_eq!(read_project(&repo), commands);
    }

    #[test]
    fn a_repo_that_says_nothing_about_indentation_gets_four_spaces() {
        assert_eq!(
            read_project(&scratch_repo("no-indent")).indent(),
            Indent::Spaces(4)
        );
    }

    #[test]
    fn a_repo_can_ask_for_tabs_or_for_a_width_of_its_own() {
        let repo = scratch_repo("indent");

        std::fs::write(project_path(&repo), r#"{ "indent": "tab" }"#)
            .expect("failed to write the file");
        assert_eq!(read_project(&repo).indent(), Indent::Tab);

        std::fs::write(project_path(&repo), r#"{ "indent": { "spaces": 2 } }"#)
            .expect("failed to write the file");
        assert_eq!(read_project(&repo).indent(), Indent::Spaces(2));
    }

    /// What the pane holds is what the file says, indentation included: the row of choices
    /// is written by the same save as the two boxes.
    #[test]
    fn what_the_pane_holds_is_written_indentation_and_all() {
        let repo = scratch_repo("pane-indent");
        let config = ProjectConfig::typed("cargo build", "cargo run", Indent::Tab);

        write_project(&repo, &config).expect("failed to write the configuration");

        assert_eq!(read_project(&repo).indent(), Indent::Tab);
        assert_eq!(read_project(&repo).build, Some("cargo build".to_string()));
    }

    #[test]
    fn a_blank_box_is_a_command_that_is_not_set() {
        let commands = ProjectConfig::typed("  ", "cargo run", Indent::default());

        assert_eq!(commands.build, None);
        assert_eq!(commands.run, Some("cargo run".to_string()));
    }

    #[test]
    fn a_file_that_makes_no_sense_leaves_both_commands_unset() {
        let repo = scratch_repo("broken");
        std::fs::write(project_path(&repo), "{ not json").expect("failed to write the file");

        assert_eq!(read_project(&repo), ProjectConfig::default());
    }

    #[test]
    fn build_and_run_is_the_two_commands_chained_on_success() {
        let commands = ProjectConfig::typed("cargo build", "cargo run -- .", Indent::default());

        assert_eq!(
            commands.line(ProjectCommand::BuildAndRun),
            Some("cargo build && cargo run -- .".to_string())
        );
    }

    #[test]
    fn build_and_run_needs_both_commands() {
        assert_eq!(
            ProjectConfig::typed("cargo build", "", Indent::default())
                .line(ProjectCommand::BuildAndRun),
            None
        );
        assert_eq!(
            ProjectConfig::typed("", "cargo run", Indent::default())
                .line(ProjectCommand::BuildAndRun),
            None
        );
    }

    #[test]
    fn a_restart_word_run_command_makes_the_build_shell_exit_on_success() {
        let commands = ProjectConfig::typed("cargo build", RESTART_RUN_COMMAND, Indent::default());

        assert!(commands.run_restarts_window());
        assert_eq!(
            commands.line(ProjectCommand::BuildAndRun),
            Some("cargo build && exit".to_string())
        );
    }

    #[test]
    fn every_command_parses_back_from_its_token() {
        for which in [
            ProjectCommand::Build,
            ProjectCommand::Run,
            ProjectCommand::BuildAndRun,
        ] {
            assert_eq!(
                which
                    .token()
                    .parse::<ProjectCommand>()
                    .expect("should parse"),
                which
            );
        }
    }
}
