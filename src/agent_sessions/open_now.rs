//! The sessions an agent has open right now on this machine, read from where the agent itself
//! writes them down.
//!
//! A run on a task is going when a shell of this moon has it, and that is all a moon can see
//! of its own. But a session can be open with no shell of any moon behind it - an agent
//! started in a terminal of the person's, which put itself on a task - and then the agent's
//! own word is the only word there is that it is going.

use std::path::Path;

use serde::Deserialize;

use crate::api::AgentKind;

/// How one agent writes down the sessions it has open.
struct OpenSessionRecords {
    agent: AgentKind,
    /// The folder it keeps a file per open session in, from the home directory.
    dir: &'static str,
    /// The session a file of that folder is about, and the process that has it open.
    read: fn(&str) -> Option<(String, u32)>,
}

/// The agents that write their open sessions down. One that is not here says nothing about
/// what it has open, so a run of it is only ever known to be going by its shell.
const OPEN_SESSION_RECORDS: &[OpenSessionRecords] = &[OpenSessionRecords {
    agent: AgentKind::Claude,
    dir: ".claude/sessions",
    read: claude_record,
}];

/// Claude keeps `<pid>.json` for as long as that process has a session open, and rewrites it
/// when the session inside is swapped for another.
fn claude_record(text: &str) -> Option<(String, u32)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Record {
        pid: u32,
        session_id: String,
    }
    let record: Record = serde_json::from_str(text).ok()?;
    Some((record.session_id, record.pid))
}

/// Every session the agents say they have open, and the process each is open in.
///
/// As the agents wrote it: a file left behind by an agent that was killed still names its
/// process, so whoever asks checks the process is there.
#[derive(Default)]
pub(crate) struct OpenSessions {
    open: Vec<(AgentKind, String, u32)>,
}

impl OpenSessions {
    /// Read them all. None for an account with no home directory.
    pub(crate) fn read() -> Self {
        match std::env::var_os("HOME").filter(|home| !home.is_empty()) {
            Some(home) => Self::read_under(Path::new(&home)),
            None => Self::default(),
        }
    }

    /// These sessions, each said to be open in that process, for a test of what is made of it.
    #[cfg(test)]
    pub(crate) fn said_open(open: &[(AgentKind, &str, u32)]) -> Self {
        Self {
            open: open
                .iter()
                .map(|(agent, session_id, pid)| (*agent, session_id.to_string(), *pid))
                .collect(),
        }
    }

    fn read_under(home: &Path) -> Self {
        let mut open = Vec::new();
        for records in OPEN_SESSION_RECORDS {
            // An agent that has never been run has no folder, and a file being rewritten as
            // it is read is a session that will be read next time.
            let Ok(entries) = std::fs::read_dir(home.join(records.dir)) else {
                continue;
            };
            for entry in entries.flatten() {
                let is_json = entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json");
                if !is_json {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                if let Some((session_id, pid)) = (records.read)(&text) {
                    open.push((records.agent, session_id, pid));
                }
            }
        }
        Self { open }
    }

    /// The process an agent says it has this session open in.
    pub(crate) fn process_of(&self, agent: AgentKind, session_id: &str) -> Option<u32> {
        self.open
            .iter()
            .find(|(of, open, _)| *of == agent && open == session_id)
            .map(|(_, _, pid)| *pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claudes_open_sessions_are_read_from_its_own_records() {
        let home = std::env::temp_dir().join(format!(
            "moonreview-open-sessions-{}-claude",
            std::process::id()
        ));
        let sessions = home.join(".claude/sessions");
        std::fs::create_dir_all(&sessions).expect("expected the folder");
        std::fs::write(
            sessions.join("4242.json"),
            r#"{"pid":4242,"sessionId":"a-session","cwd":"/repo","status":"busy"}"#,
        )
        .expect("expected the record");
        // Beside each record claude keeps a key, which is no session.
        std::fs::write(sessions.join("4242.abc.key"), "secret").expect("expected the key");
        std::fs::write(sessions.join("half.json"), "{").expect("expected the half record");

        let open = OpenSessions::read_under(&home);

        assert_eq!(open.process_of(AgentKind::Claude, "a-session"), Some(4242));
        assert_eq!(open.process_of(AgentKind::Codex, "a-session"), None);
        assert_eq!(open.process_of(AgentKind::Claude, "another"), None);

        std::fs::remove_dir_all(home).expect("failed to remove the test home");
    }
}
