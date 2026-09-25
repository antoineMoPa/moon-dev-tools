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

use egui_moon_code_ide::LanguageSource;

use crate::{
    api::{LspPosition, LspStatus, LspTriggersPayload, OpenSessionRequest},
    backend::{Backend, remote::RemoteBackend},
    git::run_git_no_output,
    moontasks::{ColumnEnd, ColumnId, CreateTaskRequest, review_request::Amend},
    native::{language_source::SessionLanguages, workspace_color::WorkspaceColor},
    pass_keys::of_this_test_run,
    settings::SettingsChange,
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

/// A key the test servers let in - see [`of_this_test_run`].
fn pass_key() -> String {
    of_this_test_run().generate()
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
            let _ = crate::server::serve_on(
                state,
                crate::server::users::of_this_test_run(),
                listener,
                None,
            )
            .await;
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
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");

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

/// A window on another machine is working on the server's projects, so it gets the server's
/// settings - the color a project was marked from one machine is the color it is on another.
#[test]
fn a_remote_window_reads_and_changes_the_server_s_settings() {
    let served = serve_a_repo("settings");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let project_path = served.root.display().to_string();

    backend
        .change_settings(SettingsChange::MarkWorkspace {
            project_path: project_path.clone(),
            color: WorkspaceColor::Ember,
        })
        .expect("expected the mark to be written");
    backend
        .change_settings(SettingsChange::RememberProject(project_path.clone()))
        .expect("expected the project to be remembered");

    let settings = backend.settings().expect("expected the server's settings");
    assert_eq!(settings.workspace_color(&project_path), WorkspaceColor::Ember);
    assert_eq!(settings.recent_projects.first(), Some(&project_path));
    // The one change after the other: the second did not write back a file without the first.
    assert_eq!(
        crate::settings::load().workspace_color(&project_path),
        WorkspaceColor::Ember,
        "the mark should be in the server's own file"
    );

    if let Some(path) = crate::settings::path() {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn staging_through_a_remote_review_changes_the_repo() {
    let served = serve_a_repo("stage");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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

/// The language routes as a remote window uses them. A markdown file has no server behind
/// it on any machine, so this says the same thing everywhere and says it in milliseconds -
/// what is being checked is that the route is wired up and the answer survives the wire.
#[test]
fn a_remote_review_answers_that_a_markdown_file_has_no_language_server() {
    let served = serve_a_repo("lsp");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    assert_eq!(
        backend
            .lsp_status(&opened.session_id, "notes.md")
            .expect("expected a language status over HTTP"),
        LspStatus::Unavailable
    );
    // Opening and closing one is quietly nothing to do rather than an error.
    backend
        .lsp_did_open(&opened.session_id, "notes.md", "# notes\n")
        .expect("expected opening an unserved file to be accepted");
    backend
        .lsp_did_change(&opened.session_id, "notes.md", "# notes, changed\n")
        .expect("expected changing an unserved file to be accepted");
    backend
        .lsp_did_close(&opened.session_id, "notes.md")
        .expect("expected closing an unserved file to be accepted");
    assert!(
        backend
            .lsp_places(
                &opened.session_id,
                "notes.md",
                LspPosition { line: 0, column: 2 },
                crate::api::LspPlaces::Definition
            )
            .expect("expected a definition answer")
            .is_empty(),
        "a file with no server behind it has no definitions"
    );
}

/// Rename's two routes, over the wire. Markdown for the same reason as above: nothing serves it
/// on any machine, so nothing in it can be renamed and a rename in it changes nothing - the
/// same answer everywhere, in milliseconds, and what is checked is that both routes are wired
/// up and their answers survive the trip.
#[test]
fn a_remote_review_is_answered_that_nothing_in_a_markdown_file_can_be_renamed() {
    let served = serve_a_repo("lsp-rename");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");
    let at = LspPosition { line: 0, column: 2 };

    assert_eq!(
        backend
            .lsp_prepare_rename(&opened.session_id, "notes.md", at)
            .expect("expected a prepareRename answer over HTTP"),
        None,
        "nothing in a file no server serves can be renamed"
    );
    assert!(
        backend
            .lsp_rename(&opened.session_id, "notes.md", at, "renamed")
            .expect("expected a rename answer over HTTP")
            .is_empty(),
        "a rename in a file no server serves changes nothing"
    );
}

/// Hover and diagnostics, over the wire: nothing to say about a file nothing serves, carried
/// across the same way whatever there is to say.
#[test]
fn a_remote_review_hears_nothing_about_a_markdown_file_on_hover_or_diagnostics() {
    let served = serve_a_repo("lsp-hover");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    assert_eq!(
        backend
            .lsp_hover(
                &opened.session_id,
                "notes.md",
                LspPosition { line: 0, column: 2 }
            )
            .expect("expected a hover answer over HTTP"),
        None
    );
    assert!(
        backend
            .lsp_diagnostics(&opened.session_id, "notes.md")
            .expect("expected diagnostics over HTTP")
            .is_empty()
    );
    backend
        .lsp_did_save(&opened.session_id, "notes.md")
        .expect("expected saving an unserved file to be accepted");
}

/// Code actions and signature help, over the wire: nothing on offer and no call around, in a
/// file nothing serves.
#[test]
fn a_remote_review_is_offered_nothing_to_do_and_no_signature_in_a_markdown_file() {
    let served = serve_a_repo("lsp-actions");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");
    let at = LspPosition { line: 0, column: 2 };

    assert!(
        backend
            .lsp_code_actions(&opened.session_id, "notes.md", at)
            .expect("expected code actions over HTTP")
            .is_empty()
    );
    assert_eq!(
        backend
            .lsp_signature_help(&opened.session_id, "notes.md", at)
            .expect("expected a signature answer over HTTP"),
        None
    );
}

/// The seventh question, over the wire and through the same [`SessionLanguages`] the panes
/// use: what the server behind a file says opens a completion list on its own.
///
/// This is the one that was missing. Everything above it can be right - the server declares
/// its `.` and `:`, the crate asks with no prefix when one is typed - and typing a `.` in a
/// remote review still does nothing, because the window never carried the question across.
/// So it is asked here the way the window asks it: through the trait, on a backend that is
/// really HTTP.
///
/// Markdown for the same reason the definition test uses it - no machine has a server behind
/// it, so the answer is the same everywhere and arrives in milliseconds. That the wire itself
/// carries a real list is checked beside it, on the payload the route answers with: a `char`
/// goes out as a one-character string and has to come back the character it was, which is the
/// half of this that a file with no server cannot prove.
#[test]
fn a_remote_review_is_asked_what_opens_a_completion_list_through_the_pane_s_own_source() {
    let served = serve_a_repo("lsp-triggers");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    assert!(
        backend
            .lsp_trigger_characters(&opened.session_id, "notes.md")
            .expect("expected an answer about what opens a list")
            .is_empty(),
        "nothing serves a markdown file, so nothing opens a list in one"
    );

    // And as the pane really asks it: through the source the crate is handed, which is what
    // makes the crate's default answer of "none" stop applying to this window.
    let source = SessionLanguages::new(&backend, &opened.session_id);
    assert!(
        LanguageSource::trigger_characters(&source, "notes.md").is_empty(),
        "the window\'s source has to answer the question, not fall back on the default"
    );

    // What a served file would send back, over the format the route answers in.
    let sent = serde_json::to_string(&LspTriggersPayload {
        triggers: vec!['.', ':', '\'', '('],
    })
    .expect("expected the triggers to serialise");
    let read: LspTriggersPayload =
        serde_json::from_str(&sent).expect("expected the triggers to be read back");
    assert_eq!(read.triggers, ['.', ':', '\'', '(']);
}

/// The status bar's question, over the wire. A session whose servers have not been started -
/// nothing has opened a file yet - is doing nothing, which is an empty answer rather than an
/// error: that is the ordinary state of a review, and the bar reads it as nothing to wait for.
#[test]
fn a_remote_review_says_its_language_servers_are_doing_nothing_when_none_are_running() {
    let served = serve_a_repo("lsp-working");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    assert!(
        backend
            .lsp_working(&opened.session_id)
            .expect("expected an answer about what the servers are doing")
            .is_empty()
    );
}

#[test]
fn a_remote_shell_carries_bytes_both_ways_over_the_websocket() {
    let served = serve_a_repo("shell");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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

/// A shell's socket is admitted as it opens, so a kick has to reach it too: the server hangs
/// up on the kicked user's shells - see `crate::server::users`.
#[test]
fn a_kicked_user_s_remote_shell_is_hung_up_on() {
    let served = serve_a_repo("shell-kicked");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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
    let attachment = backend
        .attach_terminal(&opened.session_id, &terminal_id)
        .expect("expected to attach to the remote shell");

    // Another user of the server kicks the shell's.
    let kicker = reqwest::blocking::Client::new();
    let listed: crate::server::users::UserList = kicker
        .get(format!("{}/api/users", served.base_url))
        .bearer_auth(pass_key())
        .send()
        .expect("failed to ask for the users")
        .json()
        .expect("failed to decode the users");
    let shell_s_user = listed
        .users
        .iter()
        .find(|user| !user.you)
        .expect("the shell's user was seen");
    let kicked = kicker
        .post(format!(
            "{}/api/users/{}/kick",
            served.base_url, shell_s_user.id
        ))
        .bearer_auth(pass_key())
        .send()
        .expect("failed to ask for the kick");
    assert_eq!(kicked.status(), reqwest::StatusCode::NO_CONTENT);

    // The socket closes: its output ends rather than staying open for more.
    let deadline = Instant::now() + Duration::from_secs(20);
    let hung_up = loop {
        match attachment.output.recv_timeout(Duration::from_millis(200)) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break false,
        }
    };
    assert!(hung_up, "the kicked user's shell stayed attached");
}

/// Opening the work log over HTTP makes the file, in the board's folder, holding the line an
/// entry goes above - and says where it is the way the file pane addresses files.
#[test]
fn the_work_log_opens_over_http() {
    let served = serve_a_repo("work-log");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let path = backend
        .open_work_log(&opened.session_id)
        .expect("expected the work log to open");
    assert_eq!(path, ".moontasks/work-log.org");

    let content = backend
        .file_content(&opened.session_id, &path)
        .expect("expected the file pane's read to reach the work log");
    assert_eq!(content.content, "#now#\n");
    assert!(served.root.join(".moontasks/.gitignore").is_file());
}

/// A task's `request_for_review.txt` is on the server, beside the board, so a remote window is
/// told its rows over HTTP and crosses them off there - not in a folder of its own machine.
#[test]
fn review_requests_are_read_and_crossed_off_over_http() {
    let served = serve_a_repo("review-requests");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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
                title: "Deploy the thing".to_string(),
                status: ColumnId::new("todo"),
                joins: ColumnEnd::Top,
            },
        )
        .expect("expected the remote task to be created");
    // Written the way an agent writes it: straight into the task's folder on the server.
    fs::write(
        served
            .root
            .join(format!(".moontasks/{}/request_for_review.txt", task.id)),
        ". // fix: the thing\n",
    )
    .expect("expected the request to be written");

    let requests = backend
        .list_review_requests(&opened.session_id)
        .expect("expected the remote review requests");
    assert_eq!(requests.len(), 1, "expected the one line");
    assert_eq!(requests[0].task_id, task.id);
    assert!(!requests[0].done);

    backend
        .amend_review_request(&opened.session_id, &task.id, 0, Amend::Done(true))
        .expect("expected the line to be crossed off");

    let requests = backend
        .list_review_requests(&opened.session_id)
        .expect("expected the remote review requests");
    assert!(requests[0].done, "the line should read as crossed off");
}

/// The card's notes and the file pane read the same file: opening the notes makes it real,
/// the file pane's own write path edits it, and the board's next read shows what was written.
#[test]
fn task_notes_round_trip_over_http() {
    let served = serve_a_repo("notes");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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

/// The read this route serves is a read of somebody else's disk, so a path no language server
/// ever named is refused over the wire the way it is refused in process: a credential file by
/// its absolute path, and a walk out of the repo with `..`. This is the one that matters -
/// every other test of the allow-list is in the same process as the state it is asserting on,
/// and this one is the shape a real attempt would take.
#[test]
fn a_file_outside_the_repo_is_refused_over_http() {
    let served = serve_a_repo("outside-the-repo");
    let secret = served.root.with_file_name(format!(
        "{}-secret.txt",
        served
            .root
            .file_name()
            .expect("the fixture repo has a name")
            .to_string_lossy()
    ));
    fs::write(&secret, "a private key\n").expect("failed to write the fixture secret");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    for path in [
        "/etc/passwd".to_string(),
        "../../../etc/passwd".to_string(),
        secret.display().to_string(),
        format!(
            "../{}",
            secret
                .file_name()
                .expect("the fixture secret has a name")
                .to_string_lossy()
        ),
    ] {
        assert!(
            backend.file_content(&opened.session_id, &path).is_err(),
            "{path} is not in the repo and no language server named it"
        );
    }

    // The repo's own files still read, and read as files of the repo.
    let inside = backend
        .file_content(&opened.session_id, "main.rs")
        .expect("expected the repo's own file to read");
    assert!(inside.content.contains("fn main()"));
    assert!(!inside.outside_the_repo);

    let _ = fs::remove_file(&secret);
}

#[test]
fn an_unreachable_address_fails_with_the_address_in_the_message() {
    // Port 1 is reserved and nothing listens there, so this is a connection refusal.
    let error = match RemoteBackend::connect("127.0.0.1:1", pass_key()) {
        Ok(_) => panic!("expected the connect to fail"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("127.0.0.1:1"),
        "the error should name what it could not reach: {error}"
    );
}

/// A key the server did not make is refused while connecting, with what to do about it,
/// rather than on whatever the window asks for first.
#[test]
fn a_wrong_pass_key_fails_at_connect_saying_how_to_get_one() {
    let served = serve_a_repo("wrong-key");
    let other_secret = std::env::temp_dir().join(format!(
        "moonreview-remote-other-secret-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&other_secret);
    let wrong = crate::pass_keys::PassKeys::kept_at(&other_secret)
        .expect("expected a second secret")
        .generate();

    let error = match RemoteBackend::connect(&served.base_url, wrong) {
        Ok(_) => panic!("expected a key of another secret to be refused"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("did not accept the pass key"),
        "got {message}"
    );
    assert!(message.contains("generate-pass-key"), "got {message}");
    let _ = fs::remove_file(&other_secret);
}

/// A window hands a browser, or a person, a key of its own making - Tools › Open in Web and
/// Generate Pass Key - and a key the far side minted lets a new window in.
#[test]
fn a_minted_pass_key_lets_another_window_in() {
    let served = serve_a_repo("minted");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");

    let minted = backend.mint_pass_key().expect("expected a new key");

    assert!(of_this_test_run().admits(&minted));
    RemoteBackend::connect(&served.base_url, minted)
        .expect("the minted key should let a window in");
}

/// Tools › Open in Web on a remote window opens a browser with a ticket the far side made,
/// which that server's login redeems.
#[test]
fn a_minted_login_ticket_is_the_far_side_s() {
    let served = serve_a_repo("minted-ticket");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");

    let ticket = backend
        .mint_login_ticket(crate::pass_keys::OPEN_IN_WEB_TICKET_LIFETIME)
        .expect("expected a ticket");

    assert!(
        crate::pass_keys::RedeemedTickets::default()
            .redeem(&of_this_test_run(), &ticket)
            .is_ok()
    );
    assert!(!of_this_test_run().admits(&ticket));
}

/// A file linked over the wire is on the card the next time the board is read, by the same
/// path the file pane then opens it with.
#[test]
fn a_linked_file_round_trips_over_http() {
    let served = serve_a_repo("task-files");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
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

/// A column added from the middle of the board goes where it was asked for, not on the end.
/// Over HTTP because the place is carried in the request body rather than the path.
#[test]
fn a_column_is_added_where_it_was_asked_for() {
    let served = serve_a_repo("columns");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let added = backend
        .add_column(&opened.session_id, "Blocked", Some(1))
        .expect("expected the column to be added");

    let labels: Vec<String> = backend
        .list_columns(&opened.session_id)
        .expect("expected the columns to be listed")
        .into_iter()
        .map(|column| column.label)
        .collect();
    assert_eq!(labels, ["TODO", "Blocked", "IN PROGRESS", "DONE"]);
    assert_eq!(added.label, "Blocked");

    // Nothing said where this one goes, so it joins the right-hand end.
    backend
        .add_column(&opened.session_id, "Shipped", None)
        .expect("expected the column to be added");
    let last = backend
        .list_columns(&opened.session_id)
        .expect("expected the columns to be listed")
        .pop()
        .expect("expected a last column");
    assert_eq!(last.label, "Shipped");
}
