//! What a shell finds, and what a window does when it is asked.

use std::path::{Path, PathBuf};

use super::{
    Answer, Ask, Instance, dir, home_instances_dir, running, socket_path, window::ShellAsks,
    windows_for, write_record,
};

fn instance(pid: u32, project_path: &str) -> Instance {
    in_front_at(pid, project_path, 0)
}

/// The same, for a window that was last in front at this time - which is what orders the
/// windows a file can fall back to.
fn in_front_at(pid: u32, project_path: &str, focused_at_unix: u64) -> Instance {
    Instance {
        pid,
        program: "moon shell".to_string(),
        project_path: project_path.to_string(),
        focused_at_unix,
    }
}

/// A window that is really there, so `running` keeps it: this test process.
fn this_process(project_path: &str) -> Instance {
    instance(std::process::id(), project_path)
}

/// A pid nothing is running under, which is what a window that was killed leaves behind.
/// Higher than any pid a system hands out, so it cannot come to be running mid-test.
const PID_OF_NOTHING: u32 = 4_194_303;

/// The window open on the file's project is asked before the one that is not.
#[test]
fn the_window_holding_the_file_is_asked_first() {
    let windows = windows_for(
        Path::new("/repos/project/src/main.rs"),
        None,
        vec![
            instance(1, "/repos/elsewhere"),
            instance(2, "/repos/project"),
        ],
    );

    assert_eq!(
        windows.iter().map(|window| window.pid).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

/// A submodule's window is open on a project inside another window's project, and the file
/// belongs to the one nearest it.
#[test]
fn the_innermost_project_holding_the_file_is_asked_first() {
    let windows = windows_for(
        Path::new("/repos/project/vendor/thing/src/lib.rs"),
        None,
        vec![
            instance(1, "/repos/project"),
            instance(2, "/repos/project/vendor/thing"),
        ],
    );

    assert_eq!(
        windows.iter().map(|window| window.pid).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

/// Typed in one of a window's own shells, the file goes to that window - even when another
/// window is open on a project that holds it too.
#[test]
fn the_shells_own_window_comes_first() {
    let windows = windows_for(
        Path::new("/repos/project/src/main.rs"),
        Some(1),
        vec![instance(1, "/repos/project"), instance(2, "/repos/project")],
    );

    assert_eq!(
        windows.iter().map(|window| window.pid).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

/// No window is open on the file's project, so the file falls to the window that was in
/// front most recently rather than to an error.
#[test]
fn a_file_no_window_is_open_on_goes_to_the_window_last_in_front() {
    let windows = windows_for(
        Path::new("/repos/project/src/main.rs"),
        None,
        vec![
            in_front_at(1, "/repos/elsewhere", 100),
            in_front_at(2, "/repos/somewhere-else", 200),
        ],
    );

    assert_eq!(
        windows.iter().map(|window| window.pid).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

/// A window that holds the file is a better answer than the one that happens to be in front,
/// however long ago it was last looked at.
#[test]
fn the_window_holding_the_file_beats_the_one_in_front() {
    let windows = windows_for(
        Path::new("/repos/project/src/main.rs"),
        None,
        vec![
            in_front_at(1, "/repos/elsewhere", 900),
            in_front_at(2, "/repos/project", 0),
        ],
    );

    assert_eq!(
        windows.iter().map(|window| window.pid).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn the_records_live_beside_the_settings() {
    let dir = home_instances_dir().expect("expected a directory");

    assert!(dir.ends_with(".moonreview/instances"), "got {dir:?}");
}

/// A window writes as it runs, so a test run must not be able to reach the real records - it
/// would find the windows the developer running it has open, and leave records of its own.
#[test]
fn a_test_run_never_writes_to_the_home_directory() {
    let dir = dir().expect("expected a directory");

    assert!(dir.starts_with(std::env::temp_dir()), "got {dir:?}");
}

#[test]
fn a_window_that_wrote_itself_down_is_found() {
    write_record(&this_process("/repos/project")).expect("expected the record to be written");

    assert_eq!(running(), vec![this_process("/repos/project")]);
}

/// A window that was killed cannot take its record away, so the next read does it.
#[test]
fn the_record_of_a_window_that_is_gone_is_cleared_away() {
    write_record(&instance(PID_OF_NOTHING, "/repos/project")).expect("expected a record");

    assert!(running().is_empty());
    assert!(
        !dir()
            .expect("expected a directory")
            .join(format!("{PID_OF_NOTHING}.json"))
            .exists()
    );
}

/// The whole of it, over the real socket: a window listening, a shell asking, and the file
/// waiting for the window's next frame.
#[test]
fn a_file_of_the_project_is_taken_and_waits_for_the_next_frame() {
    let project = temporary_project("taken");
    let file = project.join("notes.md");
    std::fs::write(&file, "# notes").expect("expected a file");

    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&Ask::OpenFile {
            path: file.display().to_string(),
            line: Some(12),
        })
        .expect("expected an answer");

    assert_eq!(answer, Answer::Opened);
    let arrived = asks.drain();
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].path, file);
    assert_eq!(arrived[0].line, Some(12));
}

/// A window takes a file of another project too: it opens a session on that project and puts
/// the file in a tab of it, which is what a shell asks it for when no window is open there.
#[test]
fn a_file_of_another_project_is_taken_as_well() {
    let project = temporary_project("another");
    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&Ask::OpenFile {
            path: "/somewhere/else/main.rs".to_string(),
            line: None,
        })
        .expect("expected an answer");

    assert_eq!(answer, Answer::Opened);
    let arrived = asks.drain();
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].path, Path::new("/somewhere/else/main.rs"));
}

/// A window whose repo is on another machine reads none of the files a shell here can name,
/// so it refuses and the shell tries the next window.
#[test]
fn a_window_on_another_machines_repo_is_refused_with_the_reason() {
    let project = temporary_project("remote");
    let asks = ShellAsks::listen("moon shell".to_string(), false, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&Ask::OpenFile {
            path: project.join("notes.md").display().to_string(),
            line: None,
        })
        .expect("expected an answer");

    assert_eq!(
        answer,
        Answer::Refused {
            reason: format!(
                "this window is open on {} on another machine",
                project.display()
            )
        }
    );
    assert!(asks.drain().is_empty());
}

/// The window being looked at is where a file with no window on its project goes, so coming
/// to the front is written into the record for a shell to read.
#[test]
fn a_window_that_comes_to_the_front_writes_when_it_did() {
    let project = temporary_project("in-front");
    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    assert_eq!(running()[0].focused_at_unix, 0);

    asks.came_to_the_front()
        .expect("expected the record to be written");

    assert!(running()[0].focused_at_unix > 0, "got {:?}", running()[0]);
}

/// Closing a window takes its record and its socket with it, so nothing looks for a window
/// that is not there.
#[test]
fn a_window_that_closes_takes_its_record_away() {
    let project = temporary_project("closed");
    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    assert_eq!(running().len(), 1);

    drop(asks);

    assert!(running().is_empty());
    assert!(
        !socket_path(std::process::id())
            .expect("expected a path")
            .exists()
    );
}

/// A directory to stand in for a project, named after the test so two cannot collide.
fn temporary_project(name: &str) -> PathBuf {
    let project = std::env::temp_dir().join(format!(
        "moonreview-test-project-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&project).expect("expected a project directory");
    project
        .canonicalize()
        .expect("expected the project to resolve")
}
