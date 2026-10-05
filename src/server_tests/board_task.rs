//! The board task over HTTP: the task on no column that the board's own agents and shells
//! are runs of, started in and stopped by the routes a card's are.

use super::serve;

#[test]
fn the_board_task_holds_runs_and_is_no_card() {
    let served = serve("board-task");
    let session_id = served.open_session();
    let session_url = format!("{}/api/session/{session_id}", served.base_url);
    let read = |path: &str| -> serde_json::Value {
        served
            .client
            .get(format!("{session_url}{path}"))
            .send()
            .expect("failed to read")
            .error_for_status()
            .expect("the server refused the read")
            .json()
            .expect("failed to decode")
    };

    // Reading the board task is what makes it, on a board that had none.
    let board_task = read("/board-task");
    assert_eq!(board_task["id"], "board-task");
    assert_eq!(board_task["title"], "board");
    assert_eq!(board_task["resources"], serde_json::json!([]));
    assert!(
        served
            .root
            .join(".moontasks/board-task/metadata.json")
            .is_file(),
        "the board task's folder should be in the repo"
    );
    assert_eq!(read("/tasks"), serde_json::json!([]), "it is no card");

    // A shell is started in it the way one is started in a task.
    let opened: serde_json::Value = served
        .client
        .post(format!("{session_url}/tasks/board-task/resources"))
        .json(&serde_json::json!({ "kind": "shell", "agent": "none", "opens_in": "repo" }))
        .send()
        .expect("failed to start a shell")
        .error_for_status()
        .expect("the server refused to start a shell")
        .json()
        .expect("failed to decode the shell");
    let terminal_id = opened["terminal_id"].as_str().expect("expected a shell");

    let board_task = read("/board-task");
    let shell = &board_task["resources"][0];
    assert_eq!(shell["kind"], "shell");
    assert_eq!(shell["running"], true);
    assert_eq!(shell["terminal_id"], serde_json::json!(terminal_id));
    // Named after the board task, as a task's shell is named after the task.
    assert_eq!(shell["label"], "board shell - 1");
    assert!(
        !read("/terminals")["terminal_ids"]
            .as_array()
            .expect("expected an array")
            .contains(&serde_json::json!(terminal_id)),
        "the board task's shell is the board task's to show, not a workspace shell"
    );
    assert_eq!(
        read("/tasks"),
        serde_json::json!([]),
        "and it is still no card"
    );

    // A board task is on no column, and cannot be dropped into one.
    let placed = served
        .client
        .post(format!("{session_url}/tasks/placement"))
        .json(&serde_json::json!({ "task_ids": ["board-task"], "status": "todo", "position": 0 }))
        .send()
        .expect("failed to ask for the placement");
    assert!(
        placed.status().is_client_error() || placed.status().is_server_error(),
        "placing the board task in a column should have been refused, got {}",
        placed.status()
    );

    // And the shell is closed the way a task's is.
    served
        .client
        .delete(format!(
            "{session_url}/tasks/board-task/resources/{terminal_id}"
        ))
        .send()
        .expect("failed to close the shell")
        .error_for_status()
        .expect("the server refused to close the shell");
    assert_eq!(read("/board-task")["resources"], serde_json::json!([]));
}
