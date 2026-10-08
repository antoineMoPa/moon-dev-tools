//! What each shape of command line parses to.

use std::path::Path;

use super::args::*;
use super::command::*;
use super::*;

fn parse(args: &[&str]) -> CliCommand {
    parse_cli_args(
        args.iter().map(|arg| arg.to_string()).collect(),
        Frame::Review,
    )
    .expect("expected CLI args to parse")
}

#[test]
fn parse_bare_review_of_the_working_tree() {
    assert_eq!(
        parse(&[]),
        CliCommand::Review {
            target: ReviewTarget::WorkingTree,
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn parse_dot_as_current_directory_review() {
    assert_eq!(
        parse(&["."]),
        CliCommand::Review {
            target: ReviewTarget::CurrentDirectory,
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn parse_single_path_as_working_tree_pathspec() {
    assert_eq!(
        parse(&["packages/app/src/example.ts"]),
        CliCommand::Review {
            target: ReviewTarget::Path("packages/app/src/example.ts".to_string()),
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn a_revision_range_is_told_apart_from_a_path() {
    assert!(is_revision_range("main..egui-version"));
    assert!(is_revision_range("main...egui-version"));
    assert!(is_revision_range("release/1.0..main"));
    assert!(!is_revision_range("src/main.rs"));
    assert!(!is_revision_range(".."));
    assert!(!is_revision_range("../sibling/file.rs"));
    assert!(!is_revision_range("main.."));
}

#[test]
fn a_range_of_branches_is_reviewed_as_a_diff_against_its_base() {
    let request = review_open_request(
        Path::new("/repo"),
        ReviewTarget::Path("main..egui-version".to_string()),
        None,
        Path::new("/repo"),
    )
    .expect("expected review request");

    assert_eq!(
        request.diff_target.base.as_deref(),
        Some("main..egui-version")
    );
    assert_eq!(request.diff_target.pathspec, None);
    assert_eq!(request.active_commit, None);
}

#[test]
fn working_tree_review_ignores_current_directory_pathspec() {
    let request = review_open_request(
        Path::new("/repo"),
        ReviewTarget::WorkingTree,
        Some("src".to_string()),
        Path::new("/repo/src"),
    )
    .expect("expected review request");

    assert_eq!(request.diff_target.base, None);
    assert_eq!(request.diff_target.pathspec, None);
    assert_eq!(request.active_commit, None);
}

#[test]
fn current_directory_review_uses_current_directory_pathspec() {
    let request = review_open_request(
        Path::new("/repo"),
        ReviewTarget::CurrentDirectory,
        Some("src".to_string()),
        Path::new("/repo/src"),
    )
    .expect("expected review request");

    assert_eq!(request.diff_target.base, None);
    assert_eq!(request.diff_target.pathspec.as_deref(), Some("src"));
    assert_eq!(request.active_commit, None);
}

#[test]
fn single_path_review_uses_repo_relative_pathspec() {
    let request = review_open_request(
        Path::new("/repo"),
        ReviewTarget::Path("src/example.ts".to_string()),
        Some("packages/app".to_string()),
        Path::new("/repo/packages/app"),
    )
    .expect("expected review request");

    assert_eq!(request.diff_target.base, None);
    assert_eq!(
        request.diff_target.pathspec.as_deref(),
        Some("packages/app/src/example.ts")
    );
    assert_eq!(request.active_commit, None);
}

#[test]
fn parse_two_paths_as_file_comparison() {
    assert_eq!(
        parse(&["a.txt", "b.txt"]),
        CliCommand::Review {
            target: ReviewTarget::Comparison(["a.txt".to_string(), "b.txt".to_string()]),
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn comparison_paths_are_relative_to_current_directory() {
    let request = review_open_request(
        Path::new("/repo"),
        ReviewTarget::Comparison(["a.txt".to_string(), "nested/b.txt".to_string()]),
        Some("src".to_string()),
        Path::new("/repo/src"),
    )
    .expect("expected review request");

    assert_eq!(
        request.diff_target.comparison,
        Some([
            "/repo/src/a.txt".to_string(),
            "/repo/src/nested/b.txt".to_string()
        ])
    );
    assert_eq!(request.active_commit, None);
}

#[test]
fn parse_diff_review_against_a_named_ref() {
    assert_eq!(
        parse(&["diff", "dev"]),
        CliCommand::Review {
            target: ReviewTarget::Diff("dev".to_string()),
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn parse_bare_short_sha_as_commit_review() {
    assert_eq!(
        parse(&["4542abe"]),
        CliCommand::Review {
            target: ReviewTarget::Commit("4542abe".to_string()),
            source: ReviewSource::ThisMachine,
        }
    );
}

#[test]
fn parse_diff_short_sha_as_range_diff_review() {
    assert_eq!(
        parse(&["diff", "4542abe"]),
        CliCommand::Review {
            target: ReviewTarget::Diff("4542abe".to_string()),
            source: ReviewSource::ThisMachine,
        }
    );
}

/// The command line names the window first, and everything after it is what that window
/// opens on.
#[test]
fn a_window_is_named_by_the_word_after_moon() {
    assert_eq!(
        parse_command(None, vec!["review".to_string(), "src".to_string()])
            .expect("expected it to parse"),
        MoonCommand::Window {
            frame: Frame::Review,
            args: vec!["src".to_string()]
        }
    );
    assert_eq!(
        parse_command(None, vec!["tasks".to_string()]).expect("expected it to parse"),
        MoonCommand::Window {
            frame: Frame::Tasks,
            args: Vec::new()
        }
    );
}

/// `moon shell .` and `moon shell <folder>` are a shell in that folder as a tab of a window
/// that is already open, the way `moon edit` is a file in one - not another window.
#[test]
fn a_shell_in_a_folder_joins_a_window_that_is_open() {
    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };

    assert_eq!(
        parse(&["shell", "."]),
        MoonCommand::OpenShell {
            path: ".".to_string()
        }
    );
    assert_eq!(
        parse(&["shell", "src/native"]),
        MoonCommand::OpenShell {
            path: "src/native".to_string()
        }
    );
}

/// Everything else `moon shell` is asked is the window's own: a bare `moon shell` opens a
/// window, and so does anything with an option in it.
#[test]
fn a_shell_asked_for_any_other_way_opens_a_window() {
    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };
    let window = |args: &[&str]| MoonCommand::Window {
        frame: Frame::Shell,
        args: args.iter().map(|arg| arg.to_string()).collect(),
    };

    assert_eq!(parse(&["shell"]), window(&[]));
    assert_eq!(parse(&["shell", "--pick"]), window(&["--pick"]));
    assert_eq!(
        parse(&["shell", "--repo", "/repos/project"]),
        window(&["--repo", "/repos/project"])
    );
    assert_eq!(parse(&["shell", "--help"]), window(&["--help"]));
    // A review's two paths are not a folder, so the window reads them and says what it makes
    // of them.
    assert_eq!(parse(&["shell", "a", "b"]), window(&["a", "b"]));
    // A folder is only a shell's business: the other windows open on it as they always have.
    assert_eq!(
        parse(&["review", "."]),
        MoonCommand::Window {
            frame: Frame::Review,
            args: vec![".".to_string()]
        }
    );
}

/// A macOS bundle passes no arguments at all, so the window it opens is the one its plist
/// named in the environment - see `crate::cli::FRAME_ENV`.
#[test]
fn a_window_started_from_a_launcher_opens_on_the_frame_the_launcher_named() {
    assert_eq!(
        parse_command(Some(Frame::Shell), Vec::new()).expect("expected it to parse"),
        MoonCommand::Window {
            frame: Frame::Shell,
            args: Vec::new()
        }
    );
}

#[test]
fn parse_install_launchers_command() {
    assert_eq!(
        parse_command(None, vec!["install-launchers".to_string()]).expect("expected it to parse"),
        MoonCommand::InstallLaunchers
    );
}

/// `install-launchers` takes nothing, so an argument after it is a mistake rather than a
/// path to review.
#[test]
fn install_launchers_takes_no_arguments() {
    let error = parse_command(
        None,
        vec!["install-launchers".to_string(), "extra".to_string()],
    )
    .expect_err("expected an argument after install-launchers to be rejected");

    assert!(error.to_string().contains("takes nothing else"));
}

/// `generate-pass-key` prints a key and nothing else, so it takes nothing either.
#[test]
fn parse_generate_pass_key_command() {
    assert_eq!(
        parse_command(None, vec!["generate-pass-key".to_string()]).expect("expected it to parse"),
        MoonCommand::GeneratePassKey
    );
    let error = parse_command(
        None,
        vec!["generate-pass-key".to_string(), "extra".to_string()],
    )
    .expect_err("expected an argument after generate-pass-key to be rejected");
    assert!(error.to_string().contains("takes nothing else"));
}

/// `--pass-key` is what a remote window shows its server, in either spelling.
#[test]
fn parse_pass_key_with_remote() {
    for args in [
        &["--remote", "dev-box", "--pass-key", "id.mac"][..],
        &["--remote", "dev-box", "--pass-key=id.mac"][..],
    ] {
        assert_eq!(
            parse(args),
            CliCommand::Review {
                target: ReviewTarget::WorkingTree,
                source: ReviewSource::Remote {
                    target: "dev-box".to_string(),
                    repo_path: None,
                    pass_key: Some("id.mac".to_string()),
                },
            }
        );
    }
}

/// A window of this machine reaches its own server without HTTP, so a key given to it is a
/// mistake rather than something to ignore.
#[test]
fn pass_key_without_remote_is_rejected() {
    let error = parse_cli_args(
        vec!["--pass-key".to_string(), "id.mac".to_string()],
        Frame::Review,
    )
    .expect_err("expected --pass-key without --remote to be rejected");

    assert!(
        error.to_string().contains("goes with --remote"),
        "got {error}"
    );
}

/// `licenses` prints what is compiled in, and takes nothing.
#[test]
fn parse_licenses_command() {
    assert_eq!(
        parse_command(None, vec!["licenses".to_string()]).expect("expected it to parse"),
        MoonCommand::Licenses
    );
    let error = parse_command(None, vec!["licenses".to_string(), "extra".to_string()])
        .expect_err("expected an argument after licenses to be rejected");
    assert!(error.to_string().contains("takes nothing else"));
}

/// What `moon licenses` prints names the work moon copied, not only the crates it links.
#[test]
fn third_party_licenses_carry_the_codex_notice() {
    // Codex's NOTICE, which is the one file that names Ratatui.
    assert!(THIRD_PARTY_LICENSES.contains("code derived from [Ratatui]"));
    assert!(THIRD_PARTY_LICENSES.contains("Apache License"));
}

/// The server is a command of its own now, and `--logs` is the only thing it takes.
#[test]
fn parse_serve_with_and_without_logs() {
    assert_eq!(
        parse_command(None, vec!["serve".to_string()]).expect("expected it to parse"),
        MoonCommand::Serve { logs: false }
    );
    assert_eq!(
        parse_command(None, vec!["serve".to_string(), "--logs".to_string()])
            .expect("expected it to parse"),
        MoonCommand::Serve { logs: true }
    );
}

/// `moon desktop` is the session: the folder it opens on is optional, and it takes nothing
/// else - the window it opens is the whole screen, not a review of something.
#[test]
fn parse_desktop_with_and_without_a_folder() {
    assert_eq!(
        parse_command(None, vec!["desktop".to_string()]).expect("expected it to parse"),
        MoonCommand::Desktop { path: None }
    );
    assert_eq!(
        parse_command(None, vec!["desktop".to_string(), "/srv/work".to_string()])
            .expect("expected it to parse"),
        MoonCommand::Desktop {
            path: Some("/srv/work".to_string())
        }
    );

    let error = parse_command(
        None,
        vec![
            "desktop".to_string(),
            "/srv/work".to_string(),
            "/srv/other".to_string(),
        ],
    )
    .expect_err("expected two folders to be refused");
    assert!(error.to_string().contains("one folder"), "got {error}");
}

/// A word that names no command says so, rather than being read as something to review.
#[test]
fn a_command_that_is_not_one_is_refused() {
    let error = parse_command(None, vec!["frobnicate".to_string()])
        .expect_err("expected an unknown command to be rejected");

    assert!(error.to_string().contains("is not a command"));
}

/// `moon` on its own has nothing to do, so it says what it can do.
#[test]
fn no_arguments_at_all_is_the_help() {
    assert_eq!(
        parse_command(None, Vec::new()).expect("expected it to parse"),
        MoonCommand::Help
    );
}

/// `--repo` on its own names the repo the window opens on, which is how a restarted
/// window comes back where it was rather than on the launch screen.
#[test]
fn parse_repo_as_the_window_opening_on_that_repo() {
    assert_eq!(
        parse(&["--repo", "/home/you/project"]),
        CliCommand::OpenRepo("/home/you/project".to_string())
    );
    assert_eq!(
        parse(&["--repo=/home/you/project"]),
        CliCommand::OpenRepo("/home/you/project".to_string())
    );
}

/// With `--remote` the path is on the far machine, and it is the remote window that opens
/// on it rather than one of this machine.
#[test]
fn parse_repo_with_remote_as_a_path_on_that_machine() {
    assert_eq!(
        parse(&["--remote", "dev-box", "--repo", "/home/you/project"]),
        CliCommand::Review {
            target: ReviewTarget::WorkingTree,
            source: ReviewSource::Remote {
                target: "dev-box".to_string(),
                repo_path: Some("/home/you/project".to_string()),
                pass_key: None,
            },
        }
    );
}

/// A repo to open is the whole of what that window was asked for, so a review target
/// beside it is a mistake rather than something to narrow it to.
#[test]
fn a_repo_to_open_takes_nothing_else() {
    let error = parse_cli_args(
        vec!["--repo".to_string(), "/repo".to_string(), "src".to_string()],
        Frame::Review,
    )
    .expect_err("expected a review target beside --repo to be rejected");

    assert!(error.to_string().contains("takes nothing else"));
}

#[test]
fn parse_short_version_option() {
    assert_eq!(parse(&["-v"]), CliCommand::Version);
}

#[test]
fn parse_long_version_option() {
    assert_eq!(parse(&["--version"]), CliCommand::Version);
}

#[test]
fn parse_rejects_unknown_options() {
    let error = parse_cli_args(vec!["-ns".to_string()], Frame::Review)
        .expect_err("expected an unknown option to be rejected");

    assert!(error.to_string().contains("unknown option: -ns"));
}

/// A window opens on the folder it was started in, whatever git makes of that folder.
#[test]
fn a_window_started_in_a_folder_opens_on_that_folder() {
    let folder = std::env::temp_dir();

    assert_eq!(
        folder_when_there_is_no_repo(&folder).expect("expected the folder it was started in"),
        folder
    );
}

/// A window opened from a desktop launcher is started by the OS at the root of the
/// filesystem. Nobody works there, so it opens where the last one did.
#[test]
fn a_window_started_at_the_root_of_the_filesystem_opens_on_the_last_project() {
    let last_project = std::env::temp_dir();
    let mut saved = crate::settings::Settings::default();
    saved.remember_project(&last_project.display().to_string());
    crate::settings::store(&saved).expect("expected the settings to be written");

    let opened = folder_when_there_is_no_repo(Path::new("/")).expect("expected the last project");

    if let Some(path) = crate::settings::path() {
        let _ = std::fs::remove_file(path);
    }
    assert_eq!(opened, last_project);
}

/// `moon tasks new <title>` writes a card rather than opening a window, and the title is the
/// whole of the rest of the line so an unquoted one still reads as one title.
#[test]
fn the_board_makes_a_card_from_the_command_line() {
    assert_eq!(
        parse_command(
            None,
            vec![
                "tasks".to_string(),
                "new".to_string(),
                "fix the races".to_string()
            ]
        )
        .expect("expected it to parse"),
        MoonCommand::NewTask {
            title: "fix the races".to_string()
        }
    );
    assert_eq!(
        parse_command(
            None,
            vec![
                "tasks".to_string(),
                "new".to_string(),
                "fix".to_string(),
                "the".to_string(),
                "races".to_string()
            ]
        )
        .expect("expected it to parse"),
        MoonCommand::NewTask {
            title: "fix the races".to_string()
        }
    );

    let error = parse_command(None, vec!["tasks".to_string(), "new".to_string()])
        .expect_err("a card with no title is refused");
    assert!(error.to_string().contains("title"), "{error}");

    // The other two windows have no such word: `moon review new` is a path called `new`.
    assert_eq!(
        parse_command(None, vec!["review".to_string(), "new".to_string()])
            .expect("expected it to parse"),
        MoonCommand::Window {
            frame: Frame::Review,
            args: vec!["new".to_string()]
        }
    );
}

/// `moon tasks list` and `moon tasks move <column>` read and move the board's cards without a
/// window, as `new` makes one, and the column's name is the whole of the rest of the line so
/// an unquoted `IN PROGRESS` still reads as one name.
#[test]
fn the_board_lists_and_moves_its_cards_from_the_command_line() {
    let parse =
        |args: &[&str]| parse_command(None, args.iter().map(|arg| arg.to_string()).collect());
    let moved_to = |column: &str| MoonCommand::MoveTask {
        column: column.to_string(),
    };

    assert_eq!(
        parse(&["tasks", "list"]).expect("expected it to parse"),
        MoonCommand::ListTasks
    );
    assert_eq!(
        parse(&["tasks", "move", "IN", "PROGRESS"]).expect("expected it to parse"),
        moved_to("IN PROGRESS")
    );
    assert_eq!(
        parse(&["tasks", "move", "IN PROGRESS"]).expect("expected it to parse"),
        moved_to("IN PROGRESS")
    );
    // As it was typed: whether the board has a column called that is the board's to say.
    assert_eq!(
        parse(&["tasks", "move", "backburner"]).expect("expected it to parse"),
        moved_to("backburner")
    );

    let error = parse(&["tasks", "list", "DONE"]).expect_err("a list of one column is refused");
    assert!(error.to_string().contains("takes nothing"), "{error}");
    let error = parse(&["tasks", "move"]).expect_err("a move to no column is refused");
    assert!(error.to_string().contains("needs the column"), "{error}");
    let error = parse(&["tasks", "move", "--top", "DONE"]).expect_err("an option is refused");
    assert!(
        error.to_string().contains("no options, not --top"),
        "{error}"
    );

    // Every other word after `tasks` is still something to open the window on, and the
    // other two windows have no such words: `moon review list` is a path called `list`.
    for (frame, opened_on) in [
        (Frame::Tasks, &["."][..]),
        (Frame::Tasks, &["src/list"]),
        (Frame::Tasks, &["diff", "dev"]),
        (Frame::Tasks, &["--pick"]),
        (Frame::Review, &["list"]),
        (Frame::Shell, &["move", "DONE"]),
    ] {
        let mut line = vec![frame.subcommand()];
        line.extend(opened_on);
        assert_eq!(
            parse(&line).expect("expected it to parse"),
            MoonCommand::Window {
                frame,
                args: opened_on.iter().map(|arg| arg.to_string()).collect()
            }
        );
    }

    let help = help_text_for(Frame::Tasks);
    for said in [
        "  moon tasks list\n",
        "  moon tasks move <column>\n",
        "  moon tasks move IN PROGRESS\n",
        "`moon tasks list` prints",
        "`moon tasks move <column>` moves",
        "MOONREVIEW_TASK_DIR",
    ] {
        assert!(
            help.contains(said),
            "the board's help should say {said:?}:\n{help}"
        );
    }
    assert!(!help.lines().any(|line| line.trim_start().starts_with('#')));
    assert!(!help_text_for(Frame::Review).contains("move <column>"));
}

/// `--help` after a command is a question about it, whatever else was typed around it.
#[test]
fn every_command_answers_help_without_doing_anything() {
    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };

    assert_eq!(parse(&["list", "--help"]), MoonCommand::Help);
    assert_eq!(parse(&["serve", "--logs", "-h"]), MoonCommand::Help);
    assert_eq!(parse(&["licenses", "--help"]), MoonCommand::Help);
    assert_eq!(parse(&["install-launchers", "--help"]), MoonCommand::Help);
    assert_eq!(parse(&["generate-pass-key", "--help"]), MoonCommand::Help);
    // The board's window is where `new`, `list` and `move` are written up, so that is the
    // help they get.
    for asked in [
        &["tasks", "new", "--help"][..],
        &["tasks", "list", "-h"],
        &["tasks", "move", "IN PROGRESS", "--help"],
    ] {
        assert_eq!(
            parse(asked),
            MoonCommand::Window {
                frame: Frame::Tasks,
                args: vec!["--help".to_string()]
            },
            "{asked:?}"
        );
    }
}

/// `moon wire post <line>`: the line is the rest of the command line, quoted or not, and the
/// words starting with `@` in front of it are who it is for.
#[test]
fn a_line_for_the_wire_is_read_with_the_tags_in_front_of_it() {
    use super::wire::WireCommand;

    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };
    let post = |tags: &[&str], message: &str| {
        MoonCommand::Wire(WireCommand::Post {
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            message: message.to_string(),
        })
    };

    assert_eq!(
        parse(&["wire", "post", "rewriting src/cli - tests too"]),
        post(&[], "rewriting src/cli - tests too")
    );
    assert_eq!(
        parse(&["wire", "post", "rewriting", "src/cli"]),
        post(&[], "rewriting src/cli")
    );
    // Quoted, the tags arrive inside the one word the line is.
    assert_eq!(
        parse(&["wire", "post", "@fix-the-races are you in src/cli?"]),
        post(&["fix-the-races"], "are you in src/cli?")
    );
    assert_eq!(
        parse(&[
            "wire",
            "post",
            "@fix-the-races",
            "@bing-bong-313",
            "are",
            "you there?"
        ]),
        post(&["fix-the-races", "bing-bong-313"], "are you there?")
    );
    // Only the words in front are tags: one further on is part of what is said.
    assert_eq!(
        parse(&["wire", "post", "ask @fix-the-races about src/cli"]),
        post(&[], "ask @fix-the-races about src/cli")
    );
}

#[test]
fn a_line_for_the_wire_with_nothing_said_is_refused() {
    let refused = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect_err("expected it to be refused")
            .to_string()
    };

    assert!(refused(&["wire", "post"]).contains("needs the line to post"));
    assert!(refused(&["wire", "post", "  "]).contains("needs the line to post"));
    assert!(
        refused(&["wire", "post", "@fix-the-races"]).contains("needs a message after the tags")
    );
    assert!(
        refused(&["wire", "post", "@fix-the-races", "@bing-bong"])
            .contains("needs a message after the tags")
    );
    assert!(refused(&["wire", "post", "@ hello"]).contains("tags nobody"));
    assert!(refused(&["wire", "send", "hello"]).contains("`moon wire send` is not a command"));
}

/// The wire's rules are in its own help, which is what every way of asking gets - and
/// `moon --help` says the command is there.
#[test]
fn the_wire_answers_help_with_its_own_rules() {
    use super::wire::WireCommand;

    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };

    for asked in [
        &["wire"][..],
        &["wire", "--help"],
        &["wire", "-h"],
        &["wire", "post", "--help"],
        &["wire", "post", "@fix-the-races", "hello", "-h"],
    ] {
        assert_eq!(
            parse(asked),
            MoonCommand::Wire(WireCommand::Help),
            "{asked:?}"
        );
    }

    let help = super::wire::help_text();
    for said in [
        "moon wire post <one line>",
        "moon wire post @<handle> <one line>",
        ".moontasks/messageboard.txt",
        "latest 30 lines",
        "its only writer",
        "agent @<your handle> sent this message: <line>",
        "MOONREVIEW_TASK_DIR",
        "@board-task",
        "no running agent",
    ] {
        assert!(
            help.contains(said),
            "the wire's help should say {said:?}:\n{help}"
        );
    }
    assert!(help_text().contains("moon wire post <one line>"));
    assert!(help_text().contains("moon wire post \"rewriting src/cli\""));
}

/// `moon agent start|view|tell`: the task is the first word after the command, with or
/// without the `@` the wire writes a handle with.
#[test]
fn what_is_asked_of_an_agent_is_read_with_the_task_in_front() {
    use super::agent::AgentCommand;
    use crate::{api::AgentKind, terminal::Shown};

    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };
    let task = "fix-the-races".to_string();

    assert_eq!(
        parse(&["agent", "list"]),
        MoonCommand::Agent(AgentCommand::List)
    );
    assert_eq!(
        parse(&["agent", "start", "fix-the-races", "claude"]),
        MoonCommand::Agent(AgentCommand::Start {
            task: task.clone(),
            agent: AgentKind::Claude,
        })
    );
    assert_eq!(
        parse(&["agent", "start", "@fix-the-races", "opencode"]),
        MoonCommand::Agent(AgentCommand::Start {
            task: task.clone(),
            agent: AgentKind::OpenCode,
        })
    );
    assert_eq!(
        parse(&["agent", "view", "fix-the-races"]),
        MoonCommand::Agent(AgentCommand::View {
            task: task.clone(),
            wanted: Shown::Screen,
        })
    );
    for lines in [
        &["agent", "view", "fix-the-races", "--lines", "40"][..],
        &["agent", "view", "fix-the-races", "--lines=40"],
    ] {
        assert_eq!(
            parse(lines),
            MoonCommand::Agent(AgentCommand::View {
                task: task.clone(),
                wanted: Shown::LastRows(40),
            }),
            "{lines:?}"
        );
    }
    // The line is the rest, quoted or not.
    for line in [
        &["agent", "tell", "fix-the-races", "start with the tests"][..],
        &[
            "agent",
            "tell",
            "@fix-the-races",
            "start",
            "with",
            "the tests",
        ],
    ] {
        assert_eq!(
            parse(line),
            MoonCommand::Agent(AgentCommand::Tell {
                task: task.clone(),
                line: "start with the tests".to_string(),
            }),
            "{line:?}"
        );
    }
}

#[test]
fn what_is_asked_of_an_agent_with_a_part_missing_is_refused() {
    let refused = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect_err("expected it to be refused")
            .to_string()
    };

    assert!(refused(&["agent", "list", "fix-the-races"]).contains("so it takes nothing"));
    assert!(refused(&["agent", "start"]).contains("takes the task and the agent"));
    assert!(refused(&["agent", "start", "fix-the-races"]).contains("takes the task and the agent"));
    assert!(
        refused(&["agent", "start", "fix-the-races", "gemini"])
            .contains("gemini is no agent moon starts: it starts pi, claude, codex, opencode")
    );
    assert!(refused(&["agent", "view"]).contains("`moon agent view` takes the task"));
    // The task comes first, so an option in its place is not one.
    assert!(
        refused(&["agent", "view", "--lines", "40"]).contains("`moon agent view` takes the task")
    );
    assert!(refused(&["agent", "view", "--lines"]).contains("names no task"));
    assert!(
        refused(&["agent", "view", "fix-the-races", "--tail", "40"])
            .contains("`moon agent view` takes the task")
    );
    for not_a_count in ["0", "-3", "many"] {
        assert!(
            refused(&["agent", "view", "fix-the-races", "--lines", not_a_count])
                .contains("is not a count"),
            "{not_a_count}"
        );
    }
    assert!(refused(&["agent", "tell"]).contains("takes the task and the line"));
    assert!(refused(&["agent", "tell", "fix-the-races"]).contains("takes the task and the line"));
    assert!(
        refused(&["agent", "tell", "fix-the-races", " "]).contains("takes the task and the line")
    );
    assert!(refused(&["agent", "tell", "@", "hello"]).contains("names no task"));
    assert!(
        refused(&["agent", "stop", "fix-the-races"]).contains("`moon agent stop` is not a command")
    );
}

/// The commands are written up in their own help, which is what every way of asking gets -
/// and `moon --help` says they are there.
#[test]
fn the_agents_answer_help_with_their_own_commands() {
    use super::agent::AgentCommand;

    let parse = |args: &[&str]| {
        parse_command(None, args.iter().map(|arg| arg.to_string()).collect())
            .expect("expected it to parse")
    };

    for asked in [
        &["agent"][..],
        &["agent", "--help"],
        &["agent", "-h"],
        &["agent", "start", "--help"],
        &["agent", "tell", "fix-the-races", "hello", "-h"],
    ] {
        assert_eq!(
            parse(asked),
            MoonCommand::Agent(AgentCommand::Help),
            "{asked:?}"
        );
    }

    let help = super::agent::help_text();
    for said in [
        "moon agent list",
        "moon agent start <task> <pi|claude|codex|opencode>",
        "moon agent view <task> [--lines <n>]",
        "moon agent tell <task> <one line>",
    ] {
        assert!(
            help.contains(said),
            "the agents' help should say {said:?}:\n{help}"
        );
    }
    assert!(help_text().contains("moon agent <command>"));
}
