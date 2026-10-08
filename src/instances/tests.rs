//! What a shell finds, and what a window does when it is asked.

use std::path::{Path, PathBuf};

use super::{
    ANSWER_TIMEOUT, Answer, Ask, Instance, ask_process, dir, home_instances_dir, launch_in,
    running, server::ServerAsks, socket_path, window::ShellAsks, window_of, windows_for, wire,
    write_record,
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
            wait: false,
        })
        .expect("expected an answer");

    assert_eq!(answer, Answer::Opened);
    let arrived = asks.drain();
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].path, file);
    assert_eq!(arrived[0].line, Some(12));
}

/// `moon edit --wait`: the file is still open from the moment the window takes it - before
/// any frame has opened its tab - until the window lets it go, and then the shell is told it
/// has closed.
#[test]
fn a_waited_on_file_is_open_until_the_window_releases_it() {
    let project = temporary_project("waited");
    let file = project.join("COMMIT_EDITMSG");
    std::fs::write(&file, "").expect("expected a file");

    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    let window = this_process(&project.display().to_string());
    let still_open = Ask::StillOpen {
        path: file.display().to_string(),
    };

    let answer = window
        .ask(&Ask::OpenFile {
            path: file.display().to_string(),
            line: None,
            wait: true,
        })
        .expect("expected an answer");
    assert_eq!(answer, Answer::Opened);
    assert!(asks.drain()[0].wait);
    assert_eq!(
        window.ask(&still_open).expect("expected an answer"),
        Answer::StillOpen
    );

    asks.release(&file);

    assert_eq!(
        window.ask(&still_open).expect("expected an answer"),
        Answer::Closed
    );
}

/// A file opened without `--wait` is nothing the window holds on to.
#[test]
fn a_file_nobody_waits_on_reads_as_closed() {
    let project = temporary_project("not-waited");
    let file = project.join("notes.md");
    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    let window = this_process(&project.display().to_string());

    window
        .ask(&Ask::OpenFile {
            path: file.display().to_string(),
            line: None,
            wait: false,
        })
        .expect("expected an answer");

    assert_eq!(
        window
            .ask(&Ask::StillOpen {
                path: file.display().to_string(),
            })
            .expect("expected an answer"),
        Answer::Closed
    );
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
            wait: false,
        })
        .expect("expected an answer");

    assert_eq!(answer, Answer::Opened);
    let arrived = asks.drain();
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].path, Path::new("/somewhere/else/main.rs"));
}

/// `moon shell <folder>`: the folder is taken the way a file is, and kept apart from the
/// files - a shell is started in it rather than a tab opened on it.
#[test]
fn a_folder_for_a_shell_is_taken_and_kept_apart_from_the_files() {
    let project = temporary_project("shell-folder");
    let folder = project.join("src");
    std::fs::create_dir_all(&folder).expect("expected a folder");

    let asks = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&Ask::OpenShell {
            folder: folder.display().to_string(),
        })
        .expect("expected an answer");

    assert_eq!(answer, Answer::Opened);
    assert!(asks.drain().is_empty(), "a folder is no file to open");
    let shells = asks.drain_shells();
    assert_eq!(shells.len(), 1);
    assert_eq!(shells[0].folder, folder);
    assert!(asks.drain_shells().is_empty(), "drained once");
}

/// A window whose repo is on another machine reads none of the folders a shell here can name
/// either, so a shell asked for there is refused the same way.
#[test]
fn a_shell_in_a_window_on_another_machines_repo_is_refused() {
    let project = temporary_project("remote-shell");
    let asks = ShellAsks::listen("moon shell".to_string(), false, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&Ask::OpenShell {
            folder: project.display().to_string(),
        })
        .expect("expected an answer");

    assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");
    assert!(asks.drain_shells().is_empty());
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
            wait: false,
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

fn wire_ask(terminal_id: &str, message: &str) -> Ask {
    Ask::Wire {
        terminal_id: terminal_id.to_string(),
        sender: "fix-the-races".to_string(),
        recipient: "bing-bong".to_string(),
        message: message.to_string(),
    }
}

/// `moon wire post @handle`: the line is taken, kept apart from the files and the folders,
/// and waits for the window's next frame in the order it was asked for.
#[test]
fn a_line_of_the_wire_is_taken_and_waits_in_the_order_it_was_asked_for() {
    let project = temporary_project("wired");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    let window = window_of(std::process::id()).expect("expected the window to be written down");

    for message in ["one", "two", "three"] {
        assert_eq!(
            window
                .ask(&wire_ask("terminal-a", message))
                .expect("expected an answer"),
            Answer::Wired
        );
    }
    wire(&window, &wire_ask("terminal-b", "four")).expect("expected the line to be taken");

    assert!(asks.drain().is_empty(), "a line is no file to open");
    assert!(asks.drain_shells().is_empty(), "nor a folder for a shell");
    let wired = asks.drain_wired();
    assert_eq!(
        wired
            .iter()
            .map(|line| (line.terminal_id.as_str(), line.message.as_str()))
            .collect::<Vec<_>>(),
        [
            ("terminal-a", "one"),
            ("terminal-a", "two"),
            ("terminal-a", "three"),
            ("terminal-b", "four"),
        ]
    );
    assert_eq!(wired[0].sender, "fix-the-races");
    assert_eq!(wired[0].recipient, "bing-bong");
    assert!(asks.drain_wired().is_empty(), "drained once");
}

/// The line is a shell's to be typed and sent, so what arrives is held to one line of text
/// here, whoever sent it.
#[test]
fn a_line_of_the_wire_that_is_not_one_line_is_refused() {
    let project = temporary_project("wired-two-lines");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    let window = this_process(&project.display().to_string());

    for not_a_line in ["one\ntwo", "sent early\r", ""] {
        let answer = window
            .ask(&wire_ask("terminal-a", not_a_line))
            .expect("expected an answer");
        assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");
    }
    let error = wire(&window, &wire_ask("terminal-a", "one\ntwo")).expect_err("expected a refusal");
    assert!(error.to_string().contains("refused the line"), "{error}");
    assert!(asks.drain_wired().is_empty());
}

/// A window whose repo is on another machine holds none of the shells: they are that
/// machine's, and its server is the moon that started them.
#[test]
fn a_line_of_the_wire_is_refused_by_a_window_on_another_machines_repo() {
    let project = temporary_project("wired-remote");
    let asks = ShellAsks::listen("moon tasks".to_string(), false, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");

    let answer = this_process(&project.display().to_string())
        .ask(&wire_ask("terminal-a", "hello"))
        .expect("expected an answer");

    assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");
    assert!(asks.drain_wired().is_empty());
}

/// A moon with no window - a `moon serve` - has no record, and one that has gone leaves a
/// record that is cleared: neither is a window a line can be handed to.
#[test]
fn only_a_running_window_that_wrote_itself_down_is_the_window_of_its_process() {
    assert_eq!(window_of(std::process::id()), None);

    write_record(&instance(PID_OF_NOTHING, "/repos/project")).expect("expected a record");
    assert_eq!(window_of(PID_OF_NOTHING), None);

    write_record(&this_process("/repos/project")).expect("expected a record");
    assert_eq!(
        window_of(std::process::id()),
        Some(this_process("/repos/project"))
    );
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

/// A window's answers about agents that does what it is asked, and fails a start of Codex
/// the way a machine without one would.
struct AnswersAboutAgents;

impl super::window::AgentAsks for AnswersAboutAgents {
    fn start(
        &self,
        _repo_path: &str,
        task_id: &str,
        agent: crate::api::AgentKind,
    ) -> anyhow::Result<String> {
        if agent == crate::api::AgentKind::Codex {
            anyhow::bail!("Codex is not installed here");
        }
        Ok(format!("{task_id} claude - 1"))
    }

    fn tell(&self, terminal_id: &str, _line: &str) -> anyhow::Result<()> {
        if terminal_id == "terminal-nobody" {
            anyhow::bail!("unknown terminal {terminal_id}");
        }
        Ok(())
    }

    fn shown(&self, terminal_id: &str, _wanted: crate::terminal::Shown) -> anyhow::Result<String> {
        Ok(format!("the screen of {terminal_id}\n"))
    }
}

fn start_ask(repo_path: &Path, agent: crate::api::AgentKind) -> Ask {
    Ask::StartAgent {
        repo_path: repo_path.display().to_string(),
        task_id: "fix-the-races".to_string(),
        agent,
    }
}

/// `moon agent start`, `tell` and `view`: each is answered with what came of it, there and
/// then, and leaves nothing waiting for the window's next frame.
#[test]
fn what_is_asked_about_an_agent_is_answered_with_what_came_of_it() {
    let project = temporary_project("agents");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    asks.agents_answered_by(std::sync::Arc::new(AnswersAboutAgents));
    let window = window_of(std::process::id()).expect("expected the window to be written down");

    assert_eq!(
        super::start_agent(&window, "fix-the-races", crate::api::AgentKind::Claude)
            .expect("expected a start"),
        "fix-the-races claude - 1"
    );
    super::tell(&window, "terminal-a", "hello").expect("expected the line to be taken");
    assert_eq!(
        super::shown(&window, "terminal-a", crate::terminal::Shown::Screen)
            .expect("expected the screen"),
        "the screen of terminal-a\n"
    );

    assert!(asks.drain().is_empty());
    assert!(asks.drain_shells().is_empty());
    assert!(asks.drain_wired().is_empty());
}

/// What the window could not do is its refusal, in the words of whatever failed - which is
/// what the command that asked prints.
#[test]
fn what_a_window_could_not_do_about_an_agent_is_refused_with_the_reason() {
    let project = temporary_project("agents-failing");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    asks.agents_answered_by(std::sync::Arc::new(AnswersAboutAgents));
    let window = window_of(std::process::id()).expect("expected the window to be written down");

    let error = super::start_agent(&window, "fix-the-races", crate::api::AgentKind::Codex)
        .expect_err("expected a refusal");
    assert!(
        error
            .to_string()
            .contains("refused: Codex is not installed here"),
        "{error}"
    );
    let error = super::tell(&window, "terminal-nobody", "hello").expect_err("expected a refusal");
    assert!(
        error
            .to_string()
            .contains("refused: unknown terminal terminal-nobody"),
        "{error}"
    );
}

/// An agent is started by a window open on the board's own repo, on this machine: the run
/// is listed on the board in that window, and its shell is one only that window opens.
#[test]
fn an_agent_is_started_only_by_a_window_open_on_the_boards_repo() {
    let project = temporary_project("agents-here");
    let elsewhere = temporary_project("agents-elsewhere");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.agents_answered_by(std::sync::Arc::new(AnswersAboutAgents));
    let window = this_process(&project.display().to_string());
    let claude = crate::api::AgentKind::Claude;

    // Still on its launch screen.
    let answer = window
        .ask(&start_ask(&project, claude))
        .expect("expected an answer");
    assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");

    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    assert_eq!(
        window
            .ask(&start_ask(&elsewhere, claude))
            .expect("expected an answer"),
        Answer::Refused {
            reason: format!(
                "this window is open on {}, not on {}",
                project.display(),
                elsewhere.display()
            ),
        }
    );
    assert!(matches!(
        window
            .ask(&start_ask(&project, claude))
            .expect("expected an answer"),
        Answer::Started { .. }
    ));
}

/// A window that has not said who answers about agents - a ui test's, which holds no real
/// shells - refuses rather than answering as if nothing were asked.
#[test]
fn what_is_asked_about_an_agent_is_refused_by_a_window_that_answers_nothing_about_them() {
    let project = temporary_project("agents-unanswered");
    let asks = ShellAsks::listen("moon tasks".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    asks.on_project(&project.display().to_string())
        .expect("expected the record to be written");
    let window = this_process(&project.display().to_string());

    for ask in [
        start_ask(&project, crate::api::AgentKind::Claude),
        Ask::Tell {
            terminal_id: "terminal-a".to_string(),
            line: "hello".to_string(),
        },
        Ask::Shown {
            terminal_id: "terminal-a".to_string(),
            wanted: crate::terminal::Shown::Screen,
        },
    ] {
        let answer = window.ask(&ask).expect("expected an answer");
        assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");
    }
}

/// `moon launch`, over the real socket and with a real program: a window among others starts
/// nothing and says why; a window that is its machine's session starts the line of shell in
/// the folder asked from, each quoted word one argument, and answers a start that failed with
/// what the program said; and a `moon serve` answers a launch and nothing else.
#[test]
fn a_launch_is_answered_with_how_the_start_went_by_a_moon_that_starts_programs() {
    use crate::display::started::OnThisScreen;

    let folder = temporary_project("launched");
    std::fs::write(folder.join("a b.txt"), "").expect("expected a file");
    let moon = std::process::id();

    let window = ShellAsks::listen("moon shell".to_string(), true, egui::Context::default())
        .expect("expected a socket");
    let refusal = launch_in(moon, "true", &folder).expect_err("expected a refusal");
    assert!(
        format!("{refusal}").contains("has its own way to start a program"),
        "got {refusal}"
    );

    window.applications_started_by(std::sync::Arc::new(OnThisScreen));
    assert_eq!(
        launch_in(moon, "'test' '-f' 'a b.txt'", &folder).expect("expected it to start"),
        "this screen"
    );
    let failure = launch_in(moon, "echo no display >&2; exit 3", &folder)
        .expect_err("expected the failure");
    assert_eq!(
        format!("{failure}"),
        "`echo no display >&2; exit 3` ended (exit status: 3): no display"
    );

    drop(window);
    let _server = ServerAsks::listen(std::sync::Arc::new(OnThisScreen)).expect("expected a socket");
    assert_eq!(
        launch_in(moon, "true", &folder).expect("expected it to start"),
        "this screen"
    );
    let answer = ask_process(
        moon,
        &Ask::OpenShell {
            folder: folder.display().to_string(),
        },
        ANSWER_TIMEOUT,
    )
    .expect("expected an answer");
    assert!(matches!(answer, Answer::Refused { .. }), "got {answer:?}");
}
