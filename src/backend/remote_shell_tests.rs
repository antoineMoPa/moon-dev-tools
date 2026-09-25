//! The server's shells driven through [`RemoteBackend`] over the terminal websocket: bytes
//! both ways, a shell started in a folder, a kick reaching a shell, and a shell's name.
//!
//! On the same footing as [`super::remote_tests`], whose server and repo these borrow:
//! nothing here reaches into the server's state, and every assertion goes over HTTP or the
//! socket.

use std::{
    fs,
    time::{Duration, Instant},
};

use crate::{
    api::OpenSessionRequest,
    backend::{Backend, remote::RemoteBackend},
};

use super::remote_tests::{pass_key, serve_a_repo};

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

/// `moon shell <folder>` reaches the server as a shell started in a folder of the repo, and
/// that is where the shell finds itself. A folder outside the repo is nobody's to start a
/// shell of this session in, so it is refused.
#[test]
fn a_remote_shell_starts_in_the_folder_it_was_asked_for() {
    let served = serve_a_repo("shell-in-folder");
    let folder = served.root.join("nested");
    fs::create_dir_all(&folder).expect("expected the folder");
    let folder = folder
        .canonicalize()
        .expect("expected the folder to resolve");
    let backend =
        RemoteBackend::connect(&served.base_url, pass_key()).expect("expected to reach the server");
    let opened = backend
        .open_session(OpenSessionRequest {
            repo_path: served.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected the remote session to open");

    let outside = std::env::temp_dir()
        .canonicalize()
        .expect("expected the temp dir to resolve");
    // The server's reason does not travel back over HTTP, only that it refused.
    backend
        .create_terminal_in_folder(&opened.session_id, &outside)
        .expect_err("a folder outside the repo is refused");

    let terminal_id = backend
        .create_terminal_in_folder(&opened.session_id, &folder)
        .expect("expected a remote shell to start in the folder");
    let attachment = backend
        .attach_terminal(&opened.session_id, &terminal_id)
        .expect("expected to attach to the remote shell");
    attachment
        .tty
        .write(b"printf 'in:%s:\\n' \"$(pwd)\"\n")
        .expect("expected to write to the remote shell");

    let wanted = format!("in:{}:", folder.display());
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(chunk) = attachment.output.recv_timeout(Duration::from_millis(200)) {
            seen.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&seen).contains(&wanted) {
                break;
            }
        }
    }
    let transcript = String::from_utf8_lossy(&seen);
    assert!(
        transcript.contains(&wanted),
        "the shell should be in {}; got:\n{transcript}",
        folder.display()
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
