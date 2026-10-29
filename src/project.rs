//! Project settings - `<repo>/.moonreview.json`, which holds the targets a project is built
//! and run as, and how its files are indented.
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

/// What the Project menu runs of a target. The first two are the commands a target
/// configures; the third is the two chained, built out of them rather than stored. Running one
/// is asking the server for this and the target's name, not for a line of shell to run: the
/// command text lives in the repo's file and never travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProjectCommand {
    Build,
    Run,
    BuildAndRun,
}

impl ProjectCommand {
    /// Every command, in the order the menus list them.
    pub(crate) const ALL: [Self; 3] = [Self::Build, Self::Run, Self::BuildAndRun];

    /// What the menu item and the shell's tab are called.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Run => "run",
            Self::BuildAndRun => "build and run",
        }
    }

    /// The label as a menu item reads it, capitalised.
    pub(crate) fn menu_label(self) -> String {
        let mut label = self.label().to_string();
        label[..1].make_ascii_uppercase();
        label
    }
}

/// What the window asks the server to run: one command of one target, by the target's name.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct ProjectRunRequest {
    pub(crate) target: String,
    pub(crate) which: ProjectCommand,
}

/// A time of day to the minute, as the repo's file writes it: `"16:30"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct TimeOfDay {
    hour: u8,
    minute: u8,
}

impl TimeOfDay {
    pub(crate) const HOURS: std::ops::RangeInclusive<u8> = 0..=23;
    pub(crate) const MINUTES: std::ops::RangeInclusive<u8> = 0..=59;

    pub(crate) const fn new(hour: u8, minute: u8) -> Self {
        assert!(hour <= 23 && minute <= 59, "not a time of day");
        Self { hour, minute }
    }

    pub(crate) fn hour(self) -> u8 {
        self.hour
    }

    pub(crate) fn minute(self) -> u8 {
        self.minute
    }

    /// How far into the day it is, which is what a clock is compared by.
    pub(crate) fn minutes_into_day(self) -> u32 {
        u32::from(self.hour) * 60 + u32::from(self.minute)
    }
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{:02}", self.hour, self.minute)
    }
}

impl From<TimeOfDay> for String {
    fn from(time: TimeOfDay) -> Self {
        time.to_string()
    }
}

impl TryFrom<String> for TimeOfDay {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        let parsed = text.split_once(':').and_then(|(hour, minute)| {
            let (hour, minute) = (hour.parse::<u8>().ok()?, minute.parse::<u8>().ok()?);
            (Self::HOURS.contains(&hour) && Self::MINUTES.contains(&minute))
                .then_some(Self { hour, minute })
        });
        match parsed {
            Some(time) => Ok(time),
            None => bail!("{text} is not a time of day: write it as 16:30"),
        }
    }
}

/// When a day of work ends in a project that has not said: half past four.
pub(crate) const DAY_ENDS_AT: TimeOfDay = TimeOfDay::new(16, 30);

/// One thing a project builds and runs - the program, its web page, its docs - under a name
/// the Project menu lists it by.
///
/// A command that is not set is one the menu does not offer for this target: there is no
/// sensible guess at how a thing is built, and an item that runs nothing is worse than no
/// item.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildTarget {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run: Option<String>,
}

impl BuildTarget {
    /// The command line one of the menu's items runs, if the target has set what it needs.
    /// Build and run is the two commands chained on the first's success. For a target whose
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

    /// Whether running this target means restarting the window rather than typing a line
    /// into a shell - see [`RESTART_RUN_COMMAND`].
    pub(crate) fn run_restarts_window(&self) -> bool {
        self.run.as_deref() == Some(RESTART_RUN_COMMAND)
    }

    /// The target as the configuration pane's boxes hold it: a blank box is a command that is
    /// not set.
    pub(crate) fn typed(name: &str, build: &str, run: &str) -> Self {
        Self {
            name: name.trim().to_string(),
            build: typed_command(build),
            run: typed_command(run),
        }
    }
}

/// What the repo's file says about it.
///
/// A file from before targets, with one `build` and one `run`, is read as its first target -
/// see `repo_side::ProjectFile`. A field the file does not know makes the whole file one that
/// is ignored and said so. An indentation that is not set is four spaces,
/// which is a guess worth making - every file has to be indented in something the moment a
/// tab is pressed.
#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectConfig {
    /// What the Project menu builds and runs, in the order it lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) targets: Vec<BuildTarget>,
    /// What a Tab press puts into a file of this repo: `"tab"`, or `{ "spaces": 2 }`. Picked
    /// in the configuration pane, or typed into the file by hand, and read by the editor
    /// through [`ProjectConfig::indent`].
    #[serde(default)]
    pub(crate) indent: Option<egui_moon_editor::Indent>,
    /// When a day of work on this project ends, which is when the board stops counting any of
    /// its queue as today's - see [`ProjectConfig::day_ends_at`]. A fact about the project
    /// rather than the person: one worked on in the evenings ends later than the day job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) day_ends_at: Option<TimeOfDay>,
    /// Whether a line typed after `!` in the command palette runs as shell - see
    /// [`ProjectConfig::palette_shell`]. Set to `false` by hand in a project where running
    /// shell from the palette is of no use; the configuration pane keeps what the file says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) palette_shell: Option<bool>,
}

impl ProjectConfig {
    /// The target of this name. Two targets of one name are the first of them: the pane says
    /// so where the names are typed.
    pub(crate) fn target(&self, name: &str) -> Option<&BuildTarget> {
        self.targets.iter().find(|target| target.name == name)
    }

    /// The target whose items in the macOS menu bar carry the chords: the one run last, while
    /// the file still has it, and the first one until anything has run. The bar a window draws
    /// for itself has no chords for these.
    #[cfg(any(all(target_os = "macos", not(target_arch = "wasm32")), test))]
    pub(crate) fn target_to_repeat(&self, last_run: Option<&str>) -> Option<&BuildTarget> {
        last_run
            .and_then(|name| self.target(name))
            .or_else(|| self.targets.first())
    }

    /// How this repo's files are indented, which is four spaces until the file says otherwise.
    pub(crate) fn indent(&self) -> egui_moon_editor::Indent {
        self.indent.unwrap_or_default()
    }

    /// When a day of work on this project ends, which is [`DAY_ENDS_AT`] until the file says
    /// otherwise.
    pub(crate) fn day_ends_at(&self) -> TimeOfDay {
        self.day_ends_at.unwrap_or(DAY_ENDS_AT)
    }

    /// Whether the palette runs `!` lines as shell in this project, which it does until the
    /// file says otherwise.
    pub(crate) fn palette_shell(&self) -> bool {
        self.palette_shell.unwrap_or(true)
    }

    /// What the configuration pane is holding, as a file to write. This is the one place a
    /// target whose name box is blank is dropped - so a file written by the native pane and
    /// one written through the web say the same thing about a target nobody named.
    pub(crate) fn typed(targets: Vec<BuildTarget>, indent: egui_moon_editor::Indent) -> Self {
        Self {
            targets: targets
                .into_iter()
                .filter(|target| !target.name.is_empty())
                .collect(),
            indent: Some(indent),
            day_ends_at: None,
            palette_shell: None,
        }
    }

    /// The same project, with its day of work ending at this time.
    pub(crate) fn with_day_ending_at(self, day_ends_at: TimeOfDay) -> Self {
        Self {
            day_ends_at: Some(day_ends_at),
            ..self
        }
    }

    pub(crate) fn with_palette_shell(self, palette_shell: Option<bool>) -> Self {
        Self {
            palette_shell,
            ..self
        }
    }
}

fn typed_command(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
