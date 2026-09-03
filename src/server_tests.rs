//! The HTTP contract a remote window reviews through: the routes it calls over the network,
//! which a change made only against the in-process backend would otherwise break unnoticed.

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
    let root = std::env::temp_dir().join(format!("moonreview-server-{}-{name}", std::process::id()));
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
fn the_api_a_remote_window_calls_still_answers() {
    let served = serve("api");
    let session_id = served.open_session();

    let state: serde_json::Value = served
        .client
        .get(format!(
            "{}/api/session/{session_id}/state",
            served.base_url
        ))
        .send()
        .expect("failed to fetch the session state")
        .error_for_status()
        .expect("the server refused the session state")
        .json()
        .expect("failed to decode the session state");

    // A remote window reads these by name, so their shape is part of the contract.
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
        .json(&serde_json::json!({ "title": "Fix the login page", "status": "todo", "joins": "top" }))
        .send()
        .expect("failed to create a task")
        .error_for_status()
        .expect("the server refused to create a task")
        .json()
        .expect("failed to decode the task");
    let task_id = created["id"]
        .as_str()
        .expect("expected a task id")
        .to_string();

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
        .post(format!(
            "{tasks_url}/{task_id}/resources/{resource_id}/stop"
        ))
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
        served
            .root
            .canonicalize()
            .expect("expected a path")
            .display()
            .to_string()
    );

    served
        .client
        .post(format!("{tasks_url}/placement"))
        .json(&serde_json::json!({ "task_ids": [&task_id], "status": "done", "position": 0 }))
        .send()
        .expect("failed to move the task")
        .error_for_status()
        .expect("the server refused to move the task");
    assert_eq!(board(&served)[0]["status"], "done");

    served
        .client
        .delete(format!("{tasks_url}/{task_id}"))
        .send()
        .expect("failed to delete the task")
        .error_for_status()
        .expect("the server refused to delete the task");
    assert!(
        board(&served)
            .as_array()
            .expect("expected an array")
            .is_empty()
    );
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
            .json(&serde_json::json!({ "title": title, "status": "todo", "joins": "top" }))
            .send()
            .expect("failed to create a task")
            .error_for_status()
            .expect("the server refused to create a task")
            .json()
            .expect("failed to decode the task");
        created["id"]
            .as_str()
            .expect("expected a task id")
            .to_string()
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
            .map(|task| {
                task["title"]
                    .as_str()
                    .expect("expected a title")
                    .to_string()
            })
            .collect()
    };
    let place = |task_ids: &[&str], status: &str, position: usize| {
        served
            .client
            .post(format!("{tasks_url}/placement"))
            .json(&serde_json::json!({ "task_ids": task_ids, "status": status, "position": position }))
            .send()
            .expect("failed to move the task")
            .error_for_status()
            .expect("the server refused to move the task");
    };

    let first = create("first");
    let second = create("second");
    let third = create("third");
    // Until one is moved they read in the order they were made, newest at the top.
    assert_eq!(column("todo"), ["third", "second", "first"]);

    place(&[&third], "todo", 1);
    assert_eq!(column("todo"), ["second", "third", "first"]);
    place(&[&third], "todo", 0);
    assert_eq!(column("todo"), ["third", "second", "first"]);
    // Past the end is the end, which is what dropping below the last card means.
    place(&[&third], "todo", 9);
    assert_eq!(column("todo"), ["second", "first", "third"]);

    // The order survives being read back off disk rather than only holding in this process.
    let metadata = std::fs::read_to_string(
        served
            .root
            .join(".moontasks")
            .join(&first)
            .join("metadata.json"),
    )
    .expect("failed to read the task");
    assert!(
        metadata.contains("\"position\": 1"),
        "the card's place should be written down, got: {metadata}"
    );

    place(&[&second], "in_progress", 0);
    assert_eq!(column("todo"), ["first", "third"]);
    assert_eq!(column("in_progress"), ["second"]);

    // A card dropped past the end of another column joins the end of it.
    place(&[&third], "in_progress", 9);
    assert_eq!(column("in_progress"), ["second", "third"]);

    // A drag made with a selection carries every card in it, and they land as a run in the
    // order the board already had them - whichever of them the pointer had hold of, which is
    // what puts them here the other way round.
    place(&[&third, &second], "todo", 0);
    assert_eq!(column("todo"), ["second", "third", "first"]);
    assert_eq!(column("in_progress"), Vec::<String>::new());
}

/// A column can say which end cards moved into it go to, whatever place the drop named. DONE
/// says the top out of the box, so what was finished last is what the column shows first.
#[test]
fn a_column_can_say_which_end_arrivals_go_to() {
    let served = serve("column-arrivals");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let create = |title: &str| -> String {
        let created: serde_json::Value = served
            .client
            .post(&tasks_url)
            .json(&serde_json::json!({ "title": title, "status": "todo", "joins": "top" }))
            .send()
            .expect("failed to create a task")
            .error_for_status()
            .expect("the server refused to create a task")
            .json()
            .expect("failed to decode the task");
        created["id"]
            .as_str()
            .expect("expected a task id")
            .to_string()
    };
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
            .map(|task| {
                task["title"]
                    .as_str()
                    .expect("expected a title")
                    .to_string()
            })
            .collect()
    };
    let place = |task_id: &str, status: &str, position: usize| {
        served
            .client
            .post(format!("{tasks_url}/placement"))
            .json(&serde_json::json!({ "task_ids": [task_id], "status": status, "position": position }))
            .send()
            .expect("failed to move the task")
            .error_for_status()
            .expect("the server refused to move the task");
    };
    let arrivals = |column_id: &str, end: Option<&str>| {
        served
            .client
            .post(format!(
                "{}/api/session/{session_id}/columns/{column_id}/arrivals",
                served.base_url
            ))
            .json(&serde_json::json!({ "arrivals": end }))
            .send()
            .expect("failed to set the column's arrivals")
            .error_for_status()
            .expect("the server refused to set the column's arrivals");
    };

    let first = create("first");
    let second = create("second");

    // Dropped at the bottom of DONE, and drawn at the top of it anyway.
    place(&first, "done", 9);
    place(&second, "done", 9);
    assert_eq!(column("done"), ["second", "first"]);

    // Within the column the drop still decides: this is about arriving, not about ordering.
    place(&second, "done", 1);
    assert_eq!(column("done"), ["first", "second"]);

    // And a column told to let the drop decide behaves like every other column again.
    arrivals("done", None);
    place(&first, "todo", 0);
    place(&first, "done", 9);
    assert_eq!(column("done"), ["second", "first"]);

    // The other way round for a column read as a queue: an arrival joins the back of it.
    arrivals("todo", Some("bottom"));
    let _third = create("third");
    place(&first, "todo", 0);
    assert_eq!(column("todo"), ["third", "first"]);
}

/// The columns are the board's own: they can be added, renamed, reordered and removed, and a
/// board nobody has touched answers with its three defaults.
#[test]
fn the_columns_are_the_boards_to_change() {
    let served = serve("columns");
    let session_id = served.open_session();
    let columns_url = format!("{}/api/session/{session_id}/columns", served.base_url);
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let read = |what: &str| -> Vec<(String, String)> {
        let columns: serde_json::Value = served
            .client
            .get(&columns_url)
            .send()
            .expect("failed to read the columns")
            .json()
            .expect("failed to decode the columns");
        columns
            .as_array()
            .unwrap_or_else(|| panic!("expected an array of columns {what}"))
            .iter()
            .map(|column| {
                (
                    column["id"].as_str().expect("expected an id").to_string(),
                    column["label"]
                        .as_str()
                        .expect("expected a label")
                        .to_string(),
                )
            })
            .collect()
    };
    let ids = |columns: &[(String, String)]| -> Vec<String> {
        columns.iter().map(|(id, _)| id.clone()).collect()
    };

    // A board that has never been changed has no file of its own and the columns it started
    // with - which is what every board written before this existed looks like.
    assert!(
        !served.root.join(".moontasks").join("board.json").exists(),
        "an untouched board should not have written a file yet"
    );
    assert_eq!(ids(&read("at the start")), ["todo", "in_progress", "done"]);

    // Added at the right-hand end, with an id made from its name.
    let added: serde_json::Value = served
        .client
        .post(&columns_url)
        .json(&serde_json::json!({ "label": "Waiting on review" }))
        .send()
        .expect("failed to add a column")
        .error_for_status()
        .expect("the server refused to add a column")
        .json()
        .expect("failed to decode the column");
    assert_eq!(added["id"], "waiting-on-review");
    assert_eq!(
        ids(&read("after adding")).last().map(String::as_str),
        Some("waiting-on-review")
    );

    // Renaming changes what it is called and leaves the id, which is what cards are in.
    served
        .client
        .post(format!("{columns_url}/todo/title"))
        .json(&serde_json::json!({ "label": "BACKLOG" }))
        .send()
        .expect("failed to rename the column")
        .error_for_status()
        .expect("the server refused to rename the column");
    let renamed = read("after renaming");
    assert_eq!(renamed[0], ("todo".to_string(), "BACKLOG".to_string()));

    // A card is made in the column the request names, whatever that column is called now.
    let created: serde_json::Value = served
        .client
        .post(&tasks_url)
        .json(&serde_json::json!({ "title": "Fix the login page", "status": "todo", "joins": "top" }))
        .send()
        .expect("failed to create a task")
        .error_for_status()
        .expect("the server refused to create a task")
        .json()
        .expect("failed to decode the task");
    assert_eq!(created["status"], "todo");

    // A column holding cards will not be removed: it is the only record of where they are.
    let refused = served
        .client
        .delete(format!("{columns_url}/todo"))
        .send()
        .expect("failed to ask to remove the column");
    assert!(
        refused.status().is_client_error() || refused.status().is_server_error(),
        "a column with a card in it should not be removed"
    );
    assert!(ids(&read("after the refusal")).contains(&"todo".to_string()));

    // An empty one goes.
    served
        .client
        .delete(format!("{columns_url}/waiting-on-review"))
        .send()
        .expect("failed to remove the column")
        .error_for_status()
        .expect("the server refused to remove an empty column");
    assert!(!ids(&read("after removing")).contains(&"waiting-on-review".to_string()));

    // Dragging a heading moves the column and takes its cards with it, because a card names
    // its column rather than its place on the board.
    served
        .client
        .post(format!("{columns_url}/todo/placement"))
        .json(&serde_json::json!({ "position": 2 }))
        .send()
        .expect("failed to move the column")
        .error_for_status()
        .expect("the server refused to move the column");
    assert_eq!(ids(&read("after moving")), ["in_progress", "done", "todo"]);

    let board: serde_json::Value = served
        .client
        .get(&tasks_url)
        .send()
        .expect("failed to read the board")
        .json()
        .expect("failed to decode the board");
    assert_eq!(
        board[0]["status"], "todo",
        "the card should still be in the column that moved"
    );
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

/// A file of the repo goes on a card by the path the file pane opens it with, and comes off
/// again the way a run does. Only a file that is there right now is taken: the card is a way
/// back to the file, and a link to nothing is worse than none.
#[test]
fn a_file_can_be_linked_to_a_task_over_http() {
    let served = serve("task-files");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);
    fs::create_dir_all(served.root.join("src")).expect("failed to make the fixture folder");
    fs::write(served.root.join("src/lib.rs"), "pub fn one() {}\n")
        .expect("failed to write the fixture file");

    let created: serde_json::Value = served
        .client
        .post(&tasks_url)
        .json(&serde_json::json!({ "title": "Fix the login page", "status": "todo", "joins": "top" }))
        .send()
        .expect("failed to create a task")
        .error_for_status()
        .expect("the server refused to create a task")
        .json()
        .expect("failed to decode the task");
    let task_id = created["id"]
        .as_str()
        .expect("expected a task id")
        .to_string();
    let files_url = format!("{tasks_url}/{task_id}/files");
    let link = |file_path: &str| -> reqwest::blocking::Response {
        served
            .client
            .post(&files_url)
            .json(&serde_json::json!({ "file_path": file_path }))
            .send()
            .expect("failed to link a file")
    };

    link("src/lib.rs")
        .error_for_status()
        .expect("the server refused to link a file of the repo");

    // A path that names nothing, one outside the repo, and the same file twice all stay off
    // the card.
    for (file_path, why) in [
        ("src/gone.rs", "a file that is not there"),
        ("../outside.rs", "a path outside the repo"),
        ("/etc/hosts", "an absolute path"),
        ("src", "a directory"),
        ("src/lib.rs", "a file already on the card"),
    ] {
        assert!(
            link(file_path).status().is_client_error(),
            "{why} should have been refused"
        );
    }

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
    let resources = tasks[0]["resources"]
        .as_array()
        .expect("expected an array");
    assert_eq!(resources.len(), 1, "one link, whatever was refused: {resources:?}");
    let resource = &resources[0];
    assert_eq!(resource["kind"], "file");
    assert_eq!(resource["file_path"], "src/lib.rs");
    assert_eq!(resource["label"], "src/lib.rs");
    assert_eq!(resource["running"], false);
    assert_eq!(resource["resumable"], false);
    // Written down, so it is on the card after a restart the way an agent run is.
    let metadata = fs::read_to_string(
        served
            .root
            .join(".moontasks")
            .join(&task_id)
            .join("metadata.json"),
    )
    .expect("failed to read the task's metadata");
    assert!(
        metadata.contains("\"file_path\": \"src/lib.rs\""),
        "the link should be in the task folder: {metadata}"
    );

    // Off the card again by the same delete a run leaves by, and the file itself untouched.
    let resource_id = resource["id"].as_str().expect("expected a resource id");
    served
        .client
        .delete(format!("{tasks_url}/{task_id}/resources/{resource_id}"))
        .send()
        .expect("failed to unlink the file")
        .error_for_status()
        .expect("the server refused to unlink the file");
    assert!(
        board(&served)[0]["resources"]
            .as_array()
            .expect("expected an array")
            .is_empty(),
        "an unlinked file should be off the task"
    );
    assert!(
        served.root.join("src/lib.rs").is_file(),
        "unlinking is not deleting"
    );
}
