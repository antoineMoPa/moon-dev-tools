//! Cursor-based shell sockets resume without resetting or duplicating terminal output.
use crate::{api::terminal_stream::decode, pass_keys::of_this_test_run};
use std::time::Duration;
use tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

#[test]
fn browser_terminal_socket_resumes_after_the_delivered_cursor() {
    let served = super::serve("terminal-stream-resume");
    let session = served.open_session();
    let response: serde_json::Value = served
        .client
        .post(format!(
            "{}/api/session/{session}/terminals",
            served.base_url
        ))
        .json(&serde_json::json!({"command":null}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    let terminal = response["terminal_id"].as_str().unwrap();
    let base = format!(
        "{}/api/session/{session}/terminals/{terminal}/socket?moon_terminal_stream=true",
        served.base_url.replacen("http://", "ws://", 1)
    );
    let connect = |url: &str| {
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", of_this_test_run().generate())).unwrap(),
        );
        let (socket, _) = tungstenite::connect(request).unwrap();
        socket
    };
    let mut first = connect(&base);
    let first_frame = first.read().unwrap();
    let Message::Binary(first_frame) = first_frame else {
        panic!("expected cursor frame")
    };
    let (cursor, _) = decode(&first_frame).unwrap();
    first.close(None).unwrap();

    let mut resumed = connect(&format!("{base}&moon_terminal_cursor={cursor}"));
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = resumed.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
    }
    let Message::Binary(frame) = resumed.read().unwrap() else {
        panic!("expected resume frame")
    };
    let (resumed_cursor, _) = decode(&frame).unwrap();
    assert!(resumed_cursor >= cursor);
    resumed
        .send(Message::Text(
            serde_json::json!({"type":"input","data":"echo resume-probe\r"})
                .to_string()
                .into(),
        ))
        .unwrap();
    loop {
        let Message::Binary(frame) = resumed.read().unwrap() else {
            continue;
        };
        let (next, bytes) = decode(&frame).unwrap();
        assert!(
            next > resumed_cursor,
            "already delivered output was replayed"
        );
        if String::from_utf8_lossy(bytes).contains("resume-probe") {
            break;
        }
    }
    // Closing the browser stream never ends the underlying shell.
    resumed.close(None).unwrap();
    served
        .client
        .delete(format!(
            "{}/api/session/{session}/terminals/{terminal}",
            served.base_url
        ))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
}
