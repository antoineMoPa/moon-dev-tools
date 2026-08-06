//! `~/.moonreview/settings.json`: the choices that belong to the person rather than to a repo.
//!
//! A review's session is new on every launch, and the arrangement of panes belongs to the
//! window, so neither is the right place for something like which agent to hand comments to.
//! That is a preference, it outlives both, and it is kept somewhere a person can open and
//! edit — one file, in the obvious place, in a format they can read.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

const SETTINGS_DIR_NAME: &str = ".moonreview";
const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Clone, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// The agent the review's selector is set to, which is who comments are handed to.
    #[serde(default)]
    pub(crate) selected_agent: AgentKind,
}

fn home_settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(SETTINGS_DIR_NAME)
            .join(SETTINGS_FILE_NAME),
    )
}

/// The file this run reads and writes.
///
/// Under test it is a scratch file of its own, one per test. The UI tests drive the real
/// window, and the real window saves the selector as it goes: that must never land in the
/// home directory of whoever is running them, and one test's saved agent must not turn up in
/// the next test's window.
pub(crate) fn path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        // Cargo names each test's thread after the test, which is the isolation this needs.
        let test: String = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .chars()
            .map(|character| if character.is_alphanumeric() { character } else { '-' })
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
/// No file yet is the ordinary case on a first run. A file that cannot be parsed is a file
/// someone has been editing: it is reported and then ignored, because starting with the
/// defaults is a far better outcome than refusing to start.
pub(crate) fn load() -> Settings {
    let Some(path) = path() else {
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    match serde_json::from_str(&text) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("[moonreview] ignoring {}: {error}", path.display());
            Settings::default()
        }
    }
}

pub(crate) fn store(settings: &Settings) -> Result<()> {
    let path = path().context("no home directory to keep settings in")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    std::fs::write(&path, format!("{text}\n"))
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
        })
        .expect("expected json");

        assert_eq!(encoded, r#"{"selected_agent":"claude"}"#);
    }

    #[test]
    fn a_file_that_cannot_be_parsed_falls_back_to_the_defaults() {
        // `load` reads the real home directory, so the fallback is checked on its own.
        let settings: Settings =
            serde_json::from_str("not json").unwrap_or_else(|_| Settings::default());

        assert_eq!(settings, Settings::default());
    }
}
