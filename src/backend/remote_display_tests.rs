//! Desktop tests through [`RemoteBackend`] - starts the server's desktop by starting an
//! application, watches it over its websocket, resizes it from the watching side, and checks
//! that it ends when its application exits.
//!
//! Runs only where a server has a desktop: Linux, with Xvfb installed, and `xlogo` from
//! x11-apps as the application to start. Like [`super::remote_shell_tests`], every assertion
//! goes over HTTP or the websocket.

use std::{
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};

use crate::{
    api::{
        OpenSessionRequest,
        display::{DisplayInput, DisplayPatch, StartApplicationRequest},
    },
    backend::{Backend, Socket, remote::RemoteBackend},
};

use super::remote_tests::{pass_key, serve_a_repo};

/// The first patch within a few seconds that `wanted` says yes to.
fn wait_for(socket: &Socket, wanted: impl Fn(&DisplayPatch) -> bool) -> DisplayPatch {
    let give_up = Instant::now() + Duration::from_secs(15);
    loop {
        let left = give_up.saturating_duration_since(Instant::now());
        let message = socket
            .heard
            .recv_timeout(left)
            .expect("the desktop never showed what was waited for");
        let patch = DisplayPatch::from_message(&message).expect("a desktop sends patches");
        if wanted(&patch) {
            return patch;
        }
    }
}

/// Whether the socket closes within `within`.
fn closes(socket: &Socket, within: Duration) -> bool {
    let give_up = Instant::now() + within;
    loop {
        match socket.heard.recv_timeout(Duration::from_millis(200)) {
            Ok(_) => continue,
            Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) if Instant::now() < give_up => continue,
            Err(RecvTimeoutError::Timeout) => return false,
        }
    }
}

#[test]
fn an_application_starts_the_desktop_which_is_over_when_the_application_is() {
    let served = serve_a_repo("display");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");
    let start = |command: &str| {
        backend.start_application(
            &opened.session_id,
            &StartApplicationRequest {
                command: command.to_owned(),
                width: 320,
                height: 200,
                scale: 1,
            },
        )
    };

    assert_eq!(backend.display().expect("expected an answer"), None);
    // The logo, until it is told to go: what closes it is the test's to decide.
    let desktop = start("xlogo & echo $! > logo.pid; wait").expect("expected the desktop");
    assert_eq!(
        backend.display().expect("expected an answer"),
        Some(desktop.clone())
    );
    // A second application is started on the desktop there is, not on another.
    assert_eq!(start("true").expect("expected the same desktop"), desktop);

    // Whoever starts watching is shown the whole view first, whatever was drawn before.
    let socket = backend
        .attach_display()
        .expect("expected the desktop's socket");
    let first = wait_for(&socket, |_| true);
    assert_eq!(
        (first.view, first.at, first.size),
        ([320, 200], [0, 0], [320, 200])
    );

    socket
        .said
        .say(
            serde_json::to_string(&DisplayInput::Resized {
                width: 400,
                height: 300,
            })
            .expect("an input is plain data"),
        )
        .expect("expected to reach the desktop");
    wait_for(&socket, |patch| {
        patch.view == [400, 300] && patch.size == [400, 300]
    });

    // The application ends, and with nothing left on it the desktop does: the socket closes,
    // which is how a pane hears, and the server has no desktop any more.
    let logo = std::fs::read_to_string(served.root.join("logo.pid")).expect("the logo's pid");
    std::process::Command::new("kill")
        .arg(logo.trim())
        .status()
        .expect("expected to stop the logo");
    assert!(
        closes(&socket, Duration::from_secs(20)),
        "the desktop should end once nothing is on it"
    );
    assert_eq!(backend.display().expect("expected an answer"), None);
}
