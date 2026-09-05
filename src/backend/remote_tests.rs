//! The client/server mode, end to end: a real server on a real socket, driven through
//! [`RemoteBackend`] exactly as the window drives it.
//!
//! This is the mode where the repo is on another machine, so nothing here may reach into the
//! server's state directly - every assertion goes over HTTP or the terminal websocket.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::{
    api::OpenSessionRequest,
    backend::{Backend, remote::RemoteBackend},
    git::run_git_no_output,
    moontasks::{ColumnEnd, ColumnId, CreateTaskRequest},
};

struct ServedRepo {
    root: PathBuf,
    base_url: String,
}

impl Drop for ServedRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Start a `moonreview serve` on a free port, over a throwaway repo with pending changes.
fn serve_a_repo(name: &str) -> ServedRepo {
    let root =
        std::env::temp_dir().join(format!("moonreview-remote-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("failed to create the fixture directory");

    run_git_no_output(&root, &["init"]).expect("failed to init the fixture repo");
    for (key, value) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test User"),
        ("commit.gpgsign", "false"),
    ] {
        run_git_no_output(&root, &["config", key, value]).expect("failed to configure git");
    }
    fs::write(
        root.join("main.rs"),
        "fn main() {\n    println!(\"one\");\n}\n",
    )
    .expect("failed to write the fixture file");
    run_git_no_output(&root, &["add", "-A"]).expect("failed to stage the fixture");
    run_git_no_output(&root, &["commit", "-m", "first"]).expect("failed to commit the fixture");
    fs::write(
        root.join("main.rs"),
        "fn main() {\n    println!(\"two\");\n}\n",
    )
    .expect("failed to change the fixture file");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let (port_sender, port_receiver) = std::sync::mpsc::channel();

    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build the test runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("failed to bind a test port");
            let port = listener
                .local_addr()
                .expect("failed to read the test port")
                .port();
            port_sender.send(port).expect("failed to report the port");
            let _ = crate::server::serve_on(state, listener, None).await;
        });
    });

    let port = port_receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the test server never reported a port");

    ServedRepo {
        root,
        base_url: format!("http://127.0.0.1:{port}"),
    }
}

#[test]
fn a_remote_review_loads_its_diff_over_http() {
    let served = serve_a_repo("state");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");

    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");
    let payload = backend
        .session_state(&opened.session_id)
        .expect("expected the remote review state");

    assert_eq!(payload.hunks.len(), 1, "expected one changed hunk");
    let hunk = &payload.hunks[0];
    assert_eq!(hunk.file_path, "main.rs");
    assert!(hunk.patch_preview.contains("println!(\"two\")"));
    assert!(!payload.read_only, "a working-tree review is writable");
    assert_eq!(backend.describe(), served.base_url.replace("http://", ""));
}

#[test]
fn staging_through_a_remote_review_changes_the_repo() {
    let served = serve_a_repo("stage");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let hunk_id = backend
        .session_state(&opened.session_id)
        .expect("expected the remote review state")
        .hunks
        .first()
        .map(|hunk| hunk.id.clone())
        .expect("expected a hunk to stage");
    backend
        .stage_hunk(&opened.session_id, &hunk_id)
        .expect("expected the remote stage to succeed");

    let staged = backend
        .session_state(&opened.session_id)
        .expect("expected the remote review state")
        .hunks
        .iter()
        .filter(|hunk| hunk.staged)
        .count();
    assert_eq!(staged, 1, "the hunk should now be staged on the server");
}

#[test]
fn a_remote_shell_carries_bytes_both_ways_over_the_websocket() {
    let served = serve_a_repo("shell");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let terminal_id = backend
        .create_terminal(&opened.session_id, None)
        .expect("expected a remote shell to start");
    assert!(
        backend
            .list_terminals(&opened.session_id)
            .expect("expected the remote shell list")
            .contains(&terminal_id)
    );

    let attachment = backend
        .attach_terminal(&opened.session_id, &terminal_id)
        .expect("expected to attach to the remote shell");
    attachment
        .tty
        .write(b"printf 'remote-ok\\n'\n")
        .expect("expected to write to the remote shell");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(chunk) = attachment.output.recv_timeout(Duration::from_millis(200)) {
            seen.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&seen).contains("remote-ok") {
                break;
            }
        }
    }

    let transcript = String::from_utf8_lossy(&seen);
    assert!(
        transcript.contains("remote-ok"),
        "the remote shell's output never arrived; got:\n{transcript}"
    );

    backend
        .close_terminal(&opened.session_id, &terminal_id)
        .expect("expected the remote shell to close");
}

/// The card's notes and the file pane read the same file: opening the notes makes it real,
/// the file pane's own write path edits it, and the board's next read shows what was written.
#[test]
fn task_notes_round_trip_over_http() {
    let served = serve_a_repo("notes");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let task = backend
        .create_task(
            &opened.session_id,
            &CreateTaskRequest {
                title: "Fix the login page".to_string(),
                status: ColumnId::new("todo"),
                joins: ColumnEnd::Top,
            },
        )
        .expect("expected the remote task to be created");
    assert_eq!(task.notes, "", "a new task has nothing written yet");

    let path = backend
        .open_task_notes(&opened.session_id, &task.id)
        .expect("expected the notes to open");
    assert_eq!(path, format!(".moontasks/{}/notes.md", task.id));

    // The pane saves through the same write every other file uses.
    backend
        .write_file(&opened.session_id, &path, "what the fix is about\n")
        .expect("expected the file pane's write to reach the notes");

    let tasks = backend
        .list_tasks(&opened.session_id)
        .expect("expected the remote task list");
    assert_eq!(tasks[0].notes, "what the fix is about\n");
    let content = backend
        .file_content(&opened.session_id, &path)
        .expect("expected the file pane's read to find the notes");
    assert_eq!(content.content, "what the fix is about\n");
}

#[test]
fn an_unreachable_address_fails_with_the_address_in_the_message() {
    // Port 1 is reserved and nothing listens there, so this is a connection refusal.
    let error = match RemoteBackend::connect("127.0.0.1:1") {
        Ok(_) => panic!("expected the connect to fail"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("127.0.0.1:1"),
        "the error should name what it could not reach: {error}"
    );
}

/// A file linked over the wire is on the card the next time the board is read, by the same
/// path the file pane then opens it with.
#[test]
fn a_linked_file_round_trips_over_http() {
    let served = serve_a_repo("task-files");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");
    let task = backend
        .create_task(
            &opened.session_id,
            &CreateTaskRequest {
                title: "Fix the login page".to_string(),
                status: ColumnId::new("todo"),
                joins: ColumnEnd::Top,
            },
        )
        .expect("expected the remote task to be created");

    backend
        .link_task_file(&opened.session_id, &task.id, "main.rs")
        .expect("expected the file to be linked");
    assert!(
        backend
            .link_task_file(&opened.session_id, &task.id, "missing.rs")
            .is_err(),
        "a file that is not in the repo has no place on a card"
    );

    let tasks = backend
        .list_tasks(&opened.session_id)
        .expect("expected the remote task list");
    let linked = &tasks[0].resources[0];
    assert_eq!(linked.kind, crate::moontasks::TaskResourceKind::File);
    assert_eq!(linked.file_path.as_deref(), Some("main.rs"));
    let content = backend
        .file_content(&opened.session_id, "main.rs")
        .expect("expected the file pane's read to find the linked file");
    assert!(content.content.contains("fn main()"));
}

/// A shell's name goes over HTTP like everything else: read on the way to attaching, and
/// written when its tab's title is retyped.
#[test]
fn a_remote_shell_is_renamed_over_http() {
    let served = serve_a_repo("shell-name");
    let backend = RemoteBackend::connect(&served.base_url).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let terminal_id = backend
        .create_terminal(&opened.session_id, None)
        .expect("expected a remote shell to start");
    assert_eq!(
        backend
            .terminal_name(&opened.session_id, &terminal_id)
            .expect("expected the shell's name")
            .as_deref(),
        Some("shell - 1"),
        "a plain shell starts numbered"
    );

    backend
        .rename_terminal(&opened.session_id, &terminal_id, "build")
        .expect("expected the rename");
    assert_eq!(
        backend
            .terminal_name(&opened.session_id, &terminal_id)
            .expect("expected the shell's name")
            .as_deref(),
        Some("build")
    );

    assert!(
        backend
            .rename_terminal(&opened.session_id, &terminal_id, "  ")
            .is_err(),
        "a blank name is refused by the server"
    );
    assert!(
        backend
            .terminal_name(&opened.session_id, "terminal-nobody-0")
            .is_err(),
        "a shell the server does not have is an error rather than a nameless shell"
    );

    backend
        .close_terminal(&opened.session_id, &terminal_id)
        .expect("expected the remote shell to close");
}
