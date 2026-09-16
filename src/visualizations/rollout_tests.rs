use std::{fs, io::Write};

use super::*;

/// A thread started on 2026-09-16 at 15:08:29 UTC.
const THREAD_ID: &str = "01a0aac3-1f62-7012-bcea-58307a8d2586";
/// 2026-09-16T15:00:00Z: before every line these tests write, unless a test says otherwise.
const STARTED_AT_UNIX: u64 = 1_789_570_800;

struct Home {
    codex_home: PathBuf,
    rollout: PathBuf,
    thread_dir: PathBuf,
}

/// A Codex home with one rollout begun by a thread's meta line, and a chart fragment written
/// in the thread's folder.
fn home_with_rollout(source: serde_json::Value) -> Home {
    let codex_home = std::env::temp_dir().join(format!(
        "moon-codex-home-{}",
        crate::moontasks::store::new_uuid()
    ));
    let sessions = codex_home.join("sessions/2026/09/16");
    fs::create_dir_all(&sessions).unwrap();
    let thread_dir = codex_home.join("visualizations/2026/09/16").join(THREAD_ID);
    fs::create_dir_all(&thread_dir).unwrap();
    fs::write(thread_dir.join("chart.html"), "<div id=\"widget\"></div>").unwrap();
    let codex_home = fs::canonicalize(codex_home).unwrap();
    let rollout = codex_home.join(format!(
        "sessions/2026/09/16/rollout-2026-09-16T11-08-29-{THREAD_ID}.jsonl"
    ));
    append(
        &rollout,
        &serde_json::json!({
            "timestamp": "2026-09-16T15:08:36.971Z",
            "type": "session_meta",
            "payload": { "id": THREAD_ID, "cwd": "/repo", "source": source },
        })
        .to_string(),
    );
    Home {
        thread_dir: fs::canonicalize(thread_dir).unwrap(),
        codex_home,
        rollout,
    }
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    write!(file, "{text}\n").unwrap();
}

fn answer(timestamp: &str, text: &str) -> String {
    serde_json::json!({
        "timestamp": timestamp,
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text }],
            "phase": "final_answer",
        },
    })
    .to_string()
}

#[test]
fn an_answer_announcing_a_fragment_is_picked_up() {
    let home = home_with_rollout(serde_json::json!("cli"));
    append(
        &home.rollout,
        &answer(
            "2026-09-16T15:09:00.000Z",
            "Here it is.\n\n::codex-inline-vis{file=\"chart.html\"}\n",
        ),
    );
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);

    rollouts.follow(vec![home.rollout.clone()]);

    assert_eq!(
        rollouts.announced().clone(),
        BTreeMap::from([(home.thread_dir.join("chart.html"), 1)])
    );
}

#[test]
fn a_fragment_announced_again_counts_again() {
    let home = home_with_rollout(serde_json::json!("cli"));
    let directive = "::codex-inline-vis{file=\"chart.html\"}";
    append(
        &home.rollout,
        &answer("2026-09-16T15:09:00.000Z", directive),
    );
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);
    rollouts.follow(vec![home.rollout.clone()]);

    append(
        &home.rollout,
        &answer("2026-09-16T15:10:00.000Z", directive),
    );
    rollouts.follow(vec![home.rollout.clone()]);

    assert_eq!(rollouts.announced()[&home.thread_dir.join("chart.html")], 2);
}

#[test]
fn an_answer_from_before_the_terminal_started_is_history() {
    let home = home_with_rollout(serde_json::json!("cli"));
    append(
        &home.rollout,
        &answer(
            "2026-09-16T14:59:59.000Z",
            "::codex-inline-vis{file=\"chart.html\"}",
        ),
    );
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);

    rollouts.follow(vec![home.rollout.clone()]);

    assert!(rollouts.announced().is_empty());
}

#[test]
fn a_line_still_being_written_waits_for_its_end() {
    let home = home_with_rollout(serde_json::json!("cli"));
    let line = answer(
        "2026-09-16T15:09:00.000Z",
        "::codex-inline-vis{file=\"chart.html\"}",
    );
    let (first, rest) = line.split_at(line.len() / 2);
    fs::OpenOptions::new()
        .append(true)
        .open(&home.rollout)
        .unwrap()
        .write_all(first.as_bytes())
        .unwrap();
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);
    rollouts.follow(vec![home.rollout.clone()]);
    assert!(rollouts.announced().is_empty());

    append(&home.rollout, rest);
    rollouts.follow(vec![home.rollout.clone()]);

    assert_eq!(rollouts.announced().len(), 1);
}

#[test]
fn a_subagent_announces_nothing() {
    let home = home_with_rollout(serde_json::json!({ "subagent": { "thread_spawn": {} } }));
    append(
        &home.rollout,
        &answer(
            "2026-09-16T15:09:00.000Z",
            "::codex-inline-vis{file=\"chart.html\"}",
        ),
    );
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);

    rollouts.follow(vec![home.rollout.clone()]);

    assert!(rollouts.announced().is_empty());
}

#[test]
fn a_rollout_let_go_of_is_read_to_its_end_once_more() {
    let home = home_with_rollout(serde_json::json!("cli"));
    let mut rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);
    rollouts.follow(vec![home.rollout.clone()]);

    // The thread's last answer lands as Codex moves to another thread.
    append(
        &home.rollout,
        &answer(
            "2026-09-16T15:09:00.000Z",
            "::codex-inline-vis{file=\"chart.html\"}",
        ),
    );
    rollouts.follow(Vec::new());

    assert_eq!(rollouts.announced().len(), 1);
    assert!(rollouts.followed.is_empty());
}

#[test]
fn only_rollouts_in_the_codex_home_are_followed() {
    let home = home_with_rollout(serde_json::json!("cli"));
    let rollouts = CodexRollouts::new(&home.codex_home, STARTED_AT_UNIX);

    assert!(rollouts.is_rollout(&home.rollout));
    assert!(!rollouts.is_rollout(&home.codex_home.join("state_5.sqlite")));
    assert!(!rollouts.is_rollout(Path::new("/tmp/rollout-x.jsonl")));
}
