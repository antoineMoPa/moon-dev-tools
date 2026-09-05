//! What each shape of command line parses to.

use super::args::*;
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

#[test]
fn parse_install_launchers_command() {
    assert_eq!(parse(&["install-launchers"]), CliCommand::InstallLaunchers);
}

/// `install-launchers` takes nothing, so an argument after it is a mistake rather than a
/// path to review.
#[test]
fn install_launchers_takes_no_arguments() {
    let error = parse_cli_args(
        vec!["install-launchers".to_string(), "extra".to_string()],
        Frame::Review,
    )
    .expect_err("expected an argument after install-launchers to be rejected");

    assert!(error.to_string().contains("Usage:"));
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
