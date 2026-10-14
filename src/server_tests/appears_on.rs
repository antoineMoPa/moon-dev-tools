//! Cards made ahead of time over HTTP: the board leaves one out until its day.

use super::serve;
use crate::moontasks::{
    ColumnEnd, ColumnId,
    appears_on::{parse_day, today_here},
    store,
};

#[test]
fn a_card_is_left_off_the_board_until_its_day() {
    let served = serve("appears-on");
    let session_id = served.open_session();
    let today = today_here();
    let card = |title: &str, appears_on| {
        store::create_task_appearing_on(
            &served.root,
            title,
            &ColumnId::new("todo"),
            ColumnEnd::Bottom,
            Some(appears_on),
        )
        .expect("expected the card to be made");
    };
    card("due today", today);
    card(
        "due long ago",
        parse_day("2001-01-01").expect("expected a day"),
    );
    card(
        "next year",
        today
            .replace_year(today.year() + 1)
            .expect("expected a day"),
    );

    let tasks: serde_json::Value = served
        .client
        .get(format!(
            "{}/api/session/{session_id}/tasks",
            served.base_url
        ))
        .send()
        .expect("failed to read")
        .error_for_status()
        .expect("the server refused the read")
        .json()
        .expect("failed to decode");
    let titles: Vec<&str> = tasks
        .as_array()
        .expect("expected the cards")
        .iter()
        .map(|task| task["title"].as_str().expect("expected a title"))
        .collect();
    assert_eq!(titles, ["due today", "due long ago"]);
}
