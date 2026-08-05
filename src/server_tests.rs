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
