//! `~/.moonreview/settings.json`: the choices that belong to the person rather than to a repo.
//!
//! A review's session is new on every launch, and the arrangement of panes belongs to the
//! window, so neither is the right place for something like which agent to hand comments to.
//! That is a preference, it outlives both, and it is kept somewhere a person can open and
//! edit - one file, in the obvious place, in a format they can read.
//!
//! The file is the server's: it sits on the machine the repos are on, and every window reads
//! and changes it through its backend - see [`Backend::settings`](crate::backend::Backend).
//! A `--remote` window, or the window in a browser, is someone at another machine working on
//! this one's repos, so it gets this machine's recent projects, agent and workspace colors
//! rather than starting blank. What it is not is the viewer's own: two machines each serving
//! a window keep two files.
//!
//! A window never writes the file whole. It sends one [`SettingsChange`] at a time, which the
//! server applies to what the file says then - so two windows on one server, each holding
//! the copy it read when it opened, cannot write back each other's changes.

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{api::AgentKind, native::workspace_color::WorkspaceColor};

#[cfg(not(target_arch = "wasm32"))]
const SETTINGS_DIR_NAME: &str = ".moonreview";
#[cfg(not(target_arch = "wasm32"))]
const SETTINGS_FILE_NAME: &str = "settings.json";

/// How many projects the launch screen offers. Enough to cover what someone is working on this
/// week, short enough that the list stays a list rather than a history.
const RECENT_PROJECTS_KEPT: usize = 8;

#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// The agent the review's selector is set to, which is who comments are handed to.
    #[serde(default)]
    pub(crate) selected_agent: AgentKind,
    /// The projects opened before, most recent first, offered again by the launch screen.
    #[serde(default)]
    pub(crate) recent_projects: Vec<String>,
    /// The color each project's window is marked with, by the project's path. It is the
    /// person's way of telling their projects apart rather than a fact about the repo, so it
    /// is kept here rather than in the repo's `.moonreview.json`, where it would be committed
    /// to everyone else working on it.
    ///
    /// A project that is not in the map is one nobody has marked, which is
    /// [`WorkspaceColor::Plain`]. Ordered so the file reads the same twice running.
    #[serde(default)]
    pub(crate) workspace_colors: BTreeMap<String, WorkspaceColor>,
}

/// One change a window asks the server to make to the file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SettingsChange {
    SelectAgent(AgentKind),
    RememberProject(String),
    MarkWorkspace {
        project_path: String,
        color: WorkspaceColor,
    },
}

impl Settings {
    /// Returns whether anything changed, so a change to what the file already says does not
    /// rewrite it.
    pub(crate) fn apply(&mut self, change: SettingsChange) -> bool {
        match change {
            SettingsChange::SelectAgent(agent) => {
                let changed = self.selected_agent != agent;
                self.selected_agent = agent;
                changed
            }
            SettingsChange::RememberProject(path) => self.remember_project(&path),
            SettingsChange::MarkWorkspace {
                project_path,
                color,
            } => self.mark_workspace(&project_path, color),
        }
    }

    /// Put a project at the head of the recent list. Opening one that is already there moves
    /// it up rather than listing it twice, and the oldest fall off the end.
    ///
    /// Returns whether the list changed, so a reopen of the project already at the head does
    /// not rewrite the file.
    pub(crate) fn remember_project(&mut self, path: &str) -> bool {
        if self
            .recent_projects
            .first()
            .is_some_and(|first| first == path)
        {
            return false;
        }
        self.recent_projects.retain(|recent| recent != path);
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(RECENT_PROJECTS_KEPT);
        true
    }

    /// The color the window on this project is painted, which for an unmarked project is
    /// the palette's own ground.
    pub(crate) fn workspace_color(&self, project_path: &str) -> WorkspaceColor {
        self.workspace_colors
            .get(project_path)
            .copied()
            .unwrap_or_default()
    }

    /// Mark this project's window. Returns whether anything changed, so setting the color a
    /// project already has does not rewrite the file.
    ///
    /// Going back to plain takes the project out of the map rather than writing `plain` into
    /// it: the map is the projects somebody marked, and one entry per project ever opened
    /// would be a file that only grows.
    pub(crate) fn mark_workspace(&mut self, project_path: &str, color: WorkspaceColor) -> bool {
        if self.workspace_color(project_path) == color {
            return false;
        }
        match color {
            WorkspaceColor::Plain => self.workspace_colors.remove(project_path),
            color => self
                .workspace_colors
                .insert(project_path.to_string(), color),
        };
        true
    }
}

/// `~/.moonreview`: where everything that is the person's rather than a repo's is kept - this
/// file, and the extensions they write (see [`crate::extensions`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn moonreview_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join(SETTINGS_DIR_NAME))
}

#[cfg(not(target_arch = "wasm32"))]
fn home_settings_path() -> Option<PathBuf> {
    Some(moonreview_dir()?.join(SETTINGS_FILE_NAME))
}

/// The file this run reads and writes.
///
/// Under test it is a scratch file of its own, one per test. The UI tests drive the real
/// window, and the real window saves the selector as it goes: that must never land in the
/// home directory of whoever is running them, and one test's saved agent must not turn up in
/// the next test's window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        // Cargo names each test's thread after the test, which is the isolation this needs.
        let test: String = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .chars()
            .map(|character| {
                if character.is_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect();
        Some(std::env::temp_dir().join(format!(
            "moonreview-test-settings-{}-{test}.json",
            std::process::id()
        )))
    }
    #[cfg(not(test))]
    {
        home_settings_path()
    }
}

/// Read the settings, falling back to the defaults.
///
/// Nothing stored yet is the ordinary case on a first run. A file that cannot be parsed is a
/// file someone has been editing: it is reported and then ignored, because starting with the
/// defaults is a far better outcome than refusing to start.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load() -> Settings {
    let Some(path) = path() else {
        return Settings::default();
    };
    read_at(&path).unwrap_or_else(|error| {
        eprintln!("[moonreview] ignoring {}: {error:#}", path.display());
        Settings::default()
    })
}

/// What the tests seed a window's server with before it opens.
#[cfg(test)]
pub(crate) fn store(settings: &Settings) -> Result<()> {
    write_at(
        &path().context("no home directory to keep settings in")?,
        settings,
    )
}

/// Held across each change the server makes, so two windows changing the file at once each
/// read what the other wrote.
#[cfg(not(target_arch = "wasm32"))]
static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The settings as the server serves them to a window - see [`load`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn served(state: &crate::api::AppState) -> Settings {
    let Some(path) = &state.settings_path else {
        return Settings::default();
    };
    let _held = CHANGING.lock().expect("the settings lock");
    read_at(path).unwrap_or_else(|error| {
        eprintln!("[moonreview] ignoring {}: {error:#}", path.display());
        Settings::default()
    })
}

/// Make one change to the file, on top of what it says now.
///
/// A file that cannot be parsed is refused here rather than read as the defaults, which
/// would write back a file with everything but this change gone.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn change(state: &crate::api::AppState, change: SettingsChange) -> Result<()> {
    let path = state
        .settings_path
        .as_ref()
        .context("no home directory to keep settings in")?;
    let _held = CHANGING.lock().expect("the settings lock");
    let mut settings = read_at(path)?;
    if !settings.apply(change) {
        return Ok(());
    }
    write_at(path, &settings)
}

/// No file yet reads as the defaults; one that is there has to parse.
#[cfg(not(target_arch = "wasm32"))]
fn read_at(path: &std::path::Path) -> Result<Settings> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_at(path: &std::path::Path, settings: &Settings) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_live_in_a_dot_directory_of_the_home_directory() {
        let path = home_settings_path().expect("expected a settings path");

        assert!(path.ends_with(".moonreview/settings.json"), "got {path:?}");
    }

    /// The window writes as it runs, so a test run must not be able to reach the real file.
    #[test]
    fn a_test_run_never_writes_to_the_home_directory() {
        let path = path().expect("expected a settings path");

        assert!(path.starts_with(std::env::temp_dir()), "got {path:?}");
    }

    #[test]
    fn an_unwritten_agent_reads_back_as_none() {
        let settings: Settings = serde_json::from_str("{}").expect("expected the defaults");

        assert_eq!(settings.selected_agent, AgentKind::None);
    }

    /// The file is meant to be edited by hand, so what it holds has to read as what it means.
    #[test]
    fn the_file_names_the_agent_in_words() {
        let encoded = serde_json::to_string(&Settings {
            selected_agent: AgentKind::Claude,
            recent_projects: Vec::new(),
            workspace_colors: BTreeMap::new(),
        })
        .expect("expected json");

        assert_eq!(
            encoded,
            r#"{"selected_agent":"claude","recent_projects":[],"workspace_colors":{}}"#
        );
    }

    #[test]
    fn an_unmarked_project_is_plain() {
        let settings: Settings = serde_json::from_str("{}").expect("expected the defaults");

        assert!(settings.workspace_colors.is_empty());
        assert_eq!(
            settings.workspace_color("/repos/anything"),
            WorkspaceColor::Plain
        );
    }

    #[test]
    fn a_marked_project_reads_back_the_color_it_was_marked() {
        let mut settings = Settings::default();

        assert!(settings.mark_workspace("/repos/one", WorkspaceColor::Teal));

        let text = serde_json::to_string(&settings).expect("expected json");
        let read: Settings = serde_json::from_str(&text).expect("expected the settings back");

        assert_eq!(read.workspace_color("/repos/one"), WorkspaceColor::Teal);
        assert_eq!(read.workspace_color("/repos/two"), WorkspaceColor::Plain);
    }

    /// The map is the projects somebody marked, so unmarking one takes it out again rather
    /// than leaving `plain` behind for every project ever opened.
    #[test]
    fn marking_a_project_plain_takes_it_out_of_the_file() {
        let mut settings = Settings::default();
        settings.mark_workspace("/repos/one", WorkspaceColor::Ember);

        assert!(settings.mark_workspace("/repos/one", WorkspaceColor::Plain));

        assert!(settings.workspace_colors.is_empty());
    }

    /// What keeps a window that is already the right color from rewriting the file.
    #[test]
    fn marking_a_project_the_color_it_already_is_changes_nothing() {
        let mut settings = Settings::default();
        settings.mark_workspace("/repos/one", WorkspaceColor::Moss);

        assert!(!settings.mark_workspace("/repos/one", WorkspaceColor::Moss));
        assert!(!settings.mark_workspace("/repos/two", WorkspaceColor::Plain));
    }

    #[test]
    fn a_change_to_what_the_file_already_says_changes_nothing() {
        let mut settings = Settings::default();

        assert!(!settings.apply(SettingsChange::SelectAgent(AgentKind::None)));
        assert!(settings.apply(SettingsChange::SelectAgent(AgentKind::Claude)));
        assert!(!settings.apply(SettingsChange::SelectAgent(AgentKind::Claude)));
        assert!(settings.apply(SettingsChange::RememberProject("/a".to_string())));
        assert!(!settings.apply(SettingsChange::RememberProject("/a".to_string())));
    }

    /// The change travels as JSON, and reads as what it asks for.
    #[test]
    fn a_change_names_what_it_changes() {
        let encoded = serde_json::to_string(&SettingsChange::MarkWorkspace {
            project_path: "/repos/one".to_string(),
            color: WorkspaceColor::Teal,
        })
        .expect("expected json");

        assert_eq!(
            encoded,
            r#"{"mark_workspace":{"project_path":"/repos/one","color":"teal"}}"#
        );
    }

    #[test]
    fn an_unwritten_recent_list_reads_back_empty() {
        let settings: Settings = serde_json::from_str("{}").expect("expected the defaults");

        assert!(settings.recent_projects.is_empty());
    }

    #[test]
    fn reopening_a_project_moves_it_to_the_head_rather_than_listing_it_twice() {
        let mut settings = Settings::default();
        settings.remember_project("/a");
        settings.remember_project("/b");

        assert!(settings.remember_project("/a"));
        assert_eq!(
            settings.recent_projects,
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn the_project_already_at_the_head_is_left_alone() {
        let mut settings = Settings::default();
        settings.remember_project("/a");

        assert!(!settings.remember_project("/a"));
    }

    #[test]
    fn the_oldest_projects_fall_off_the_end() {
        let mut settings = Settings::default();
        for index in 0..RECENT_PROJECTS_KEPT + 3 {
            settings.remember_project(&format!("/project-{index}"));
        }

        assert_eq!(settings.recent_projects.len(), RECENT_PROJECTS_KEPT);
        assert_eq!(settings.recent_projects[0], "/project-10");
        assert_eq!(
            settings.recent_projects[RECENT_PROJECTS_KEPT - 1],
            "/project-3"
        );
    }

    #[test]
    fn a_file_that_cannot_be_parsed_falls_back_to_the_defaults() {
        // `load` reads the real home directory, so the fallback is checked on its own.
        let settings: Settings =
            serde_json::from_str("not json").unwrap_or_else(|_| Settings::default());

        assert_eq!(settings, Settings::default());
    }
}
