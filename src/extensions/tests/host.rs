//! What a script can call on beyond Rhai: programs, requests, JSON, and the window's files.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

use serde_json::json;

use super::{Heard, Scratch, script, start, system_path};
use crate::extensions::{Effect, Input};

#[test]
fn run_then_comes_back_as_an_update_carrying_what_the_program_printed() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ said: "nothing yet" } }
            fn view() { text(this.said) }
            fn update(event) {
                if "go" in event { run_then("printf", ["hello"], #{ back: true }); }
                if "back" in event { this.said = event.result.stdout; }
            }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("nothing yet"));

    // Act
    running.send(Input::Event(json!({ "go": true })));

    // Assert
    heard.until(&running, |heard| heard.shows("hello"));
}

/// A server on a port of its own that answers every request with what it was asked - the
/// method, the path, and the body if there was one - under a `201` and a header of its own.
fn echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind a port");
    let address = listener.local_addr().expect("a bound port has an address");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            answer_echo(stream);
        }
    });
    format!("http://{address}")
}

fn answer_echo(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("a stream clones"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("a request line");
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("a header line");
        if header.trim().is_empty() {
            break;
        }
        if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().expect("a length is a number");
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("the body");

    let mut words: Vec<&str> = request_line.split_whitespace().take(2).collect();
    let body = String::from_utf8(body).expect("a text body");
    if !body.is_empty() {
        words.push(&body);
    }
    let said = words.join(" ");
    let response = format!(
        "HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nX-Echo: yes\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{said}",
        said.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("failed to answer");
}

#[test]
fn fetch_answers_with_the_status_the_headers_and_the_body_as_text() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ said: "nothing yet" } }
            fn view() { text(this.said) }
            fn update(event) {
                let response = fetch(event.url + "/things", #{
                    method: "post",
                    headers: #{ "Content-Type": "text/plain" },
                    body: "hello",
                });
                this.said = `${response.status} ${response.ok} ${response.headers["x-echo"]} ${response.body}`;
            }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("nothing yet"));

    // Act
    running.send(Input::Event(json!({ "url": echo_server() })));

    // Assert
    heard.until(&running, |heard| {
        heard.shows("201 true yes POST /things hello")
    });
}

#[test]
fn fetch_then_comes_back_as_an_update_carrying_the_response() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ said: "nothing yet" } }
            fn view() { text(this.said) }
            fn update(event) {
                if "url" in event { fetch_then(event.url + "/later", #{}, #{ back: true }); }
                if "back" in event { this.said = event.result.body; }
            }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("nothing yet"));

    // Act
    running.send(Input::Event(json!({ "url": echo_server() })));

    // Assert
    heard.until(&running, |heard| heard.shows("GET /later"));
}

#[test]
fn a_request_that_gets_no_response_is_an_error_a_script_can_catch() {
    // Arrange: a port with nobody listening on it.
    let nobody = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind a port");
        format!(
            "http://{}",
            listener.local_addr().expect("a bound port has an address")
        )
    };
    let running = start(
        script(
            r#"
            fn init(root) { #{ said: "nothing yet" } }
            fn view() { text(this.said) }
            fn update(event) {
                try {
                    fetch(event.url);
                    this.said = "answered";
                } catch (error) {
                    this.said = "caught";
                }
            }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("nothing yet"));

    // Act
    running.send(Input::Event(json!({ "url": nobody })));

    // Assert
    heard.until(&running, |heard| heard.shows("caught"));
}

#[test]
fn a_program_that_outlives_its_timeout_is_stopped() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{ said: "nothing yet" } }
            fn view() { text(this.said) }
            fn update(event) {
                let ran = run("sleep", ["5"], #{ timeout_ms: 100 });
                this.said = `${ran.ok} ${ran.stderr}`;
            }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("nothing yet"));

    // Act
    let asked = Instant::now();
    running.send(Input::Event(json!({})));

    // Assert
    heard.until(&running, |heard| heard.only_text().starts_with("false"));
    assert!(
        heard.only_text().contains("was stopped"),
        "{}",
        heard.only_text()
    );
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "stopped, not waited out"
    );
}

#[test]
fn run_takes_a_folder_and_an_environment_to_run_in() {
    // Arrange
    let scratch = Scratch::new("run-options");
    scratch.write("sub/kept.txt", "");
    let running = start(
        script(
            r#"
            fn init(root) {
                let ran = run("sh", ["-c", "printf '%s %s' \"$(basename \"$(pwd)\")\" \"$GREETING\""],
                    #{ cwd: "sub", env: #{ GREETING: "hi" } });
                #{ said: ran.stdout }
            }
            fn view() { text(this.said) }
            "#,
        ),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.shows("sub hi"));
}

#[test]
fn parse_json_reads_a_list_as_well_as_an_object() {
    // Arrange / Act
    let running = start(
        script(
            r#"
            fn init(root) { #{ things: parse_json(`[{"name": "a"}, {"name": "b"}]`) } }
            fn view() { text(`${this.things.len()} ${this.things[1].name}`) }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    heard.until(&running, |heard| heard.shows("2 b"));
}

#[test]
fn an_answered_event_may_not_already_hold_the_answer() {
    // Arrange
    let running = start(
        script(
            r#"
            fn init(root) { #{} }
            fn view() { text("fine") }
            fn update(event) { run_then("true", [], #{ result: "mine" }); }
            "#,
        ),
        &std::env::temp_dir(),
        system_path(),
    );
    let mut heard = Heard::default();
    heard.until(&running, |heard| heard.shows("fine"));

    // Act
    running.send(Input::Event(json!({})));

    // Assert
    heard.until(&running, |heard| heard.error.is_some());
    let error = heard.error.clone().expect("an error");
    assert!(error.contains("already has a `result`"), "{error}");
}

#[test]
fn a_file_is_opened_at_the_line_asked_for() {
    // Arrange / Act
    let scratch = Scratch::new("open-at-line");
    let running = start(
        script(
            r#"
            fn init(root) { open_file(join_path(root, "a.txt"), 12); #{} }
            fn view() { text("fine") }
            "#,
        ),
        &scratch.dir,
        system_path(),
    );
    let mut heard = Heard::default();

    // Assert
    let opened = Effect::OpenFile {
        path: scratch.dir.join("a.txt"),
        line: Some(12),
    };
    heard.until(&running, |heard| heard.effects.contains(&opened));
}
