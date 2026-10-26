//! The repo side of the project file: reading and writing it, and running what it says.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use serde::Deserialize;

use super::{
    BuildTarget, PROJECT_SHELL_NAME, ProjectCommand, ProjectConfig, ProjectRunRequest, TimeOfDay,
};
use crate::api::AppState;

const PROJECT_FILE_NAME: &str = ".moonreview.json";

fn project_path(repo_path: &Path) -> PathBuf {
    repo_path.join(PROJECT_FILE_NAME)
}

/// The repo's file as it may be found on disk: the targets it has now, or the one `build` and
/// one `run` it had before targets, which a repo nobody has opened the settings of since
/// still says.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    #[serde(default)]
    targets: Vec<BuildTarget>,
    #[serde(default)]
    build: Option<String>,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    indent: Option<egui_moon_editor::Indent>,
    #[serde(default)]
    day_ends_at: Option<TimeOfDay>,
}

impl ProjectFile {
    /// The project the file describes. The two commands of a file from before targets are
    /// its first target, named after the repo's folder. The file itself is left as it is: it
    /// takes the new shape the next time the settings pane writes it, rather than changing
    /// under a repo that was only opened.
    fn project(self, repo_path: &Path) -> ProjectConfig {
        let mut targets = self.targets;
        if self.build.is_some() || self.run.is_some() {
            let name = repo_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .expect("a repo's path ends in its folder");
            targets.insert(
                0,
                BuildTarget {
                    name,
                    build: self.build,
                    run: self.run,
                },
            );
        }
        ProjectConfig {
            targets,
            indent: self.indent,
            day_ends_at: self.day_ends_at,
        }
    }
}

/// What the repo's file says, or nothing at all.
///
/// A file that cannot be read or makes no sense leaves the project with no targets, the way
/// [`crate::moontasks::store::read_board`] falls back to its defaults: a window that opens
/// with an empty Project menu is worth more than one that refuses to open, and this file is
/// hand-editable, so a half-typed one is an ordinary thing to find.
pub(crate) fn read_project(repo_path: &Path) -> ProjectConfig {
    let path = project_path(repo_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ProjectConfig::default();
    };
    match serde_json::from_str::<ProjectFile>(&text) {
        Ok(file) => file.project(repo_path),
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

/// Start a shell on the repo with one of a target's commands typed into it and sent.
///
/// A target or a command the project has not set is an error rather than an empty shell: the
/// menu only offers the ones that are set, so asking for one that is not means the file
/// changed under whoever asked, and they should be told so.
pub(crate) fn run(
    state: &AppState,
    session_id: &str,
    request: &ProjectRunRequest,
) -> Result<String> {
    let repo_path = repo_of(state, session_id)?;
    let project = read_project(&repo_path);
    let Some(target) = project.target(&request.target) else {
        bail!("this project has no target called {}", request.target);
    };
    // Restarting is the window's own act, not a line of shell - a window that asks for this
    // anyway read the file before it said so.
    if request.which == ProjectCommand::Run && target.run_restarts_window() {
        bail!(
            "{}'s run command restarts the window, which only the window itself does",
            target.name
        );
    }
    let Some(line) = target.line(request.which) else {
        bail!("{} has no {} command", target.name, request.which.label());
    };
    state.terminals.spawn(crate::terminal::TerminalSpec {
        name: Some(PROJECT_SHELL_NAME.to_string()),
        runs_as: crate::api::person_of(state, session_id)?,
        ..crate::terminal::TerminalSpec::running(repo_path, &line)
    })
}

fn repo_of(state: &AppState, session_id: &str) -> Result<PathBuf> {
    crate::api::with_session(state, session_id, |session| Ok(session.repo_path.clone()))
}

#[cfg(test)]
mod tests {
    use egui_moon_editor::Indent;

    use super::*;
    use crate::project::{BuildTarget, RESTART_RUN_COMMAND};

    fn scratch_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moonreview-project-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("failed to make the scratch repo");
        dir
    }

    fn app_target() -> BuildTarget {
        BuildTarget::typed("app", "cargo build", "cargo run -- .")
    }

    #[test]
    fn a_repo_with_no_file_has_no_targets() {
        let project = read_project(&scratch_repo("empty"));

        assert_eq!(project, ProjectConfig::default());
    }

    /// When the day ends is written as a clock reads - `"21:05"` - and a project that has
    /// not said ends its day at half past four.
    #[test]
    fn when_the_day_ends_is_written_as_a_time_and_read_back() {
        use crate::project::{DAY_ENDS_AT, TimeOfDay};

        let repo = scratch_repo("day-end");
        assert_eq!(read_project(&repo).day_ends_at(), DAY_ENDS_AT);
        assert_eq!(DAY_ENDS_AT.to_string(), "16:30");

        let config = ProjectConfig::typed(vec![app_target()], Indent::default())
            .with_day_ending_at(TimeOfDay::new(21, 5));
        write_project(&repo, &config).expect("failed to write the configuration");

        let written = std::fs::read_to_string(repo.join(".moonreview.json"))
            .expect("expected the project file");
        assert!(
            written.contains(r#""day_ends_at": "21:05""#),
            "got {written}"
        );
        assert_eq!(read_project(&repo).day_ends_at(), TimeOfDay::new(21, 5));
    }

    #[test]
    fn a_time_that_no_clock_reads_is_refused() {
        use crate::project::TimeOfDay;

        for text in ["24:00", "12:60", "noon", "12", "-1:00", ""] {
            assert!(
                TimeOfDay::try_from(text.to_string()).is_err(),
                "{text:?} should not have been read as a time of day"
            );
        }
        assert_eq!(
            TimeOfDay::try_from("9:05".to_string()).expect("expected a time"),
            TimeOfDay::new(9, 5)
        );
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        let repo = scratch_repo("round-trip");
        let project = ProjectConfig::typed(
            vec![app_target(), BuildTarget::typed("web", "npm run build", "")],
            Indent::Tab,
        );

        write_project(&repo, &project).expect("failed to write the project");

        assert_eq!(read_project(&repo), project);
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

    #[test]
    fn a_blank_box_is_a_command_that_is_not_set_and_a_blank_name_no_target() {
        let target = BuildTarget::typed(" app ", "  ", "cargo run");
        assert_eq!(target.name, "app");
        assert_eq!(target.build, None);
        assert_eq!(target.run, Some("cargo run".to_string()));

        let project = ProjectConfig::typed(
            vec![BuildTarget::typed("  ", "cargo build", ""), target],
            Indent::default(),
        );
        assert_eq!(project.targets.len(), 1);
    }

    #[test]
    fn a_file_that_makes_no_sense_leaves_no_targets() {
        let repo = scratch_repo("broken");
        std::fs::write(project_path(&repo), "{ not json").expect("failed to write the file");

        assert_eq!(read_project(&repo), ProjectConfig::default());
    }

    /// The `build` and `run` a file had before targets are its first target, named after the
    /// repo's folder, and the file is not rewritten by being read.
    #[test]
    fn a_file_of_the_old_two_commands_is_read_as_a_first_target() {
        let repo = scratch_repo("old-shape");
        let old = r#"{ "build": "cargo build", "run": "cargo run", "indent": "tab" }"#;
        std::fs::write(project_path(&repo), old).expect("failed to write the file");

        let project = read_project(&repo);

        let folder = repo.file_name().expect("a folder").to_string_lossy();
        assert_eq!(
            project.targets,
            vec![BuildTarget::typed(&folder, "cargo build", "cargo run")]
        );
        assert_eq!(project.indent(), Indent::Tab);
        assert_eq!(
            std::fs::read_to_string(project_path(&repo)).expect("the file"),
            old
        );
    }

    #[test]
    fn build_and_run_is_the_two_commands_chained_on_success() {
        assert_eq!(
            app_target().line(ProjectCommand::BuildAndRun),
            Some("cargo build && cargo run -- .".to_string())
        );
    }

    #[test]
    fn build_and_run_needs_both_commands() {
        assert_eq!(
            BuildTarget::typed("app", "cargo build", "").line(ProjectCommand::BuildAndRun),
            None
        );
        assert_eq!(
            BuildTarget::typed("app", "", "cargo run").line(ProjectCommand::BuildAndRun),
            None
        );
    }

    #[test]
    fn a_restart_word_run_command_makes_the_build_shell_exit_on_success() {
        let target = BuildTarget::typed("app", "cargo build", RESTART_RUN_COMMAND);

        assert!(target.run_restarts_window());
        assert_eq!(
            target.line(ProjectCommand::BuildAndRun),
            Some("cargo build && exit".to_string())
        );
    }

    /// The shortcuts repeat the target run last, and the first one until anything has run or
    /// once the one run last is gone from the file.
    #[test]
    fn the_target_to_repeat_is_the_last_run_while_it_is_there() {
        let project = ProjectConfig::typed(
            vec![app_target(), BuildTarget::typed("web", "npm run build", "")],
            Indent::default(),
        );

        let named = |last: Option<&str>| {
            project
                .target_to_repeat(last)
                .map(|target| target.name.clone())
        };
        assert_eq!(named(None), Some("app".to_string()));
        assert_eq!(named(Some("web")), Some("web".to_string()));
        assert_eq!(named(Some("docs")), Some("app".to_string()));
        assert_eq!(ProjectConfig::default().target_to_repeat(Some("web")), None);
    }
}
