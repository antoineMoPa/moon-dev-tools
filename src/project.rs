//! `<repo>/.moonreview.json`: the commands a project is built and run with, and how its
//! files are indented.
//!
//! Which command builds a repo, and whether its code is written in tabs or in spaces, are
//! facts about the repo rather than about whoever opened it, so they are kept with the repo
//! rather than in `~/.moonreview/settings.json` alongside the choices that belong to a person.
//! It is one small file at the root, in a format anyone can open and edit, and it can be
//! committed so everyone working on the repo gets the same commands and the same
//! indentation.

// The file is read and written, and its commands run, on the server's side: the window in a
// browser only has the types it is told about them in.
#[cfg(not(target_arch = "wasm32"))]
mod repo_side;

#[cfg(test)]
pub(crate) use repo_side::write_project;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use repo_side::{run, session_commands, set_session_commands};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// The run command that is not a line of shell: it asks the window to start this program
/// again, which is how a project that builds this very program is "run" - moonreview cannot
/// run a new moonreview from a shell and cleanly close the one showing that shell. Only the
/// window can restart itself, so the server refuses to type this into a terminal.
pub(crate) const RESTART_RUN_COMMAND: &str = "@restart";

/// What the shell the Project menu's commands run in is called, on its tab and everywhere
/// else a shell's name is read. The window keeps one of these and types every build and run
/// into it - see `App::run_project_command` - so the name is the same whichever of the
/// commands opened it, and there is no number after it to tell one from the next.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PROJECT_SHELL_NAME: &str = "Build terminal";

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
