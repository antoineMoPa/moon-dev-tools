//! The sessions each agent already has for a repo, read from where that agent keeps them.
//!
//! A task resource remembers the session id its agent was started with, but that id stops
//! meaning anything when the user switches sessions inside the agent or the agent never
//! persisted it. This is the way back: list what the agents actually have on disk, so a
//! task can be attached to one of them.

use std::{
    io::BufRead,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::api::{AgentAvailability, AgentKind, AppState};

/// One session an agent has on disk, as the attach modal lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AgentSessionView {
    pub(crate) agent: AgentKind,
    /// The id the agent itself resumes the session by.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) updated_at_unix: u64,
}

/// How many sessions each agent contributes. The modal is for finding the one that was just
/// lost, not for browsing history.
const RECENT_PER_AGENT: usize = 12;

/// How many lines of a session transcript are read looking for something to title it with.
const TITLE_SCAN_LINES: usize = 60;

/// How many characters of a title the modal keeps.
const TITLE_MAX_CHARS: usize = 80;

/// How many codex transcript files are opened looking for ones from this repo. Codex keeps
/// every repo's sessions in one tree, so the newest files are read until enough match.
const CODEX_FILE_SCAN_CAP: usize = 200;

/// The recent sessions of every installed agent, for the repo this review session is on.
pub(crate) fn list_for_session(
    state: &AppState,
    session_id: &str,
) -> Result<Vec<AgentSessionView>> {
    let repo_path = crate::api::with_session(state, session_id, |session| {
        Ok(session.repo_path.clone())
    })?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set, so there is nowhere to read agent sessions from")?;
    list_recent(&repo_path, &home, state.agent_availability)
}

/// The recent sessions of every installed agent, newest first across all of them.
fn list_recent(
    repo_path: &Path,
    home: &Path,
    availability: AgentAvailability,
) -> Result<Vec<AgentSessionView>> {
    let mut sessions = Vec::new();
    if availability.claude {
        sessions.extend(claude_sessions(repo_path, home)?);
    }
    if availability.codex {
        sessions.extend(codex_sessions(repo_path, home)?);
    }
    if availability.opencode {
        sessions.extend(opencode_sessions(repo_path)?);
    }
    sessions.sort_by(|a, b| b.updated_at_unix.cmp(&a.updated_at_unix));
    Ok(sessions)
}

/// A file's mtime as unix seconds, which is when its session was last written to.
fn mtime_unix(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// The `.jsonl` files of a directory, newest first.
fn transcripts_newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // No directory means the agent has never run here, which is not an error.
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    files.sort_by_key(|path| std::cmp::Reverse(mtime_unix(path)));
    files
}

/// A title cut down to one line the modal has room for.
fn tidy_title(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= TITLE_MAX_CHARS {
        return line.to_string();
    }
    line.chars().take(TITLE_MAX_CHARS - 1).collect::<String>() + "…"
}

// --- Claude ---------------------------------------------------------------------------------

/// The directory name Claude keeps a repo's sessions under: the repo path with every
/// character that is not a letter or digit turned into `-`.
fn claude_project_dir_name(repo_path: &Path) -> String {
    repo_path
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn claude_sessions(repo_path: &Path, home: &Path) -> Result<Vec<AgentSessionView>> {
    let dir = home
        .join(".claude")
        .join("projects")
        .join(claude_project_dir_name(repo_path));

    let mut sessions = Vec::new();
    for path in transcripts_newest_first(&dir).into_iter().take(RECENT_PER_AGENT) {
        let Some(id) = path.file_stem().map(|stem| stem.to_string_lossy().to_string()) else {
            continue;
        };
        let file = std::fs::File::open(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let lines = std::io::BufReader::new(file)
            .lines()
            .map_while(|line| line.ok());
        let Some(title) = claude_session_title(lines) else {
            // A transcript with nothing said in it is not a session worth going back to.
            continue;
        };
        sessions.push(AgentSessionView {
            agent: AgentKind::Claude,
            id,
            title,
            updated_at_unix: mtime_unix(&path),
        });
    }
    Ok(sessions)
}

/// What a Claude transcript is about: its summary line if it has one, else the first thing
/// the user said. `None` when nothing was ever said.
fn claude_session_title(lines: impl Iterator<Item = String>) -> Option<String> {
    for line in lines.take(TITLE_SCAN_LINES) {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match record["type"].as_str() {
            Some("summary") => {
                if let Some(summary) = record["summary"].as_str() {
                    return Some(tidy_title(summary));
                }
            }
            Some("user") => {
                let content = &record["message"]["content"];
                let text = content.as_str().map(str::to_string).or_else(|| {
                    content.as_array().and_then(|parts| {
                        parts
                            .iter()
                            .filter(|part| part["type"].as_str() == Some("text"))
                            .find_map(|part| part["text"].as_str().map(str::to_string))
                    })
                });
                // Tool results, slash-command records and the like are user turns too, but
                // say nothing about what the session is for.
                if let Some(text) = text.filter(|text| !text.starts_with('<')) {
                    return Some(tidy_title(&text));
                }
            }
            _ => {}
        }
    }
    None
}

// --- Codex ----------------------------------------------------------------------------------

fn codex_sessions(repo_path: &Path, home: &Path) -> Result<Vec<AgentSessionView>> {
    let root = home.join(".codex").join("sessions");
    let mut files = Vec::new();
    collect_jsonl_files(&root, &mut files);
    files.sort_by_key(|path| std::cmp::Reverse(mtime_unix(path)));

    let mut sessions = Vec::new();
    for path in files.into_iter().take(CODEX_FILE_SCAN_CAP) {
        if sessions.len() == RECENT_PER_AGENT {
            break;
        }
        let file = std::fs::File::open(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut lines = std::io::BufReader::new(file)
            .lines()
            .map_while(|line| line.ok());
        let Some(id) = lines
            .next()
            .and_then(|meta| codex_session_id(&meta, repo_path))
        else {
            continue;
        };
        sessions.push(AgentSessionView {
            agent: AgentKind::Codex,
            id,
            title: codex_session_title(lines)
                .unwrap_or_else(|| "(nothing said yet)".to_string()),
            updated_at_unix: mtime_unix(&path),
        });
    }
    Ok(sessions)
}

/// Every `.jsonl` under `dir`, however deep — codex files sessions by date, `YYYY/MM/DD/`.
fn collect_jsonl_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "jsonl") {
            into.push(path);
        }
    }
}

/// The session id out of a codex transcript's first line, if that session was run in this
/// repo. The first line is the `session_meta` record, which names both.
fn codex_session_id(meta_line: &str, repo_path: &Path) -> Option<String> {
    let record = serde_json::from_str::<serde_json::Value>(meta_line).ok()?;
    if record["type"].as_str() != Some("session_meta") {
        return None;
    }
    if Path::new(record["payload"]["cwd"].as_str()?) != repo_path {
        return None;
    }
    record["payload"]["id"].as_str().map(str::to_string)
}

/// The first thing the user said in a codex transcript. Codex records no summary, so that
/// is what there is to title a session with.
fn codex_session_title(lines: impl Iterator<Item = String>) -> Option<String> {
    for line in lines.take(TITLE_SCAN_LINES) {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = &record["payload"];
        let text = match record["type"].as_str() {
            Some("event_msg") if payload["type"].as_str() == Some("user_message") => {
                payload["message"].as_str()
            }
            Some("response_item")
                if payload["type"].as_str() == Some("message")
                    && payload["role"].as_str() == Some("user") =>
            {
                payload["content"].as_array().and_then(|parts| {
                    parts
                        .iter()
                        .filter(|part| part["type"].as_str() == Some("input_text"))
                        .find_map(|part| part["text"].as_str())
                })
            }
            _ => None,
        };
        // Codex wraps its own context notes — `<user_instructions>`,
        // `<environment_context>` — as user turns; what the user typed does not start
        // with a tag.
        if let Some(text) = text.filter(|text| !text.starts_with('<')) {
            return Some(tidy_title(text));
        }
    }
    None
}

// --- OpenCode -------------------------------------------------------------------------------

/// One row of `opencode session list --format json`.
#[derive(Deserialize)]
struct OpenCodeSessionRow {
    id: String,
    title: String,
    /// Milliseconds since the epoch.
    updated: u64,
    directory: String,
}

fn opencode_sessions(repo_path: &Path) -> Result<Vec<AgentSessionView>> {
    let output = Command::new("opencode")
        .args(["session", "list", "--format", "json"])
        // The PATH the availability check found opencode on — see [`crate::shell_path`].
        .env("PATH", crate::shell_path::agent_path())
        .output()
        .context("failed to run opencode session list")?;
    if !output.status.success() {
        bail!(
            "opencode session list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let rows: Vec<OpenCodeSessionRow> = serde_json::from_slice(&output.stdout)
        .context("could not decode opencode session list output")?;
    Ok(opencode_rows_for_repo(rows, repo_path))
}

/// The rows that belong to this repo, newest kept, as the modal shows them. OpenCode lists
/// every project's sessions in one answer, so the repo filter happens here.
fn opencode_rows_for_repo(
    rows: Vec<OpenCodeSessionRow>,
    repo_path: &Path,
) -> Vec<AgentSessionView> {
    rows.into_iter()
        .filter(|row| Path::new(&row.directory) == repo_path)
        .take(RECENT_PER_AGENT)
        .map(|row| AgentSessionView {
            agent: AgentKind::OpenCode,
            id: row.id,
            title: tidy_title(&row.title),
            updated_at_unix: row.updated / 1000,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_keeps_a_repo_s_sessions_under_its_dashed_path() {
        assert_eq!(
            claude_project_dir_name(Path::new("/home/dev/prog/moonreview/crates/egui_tty")),
            "-home-dev-prog-moonreview-crates-egui-tty"
        );
    }

    #[test]
    fn a_claude_session_is_titled_by_its_summary_when_it_has_one() {
        let lines = [
            r#"{"type":"summary","summary":"Fixing the login page","leafUuid":"x"}"#,
            r#"{"type":"user","message":{"role":"user","content":"meow"}}"#,
        ];

        let title = claude_session_title(lines.iter().map(|line| line.to_string()));

        assert_eq!(title.as_deref(), Some("Fixing the login page"));
    }

    #[test]
    fn a_claude_session_with_no_summary_is_titled_by_what_the_user_said_first() {
        let lines = [
            r#"{"type":"file-history-snapshot","messageId":"m"}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"please fix the parser\nand the lexer"}}"#,
        ];

        let title = claude_session_title(lines.iter().map(|line| line.to_string()));

        // The first line of the message: the modal has one line of room.
        assert_eq!(title.as_deref(), Some("please fix the parser"));
    }

    #[test]
    fn a_claude_transcript_nobody_spoke_in_has_no_title() {
        let lines = [r#"{"type":"file-history-snapshot","messageId":"m"}"#];

        let title = claude_session_title(lines.iter().map(|line| line.to_string()));

        assert_eq!(title, None);
    }

    #[test]
    fn a_codex_transcript_names_its_session_and_repo_on_its_first_line() {
        let meta = r#"{"timestamp":"t","type":"session_meta","payload":{"id":"019e-abc","cwd":"/repo","originator":"codex-tui"}}"#;

        assert_eq!(
            codex_session_id(meta, Path::new("/repo")).as_deref(),
            Some("019e-abc")
        );
        assert_eq!(codex_session_id(meta, Path::new("/other")), None);
    }

    #[test]
    fn a_codex_session_is_titled_by_the_first_thing_the_user_typed() {
        let lines = [
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_instructions>be nice</user_instructions>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"rewrite the scheduler"}]}}"#,
        ];

        let title = codex_session_title(lines.iter().map(|line| line.to_string()));

        assert_eq!(title.as_deref(), Some("rewrite the scheduler"));
    }

    #[test]
    fn opencode_sessions_of_other_repos_are_left_out() {
        let rows = vec![
            OpenCodeSessionRow {
                id: "ses_here".to_string(),
                title: "Fix the board".to_string(),
                updated: 1_786_388_687_379,
                directory: "/repo".to_string(),
            },
            OpenCodeSessionRow {
                id: "ses_elsewhere".to_string(),
                title: "Another project".to_string(),
                updated: 1_786_388_687_379,
                directory: "/other".to_string(),
            },
        ];

        let sessions = opencode_rows_for_repo(rows, Path::new("/repo"));

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ses_here");
        assert_eq!(sessions[0].title, "Fix the board");
        // Milliseconds on the wire, seconds everywhere in moonreview.
        assert_eq!(sessions[0].updated_at_unix, 1_786_388_687);
    }

    #[test]
    fn a_title_is_cut_to_one_line_the_modal_has_room_for() {
        let long = "a".repeat(200);

        assert_eq!(tidy_title(&long).chars().count(), TITLE_MAX_CHARS);
        assert!(tidy_title(&long).ends_with('…'));
        assert_eq!(tidy_title("short\nrest"), "short");
    }
}
