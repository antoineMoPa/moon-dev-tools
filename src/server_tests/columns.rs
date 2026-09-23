//! The board's columns over HTTP: where dropped cards land, the end a column's arrivals go to,
//! a sorted column's own order, and the columns themselves being changed.

use super::serve;

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

/// A card carries when it arrived in its column: moving it to another column stamps it anew,
/// and shuffling it about inside the column leaves the stamp alone. DONE's date lines read it.
#[test]
fn a_card_is_stamped_with_when_it_arrived_in_its_column() {
    let served = serve("column-arrival-stamp");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let created: serde_json::Value = served
        .client
        .post(&tasks_url)
        .json(&serde_json::json!({ "title": "first", "status": "todo", "joins": "top" }))
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
    assert_eq!(
        created["entered_column_at_unix"], created["created_at_unix"],
        "a card just made arrived in its column as it was made"
    );

    // Written as though it arrived long ago, so a fresh stamp can be told from the old one
    // inside the same second.
    let metadata_path = served
        .root
        .join(".moontasks")
        .join(&task_id)
        .join("metadata.json");
    let mut metadata: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&metadata_path).expect("failed to read the task"),
    )
    .expect("expected the task's metadata");
    metadata["entered_column_at_unix"] = serde_json::json!(1_700_000_000);
    std::fs::write(&metadata_path, metadata.to_string()).expect("failed to write the task");

    let place = |status: &str, position: usize| -> serde_json::Value {
        served
            .client
            .post(format!("{tasks_url}/placement"))
            .json(&serde_json::json!({ "task_ids": [&task_id], "status": status, "position": position }))
            .send()
            .expect("failed to move the task")
            .error_for_status()
            .expect("the server refused to move the task");
        let board: serde_json::Value = served
            .client
            .get(&tasks_url)
            .send()
            .expect("failed to read the board")
            .json()
            .expect("failed to decode the board");
        board[0]["entered_column_at_unix"].clone()
    };

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("expected the time")
        .as_secs();
    let arrived = place("done", 0);
    let arrived = arrived
        .as_u64()
        .expect("expected the arrival to be written down");
    assert!(
        arrived >= before,
        "moving to DONE stamps the card anew, got {arrived}"
    );

    assert_eq!(
        place("done", 0),
        serde_json::json!(arrived),
        "a move inside the column is not an arrival"
    );
}

/// A sorted column lists its cards in its own order whatever order they were put in, and
/// remembers the order they were put in for when it is sorted no longer.
#[test]
fn a_sorted_column_keeps_its_own_order() {
    let served = serve("column-sort");
    let session_id = served.open_session();
    let tasks_url = format!("{}/api/session/{session_id}/tasks", served.base_url);

    let create = |title: &str| {
        served
            .client
            .post(&tasks_url)
            .json(&serde_json::json!({ "title": title, "status": "todo", "joins": "bottom" }))
            .send()
            .expect("failed to create a task")
            .error_for_status()
            .expect("the server refused to create a task");
    };
    let column = || -> Vec<String> {
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
            .filter(|task| task["status"] == "todo")
            .map(|task| {
                task["title"]
                    .as_str()
                    .expect("expected a title")
                    .to_string()
            })
            .collect()
    };
    let sort = |sort: Option<&str>| {
        served
            .client
            .post(format!(
                "{}/api/session/{session_id}/columns/todo/sort",
                served.base_url
            ))
            .json(&serde_json::json!({ "sort": sort }))
            .send()
            .expect("failed to sort the column")
            .error_for_status()
            .expect("the server refused to sort the column");
    };

    for title in [
        "Step 10 - ship",
        "step 2 - test",
        "Step 01 - build",
        "fix login",
    ] {
        create(title);
    }

    sort(Some("alphabetical"));
    assert_eq!(
        column(),
        [
            "fix login",
            "Step 01 - build",
            "step 2 - test",
            "Step 10 - ship"
        ],
        "numbers are read as numbers, and case is not an order"
    );

    sort(Some("numerical"));
    assert_eq!(
        column(),
        [
            "Step 01 - build",
            "step 2 - test",
            "Step 10 - ship",
            "fix login"
        ],
        "a title with no number goes last"
    );

    sort(None);
    assert_eq!(
        column(),
        [
            "Step 10 - ship",
            "step 2 - test",
            "Step 01 - build",
            "fix login"
        ],
        "unsorted, the column is back in the order the cards were put in"
    );
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
        .json(
            &serde_json::json!({ "title": "Fix the login page", "status": "todo", "joins": "top" }),
        )
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
