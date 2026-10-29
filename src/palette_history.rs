//! Palette history - what was run from the command palette, per project: the newest
//! [`HISTORY_KEPT`] commands, files, lines, lines of shell and quick tasks, one entry per thing.
//!
//! Kept in `~/.moonreview/palette-history.json`, one list per project path. The file is the
//! server's, the way `settings.json` is - see [`crate::settings`]: every window on the server
//! reads its project's list through its backend, and adds to it one [`HistoryRecord`] at a
//! time, which the server applies to what the file says then. A file of its own rather than a
//! part of `settings.json`, which a person opens and reads: a hundred entries a project would
//! bury the settings in it.

#[cfg(not(target_arch = "wasm32"))]
use std::{collections::BTreeMap, path::PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::AgentKind;

/// How many entries a project keeps: running a 101st drops the oldest.
pub(crate) const HISTORY_KEPT: usize = 100;

#[cfg(not(target_arch = "wasm32"))]
const HISTORY_FILE_NAME: &str = "palette-history.json";

/// One thing run from the palette, and when it last was.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub(crate) thing: HistoryThing,
    pub(crate) ran_at_unix: u64,
}

/// What was run. Each is known by what it runs, not by what its row said - see
/// [`HistoryThing::same_as`]; the rest is what its row is drawn with.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum HistoryThing {
    /// One of the palette's commands, known by its action as Rust's `Debug` writes it out -
    /// see `crate::native::palette::CommandAction`. A command reworded keeps its entry; one
    /// whose action no window offers any more matches no command, and its entry is hidden.
    Command {
        action: String,
        title: String,
        description: String,
    },
    /// A file opened by name, in the repo at `repo_path`.
    File {
        repo_path: String,
        file_path: String,
    },
    /// A file opened at a line, from the content search: `text` is what the line read.
    Line {
        repo_path: String,
        file_path: String,
        line_number: usize,
        text: String,
    },
    /// A line of shell run from the palette with `!`.
    Shell { line: String },
    /// A quick task, known by its card: the title it was made with and the agent started on it.
    QuickTask {
        task_id: String,
        title: String,
        agent: AgentKind,
    },
}

impl HistoryThing {
    /// Whether the two run the same thing, which makes them one entry.
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Command { action, .. }, Self::Command { action: other, .. }) => action == other,
            (
                Self::File {
                    repo_path,
                    file_path,
                },
                Self::File {
                    repo_path: other_repo,
                    file_path: other_file,
                },
            ) => repo_path == other_repo && file_path == other_file,
            (
                Self::Line {
                    repo_path,
                    file_path,
                    line_number,
                    ..
                },
                Self::Line {
                    repo_path: other_repo,
                    file_path: other_file,
                    line_number: other_number,
                    ..
                },
            ) => repo_path == other_repo && file_path == other_file && line_number == other_number,
            (Self::Shell { line }, Self::Shell { line: other }) => line == other,
            (Self::QuickTask { task_id, .. }, Self::QuickTask { task_id: other, .. }) => {
                task_id == other
            }
            _ => false,
        }
    }
}

/// What a window adds to its project's history: the entry for what was just run, and the
/// things it found no longer there - a command no longer offered, a quick task whose card is
/// done - which go with the same write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryRecord {
    pub(crate) entry: HistoryEntry,
    pub(crate) gone: Vec<HistoryThing>,
}

/// Add `record` to `history`, oldest first: the entry goes to the end, any earlier entry of
/// the same thing and every gone thing come out, and the oldest past [`HISTORY_KEPT`] go.
pub(crate) fn record(history: &mut Vec<HistoryEntry>, record: HistoryRecord) {
    let HistoryRecord { entry, gone } = record;
    history.retain(|kept| {
        !kept.thing.same_as(&entry.thing) && !gone.iter().any(|thing| thing.same_as(&kept.thing))
    });
    history.push(entry);
    let over = history.len().saturating_sub(HISTORY_KEPT);
    history.drain(..over);
}

/// The file the server keeps: each project's history by the project's path.
#[cfg(not(target_arch = "wasm32"))]
type HistoryFile = BTreeMap<String, Vec<HistoryEntry>>;

/// The file this run reads and writes. Under test it is a scratch file of the test's own -
/// see [`crate::settings::path`] for why.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::settings::test_scratch_file("palette-history"))
    }
    #[cfg(not(test))]
    {
        Some(crate::settings::moonreview_dir()?.join(HISTORY_FILE_NAME))
    }
}

/// Held across each read and write, so two windows recording at once each read what the other
/// wrote.
#[cfg(not(target_arch = "wasm32"))]
static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The history of the project `session_id` is on, oldest first, without the files that are no
/// longer in their repo.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn served(state: &crate::api::AppState, session_id: &str) -> Result<Vec<HistoryEntry>> {
    let project = project_of(state, session_id)?;
    let path = state
        .palette_history_path
        .as_ref()
        .context("no home directory to keep the palette history in")?;
    let _held = CHANGING.lock().expect("the palette history lock");
    let mut history = read_at(path)?.remove(&project).unwrap_or_default();
    history.retain(still_on_disk);
    Ok(history)
}

/// Add one record to the history of the project `session_id` is on. The files no longer in
/// their repo are dropped with it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn change(
    state: &crate::api::AppState,
    session_id: &str,
    added: HistoryRecord,
) -> Result<()> {
    let project = project_of(state, session_id)?;
    let path = state
        .palette_history_path
        .as_ref()
        .context("no home directory to keep the palette history in")?;
    let _held = CHANGING.lock().expect("the palette history lock");
    let mut file = read_at(path)?;
    let history = file.entry(project).or_default();
    history.retain(still_on_disk);
    record(history, added);
    write_at(path, &file)
}

/// The project a session is on, which its history is kept under: the repo it reviews.
#[cfg(not(target_arch = "wasm32"))]
fn project_of(state: &crate::api::AppState, session_id: &str) -> Result<String> {
    crate::api::with_session(state, session_id, |session| {
        Ok(session.repo_path.display().to_string())
    })
}

/// Whether what an entry opens is still there: a file or a line is gone with its file. The
/// rest are not on disk, and are the window's to judge.
#[cfg(not(target_arch = "wasm32"))]
fn still_on_disk(entry: &HistoryEntry) -> bool {
    match &entry.thing {
        HistoryThing::File {
            repo_path,
            file_path,
        }
        | HistoryThing::Line {
            repo_path,
            file_path,
            ..
        } => std::path::Path::new(repo_path).join(file_path).is_file(),
        HistoryThing::Command { .. }
        | HistoryThing::Shell { .. }
        | HistoryThing::QuickTask { .. } => true,
    }
}

/// No file yet reads as no history; one that is there has to parse, or a write would throw
/// away every other project's.
#[cfg(not(target_arch = "wasm32"))]
fn read_at(path: &std::path::Path) -> Result<HistoryFile> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HistoryFile::new());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_at(path: &std::path::Path, file: &HistoryFile) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(file)?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(line: &str, ran_at_unix: u64) -> HistoryEntry {
        HistoryEntry {
            thing: HistoryThing::Shell {
                line: line.to_string(),
            },
            ran_at_unix,
        }
    }

    fn recorded(history: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
        record(
            history,
            HistoryRecord {
                entry,
                gone: Vec::new(),
            },
        );
    }

    fn lines_of(history: &[HistoryEntry]) -> Vec<&str> {
        history
            .iter()
            .map(|entry| match &entry.thing {
                HistoryThing::Shell { line } => line.as_str(),
                other => panic!("only lines of shell were recorded, not {other:?}"),
            })
            .collect()
    }

    /// Running a thing again moves its one entry to the newest place, with its new time.
    #[test]
    fn running_again_moves_the_entry_to_the_newest() {
        let mut history = Vec::new();
        recorded(&mut history, shell("git status", 1));
        recorded(&mut history, shell("ls", 2));
        recorded(&mut history, shell("git status", 3));

        assert_eq!(lines_of(&history), ["ls", "git status"]);
        assert_eq!(history[1].ran_at_unix, 3);
    }

    #[test]
    fn the_oldest_go_past_the_hundred_kept() {
        let mut history = Vec::new();
        for at in 0..HISTORY_KEPT as u64 + 5 {
            recorded(&mut history, shell(&format!("echo {at}"), at));
        }

        assert_eq!(history.len(), HISTORY_KEPT);
        assert_eq!(lines_of(&history)[0], "echo 5");
    }

    /// A command is its action: a reworded title is still the same entry.
    #[test]
    fn things_are_known_by_what_they_run() {
        let command = |action: &str, title: &str| HistoryThing::Command {
            action: action.to_string(),
            title: title.to_string(),
            description: String::new(),
        };
        assert!(command("OpenWorkLog", "wl").same_as(&command("OpenWorkLog", "work log")));
        assert!(!command("OpenWorkLog", "wl").same_as(&command("OpenWire", "wl")));
    }

    #[test]
    fn gone_things_go_with_the_next_record() {
        let mut history = vec![shell("a", 1), shell("b", 2)];
        record(
            &mut history,
            HistoryRecord {
                entry: shell("c", 3),
                gone: vec![shell("a", 0).thing],
            },
        );

        assert_eq!(lines_of(&history), ["b", "c"]);
    }
}
