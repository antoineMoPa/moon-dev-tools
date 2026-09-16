//! Following the rollout a terminal's Codex writes, for the visualizations its answers announce.
//!
//! Which rollout is the terminal's is read off the process: the one Codex holds open - see
//! [`super::open_files`]. Neither the terminal's folder nor the time it started would do: two
//! Codex runs in one repo started together look the same from there, and a thread switched
//! inside Codex (`/new`, `/resume`) moves to a file neither names.
//!
//! Each poll reads what has been appended to every rollout the process has open, and to the
//! ones it had open last time - a thread just left may have finished its last line on the way
//! out. An answer written before the terminal started is not an announcement: a resumed
//! thread's old answers are history, not something the agent is showing now.

use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use super::directives::{ThreadVisualizations, directive_files};

/// Everything one terminal's Codex has announced, and how far each of its rollouts is read.
pub(crate) struct CodexRollouts {
    /// The Codex home, as the kernel names the files in it.
    codex_home: PathBuf,
    /// When the terminal started, in seconds since the epoch.
    started_at_unix: u64,
    followed: HashMap<PathBuf, FollowedRollout>,
    /// Each fragment announced, with how many times.
    announced: BTreeMap<PathBuf, u64>,
}

struct FollowedRollout {
    /// The byte after the last whole line read.
    read_to: u64,
    thread: RolloutThread,
}

/// Whose rollout it is, which its first line says.
enum RolloutThread {
    /// The first line has not been read yet.
    Unread,
    /// A thread someone talks to, and the folder its visualizations go in.
    Conversation(ThreadVisualizations),
    /// A subagent's thread, or one whose id names no folder: nothing it says is shown to a
    /// person, so nothing it announces is either.
    Ignored,
}

impl CodexRollouts {
    pub(crate) fn new(codex_home: &Path, started_at_unix: u64) -> Self {
        Self {
            // A home that is not there yet has no rollouts to match; it is looked for again as
            // the process opens one, which creates it.
            codex_home: std::fs::canonicalize(codex_home)
                .unwrap_or_else(|_| codex_home.to_path_buf()),
            started_at_unix,
            followed: HashMap::new(),
            announced: BTreeMap::new(),
        }
    }

    /// Read what the Codex in this process tree has written since the last poll, and answer
    /// every fragment it has announced so far with how many times.
    pub(crate) fn poll(&mut self, pid: u32) -> &BTreeMap<PathBuf, u64> {
        let open: Vec<PathBuf> = super::open_files::open_in_process_tree(pid)
            .into_iter()
            .filter(|path| self.is_rollout(path))
            .collect();
        self.follow(open);
        self.announced()
    }

    pub(crate) fn announced(&self) -> &BTreeMap<PathBuf, u64> {
        &self.announced
    }

    /// Read the rollouts open now to their end, and the ones that are not for the last time.
    fn follow(&mut self, open: Vec<PathBuf>) {
        for path in &open {
            self.followed
                .entry(path.clone())
                .or_insert(FollowedRollout {
                    read_to: 0,
                    thread: RolloutThread::Unread,
                });
        }
        let paths: Vec<PathBuf> = self.followed.keys().cloned().collect();
        for path in paths {
            let mut rollout = self.followed.remove(&path).expect("listed just above");
            for line in read_new_lines(&path, &mut rollout.read_to) {
                self.read_line(&mut rollout.thread, &line);
            }
            if open.contains(&path) {
                self.followed.insert(path, rollout);
            }
        }
    }

    fn is_rollout(&self, path: &Path) -> bool {
        path.starts_with(self.codex_home.join("sessions"))
            && path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("rollout-") && name.ends_with(".jsonl")
            })
    }

    fn read_line(&mut self, thread: &mut RolloutThread, line: &str) {
        // A line Codex writes is JSON; one that is not was not written by this Codex, and says
        // nothing moon can read.
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let payload = &record["payload"];
        if let RolloutThread::Unread = thread {
            // A subagent's `source` is an object naming its parent; a thread a person started
            // names where it was started from.
            assert_eq!(
                record["type"].as_str(),
                Some("session_meta"),
                "a rollout starts with its session's meta"
            );
            *thread = match (payload["id"].as_str(), payload["source"].is_string()) {
                (Some(thread_id), true) => ThreadVisualizations::new(&self.codex_home, thread_id)
                    .map_or(RolloutThread::Ignored, RolloutThread::Conversation),
                _ => RolloutThread::Ignored,
            };
            return;
        }
        let RolloutThread::Conversation(visualizations) = thread else {
            return;
        };
        if record["type"].as_str() != Some("response_item")
            || payload["type"].as_str() != Some("message")
            || payload["role"].as_str() != Some("assistant")
            || !written_since(&record, self.started_at_unix)
        {
            return;
        }
        let text: String = payload["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|part| part["type"].as_str() == Some("output_text"))
            .filter_map(|part| part["text"].as_str())
            .collect();
        for file in directive_files(&text) {
            if let Some(fragment_path) = visualizations.fragment_for(&file) {
                *self.announced.entry(fragment_path).or_default() += 1;
            }
        }
    }
}

/// Whether a rollout line was written at or after this second. Every line Codex writes carries
/// its time.
fn written_since(record: &serde_json::Value, since_unix: u64) -> bool {
    let timestamp = record["timestamp"]
        .as_str()
        .expect("every rollout line carries a timestamp");
    let written =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
            .expect("a rollout timestamp is RFC 3339");
    written.unix_timestamp() >= since_unix as i64
}

/// The whole lines appended to a file since `read_to`, moving it past them. A line still being
/// written is left for the next read.
fn read_new_lines(path: &Path, read_to: &mut u64) -> Vec<String> {
    // A rollout removed since it was listed has nothing more to give.
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    file.seek(SeekFrom::Start(*read_to))
        .expect("a file can be read from any offset");
    let mut appended = Vec::new();
    file.read_to_end(&mut appended)
        .expect("an open rollout can be read");
    let Some(last_newline) = appended.iter().rposition(|byte| *byte == b'\n') else {
        return Vec::new();
    };
    *read_to += last_newline as u64 + 1;
    String::from_utf8_lossy(&appended[..last_newline])
        .lines()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[path = "rollout_tests.rs"]
mod tests;
