//! The web frontend's contract: the page, its assets, and the API it calls.
//!
//! The native window is the default frontend now, so these exist to make sure the browser
//! one keeps working — a change that only breaks the web client would otherwise go unnoticed.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use reqwest::blocking::Client;

use crate::{api::OpenSessionRequest, git::run_git_no_output};

struct Served {
    root: PathBuf,
    base_url: String,
    client: Client,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn serve(name: &str) -> Served {
    let root = std::env::temp_dir().join(format!("moonreview-web-{}-{name}", std::process::id()));
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
    fs::write(root.join("a.txt"), "one\n").expect("failed to write the fixture file");
    run_git_no_output(&root, &["add", "-A"]).expect("failed to stage the fixture");
    run_git_no_output(&root, &["commit", "-m", "first"]).expect("failed to commit the fixture");
    fs::write(root.join("a.txt"), "two\n").expect("failed to change the fixture file");

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

    Served {
        root,
        base_url: format!("http://127.0.0.1:{port}"),
        client: Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to build the test client"),
    }
}

impl Served {
    fn open_session(&self) -> String {
        #[derive(serde::Deserialize)]
        struct Opened {
            session_id: String,
        }

        let opened: Opened = self
            .client
            .post(format!("{}/api/session/open", self.base_url))
            .json(&OpenSessionRequest {
                repo_path: self.root.display().to_string(),
                diff_target: None,
                active_commit: None,
            })
            .send()
            .expect("failed to open a session")
            .error_for_status()
            .expect("the server refused to open a session")
            .json()
            .expect("failed to decode the session");
        opened.session_id
    }
}

#[test]
fn the_review_page_and_its_assets_are_served() {
    let served = serve("assets");
    let session_id = served.open_session();

    let page = served
        .client
        .get(format!("{}/review/{session_id}", served.base_url))
        .send()
        .expect("failed to fetch the review page");
    assert!(page.status().is_success());
    let html = page.text().expect("failed to read the review page");
    assert!(html.contains("/assets/app.js"), "the page must load the bundle");
    assert!(html.contains("/assets/app.css"), "the page must load the styles");

    for (path, content_type, needle) in [
        ("/assets/app.js", "application/javascript", "moonreview"),
        ("/assets/app.css", "text/css", "--accent"),
    ] {
        let asset = served
            .client
            .get(format!("{}{path}", served.base_url))
            .send()
            .expect("failed to fetch an asset");
        assert!(asset.status().is_success(), "{path} should be served");
        assert!(
            asset
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with(content_type)),
            "{path} should be served as {content_type}"
        );
        let body = asset.text().expect("failed to read an asset");
        assert!(!body.is_empty(), "{path} should not be empty");
        assert!(body.contains(needle), "{path} does not look like the bundle");
    }
}

#[test]
fn the_api_the_web_frontend_calls_still_answers() {
    let served = serve("api");
    let session_id = served.open_session();

    let state: serde_json::Value = served
        .client
        .get(format!("{}/api/session/{session_id}/state", served.base_url))
        .send()
        .expect("failed to fetch the session state")
        .error_for_status()
        .expect("the server refused the session state")
        .json()
        .expect("failed to decode the session state");

    // The web client reads these by name, so their shape is part of the contract.
    for field in [
        "repo_name",
        "hunks",
        "review_comments",
        "export_text",
        "available_agents",
        "read_only",
        "patch_preview_line_limit",
        "local_change_summary",
    ] {
        assert!(state.get(field).is_some(), "the payload lost `{field}`");
    }
    let hunks = state["hunks"].as_array().expect("hunks should be an array");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0]["file_path"], "a.txt");
    assert!(hunks[0]["patch_preview"].as_str().unwrap().contains("+two"));

    let health = served
        .client
        .get(format!("{}/healthz", served.base_url))
        .send()
        .expect("failed to fetch health");
    assert_eq!(health.text().expect("failed to read health"), "ok");
}

/// The board over HTTP, which is the path a window pointed at another machine takes.
#[test]
fn a_task_can_be_created_worked_in_and_moved_over_http() {
    let served = serve("tasks");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let created: serde_json::Value = served
        .client
        .post(&tasks_url)
        .json(&serde_json::json!({ "title": "Fix the login page", "agent": "none" }))
        .send()
        .expect("failed to create a task")
        .error_for_status()
        .expect("the server refused to create a task")
        .json()
        .expect("failed to decode the task");
    let task_id = created["id"].as_str().expect("expected a task id").to_string();

    assert!(task_id.starts_with("fix-the-login-page-"));
    assert_eq!(created["status"], "todo");
    // The folder is the task, and it is the repo's to keep.
    assert!(
        served.root.join(".moontasks").join(&task_id).is_dir(),
        "the task folder should be in the repo"
    );

    // A shell opened in the task belongs to the task, not to the workspace's own shells.
    let opened: serde_json::Value = served
        .client
        .post(format!("{tasks_url}/{task_id}/resources"))
        .json(&serde_json::json!({ "kind": "shell", "agent": "none" }))
        .send()
        .expect("failed to start a shell")
        .error_for_status()
        .expect("the server refused to start a shell")
        .json()
        .expect("failed to decode the shell");
    let terminal_id = opened["terminal_id"]
        .as_str()
        .expect("expected a terminal id")
        .to_string();

    let workspace_shells: serde_json::Value = served
        .client
        .get(format!(
            "{}/api/session/{session_id}/terminals",
            served.base_url
        ))
        .send()
        .expect("failed to list shells")
        .json()
        .expect("failed to decode the shells");
    assert!(
        !workspace_shells["terminal_ids"]
            .as_array()
            .expect("expected an array")
            .contains(&serde_json::json!(terminal_id)),
        "a task's shell is the task's to show"
    );

    let board = |served: &Served| -> serde_json::Value {
        served
            .client
            .get(&tasks_url)
            .send()
            .expect("failed to read the board")
            .json()
            .expect("failed to decode the board")
    };

    let tasks = board(&served);
    assert_eq!(tasks.as_array().expect("expected an array").len(), 1);
    let resource = &tasks[0]["resources"][0];
    assert_eq!(resource["kind"], "shell");
    assert_eq!(resource["running"], true);
    // A shell goes by its terminal, because the running shells are the only record of it.
    assert_eq!(resource["id"], serde_json::json!(terminal_id));
    let resource_id = terminal_id.clone();

    // Closing a shell is the end of it: there is no pty left to come back to, so it leaves the
    // card rather than sitting there as a run that can never be reopened.
    served
        .client
        .post(format!("{tasks_url}/{task_id}/resources/{resource_id}/stop"))
        .json(&serde_json::json!({}))
        .send()
        .expect("failed to close the shell")
        .error_for_status()
        .expect("the server refused to close the shell");
    assert!(
        board(&served)[0]["resources"]
            .as_array()
            .expect("expected an array")
            .is_empty(),
        "a closed shell should be off the task"
    );
    // And it is not written down, so nothing brings it back on the next read of the board.
    let metadata = std::fs::read_to_string(
        served
            .root
            .join(".moontasks")
            .join(&task_id)
            .join("metadata.json"),
    )
    .expect("failed to read the task");
    assert!(
        !metadata.contains("shell"),
        "a shell should not be recorded on the task, got: {metadata}"
    );

    served
        .client
        .post(format!("{tasks_url}/{task_id}/title"))
        .json(&serde_json::json!({ "title": "Fix the login page properly" }))
        .send()
        .expect("failed to rename the task")
        .error_for_status()
        .expect("the server refused to rename the task");
    let renamed = board(&served);
    assert_eq!(renamed[0]["title"], "Fix the login page properly");
    assert_eq!(
        renamed[0]["id"], task_id,
        "a rename keeps the folder, which is what everything else points at"
    );
    // The review a card opens is the repo's, not anything inside the task folder.
    assert_eq!(
        renamed[0]["repo_path"],
        served.root.canonicalize().expect("expected a path").display().to_string()
    );

    served
        .client
        .post(format!("{tasks_url}/{task_id}/status"))
        .json(&serde_json::json!({ "status": "in_local_review" }))
        .send()
        .expect("failed to move the task")
        .error_for_status()
        .expect("the server refused to move the task");
    assert_eq!(board(&served)[0]["status"], "in_local_review");

    served
        .client
        .delete(format!("{tasks_url}/{task_id}"))
        .send()
        .expect("failed to delete the task")
        .error_for_status()
        .expect("the server refused to delete the task");
    assert!(board(&served).as_array().expect("expected an array").is_empty());
}

/// Cards keep the order they were put in, which is the order the board reads them back in.
#[test]
fn cards_are_dropped_where_they_are_let_go_of() {
    let served = serve("task-order");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let create = |title: &str| -> String {
        let created: serde_json::Value = served
            .client
            .post(&tasks_url)
            .json(&serde_json::json!({ "title": title, "agent": "none" }))
            .send()
            .expect("failed to create a task")
            .error_for_status()
            .expect("the server refused to create a task")
            .json()
            .expect("failed to decode the task");
        created["id"].as_str().expect("expected a task id").to_string()
    };
    // A column, read out of the board in the order the board hands it over.
    let column = |status: &str| -> Vec<String> {
        let board: serde_json::Value = served
            .client
            .get(&tasks_url)
            .send()
            .expect("failed to read the board")
            .json()
            .expect("failed to decode the board");
        board
            .as_array()
            .expect("expected an array")
            .iter()
            .filter(|task| task["status"] == status)
            .map(|task| task["title"].as_str().expect("expected a title").to_string())
            .collect()
    };
    let place = |task_id: &str, status: &str, position: usize| {
        served
            .client
            .post(format!("{tasks_url}/{task_id}/placement"))
            .json(&serde_json::json!({ "status": status, "position": position }))
            .send()
            .expect("failed to move the task")
            .error_for_status()
            .expect("the server refused to move the task");
    };

    create("first");
    let second = create("second");
    let third = create("third");
    // Until one is moved they read in the order they were made, newest at the bottom.
    assert_eq!(column("todo"), ["first", "second", "third"]);

    place(&third, "todo", 0);
    assert_eq!(column("todo"), ["third", "first", "second"]);
    place(&third, "todo", 1);
    assert_eq!(column("todo"), ["first", "third", "second"]);
    // Past the end is the end, which is what dropping below the last card means.
    place(&third, "todo", 9);
    assert_eq!(column("todo"), ["first", "second", "third"]);

    // The order survives being read back off disk rather than only holding in this process.
    let metadata = std::fs::read_to_string(
        served
            .root
            .join(".moontasks")
            .join(&second)
            .join("metadata.json"),
    )
    .expect("failed to read the task");
    assert!(
        metadata.contains("\"position\": 1"),
        "the card's place should be written down, got: {metadata}"
    );

    place(&second, "in_progress", 0);
    assert_eq!(column("todo"), ["first", "third"]);
    assert_eq!(column("in_progress"), ["second"]);

    // An agent moving its own card has no place in mind, so it joins the end of the column.
    served
        .client
        .post(format!("{tasks_url}/{third}/status"))
        .json(&serde_json::json!({ "status": "in_progress" }))
        .send()
        .expect("failed to move the task")
        .error_for_status()
        .expect("the server refused to move the task");
    assert_eq!(column("in_progress"), ["second", "third"]);
}

#[test]
fn an_unknown_session_is_refused_rather_than_panicking() {
    let served = serve("unknown");

    let response = served
        .client
        .get(format!("{}/api/session/nope/state", served.base_url))
        .send()
        .expect("failed to fetch an unknown session");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .expect("failed to read the refusal")
            .contains("unknown session")
    );
}
