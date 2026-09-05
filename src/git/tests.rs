//! Hunk collection against throwaway repositories built on disk.

use super::hunks::{
    collect_commit_hunks, collect_hunks, collect_session_hunks, local_change_summary_from_status,
};
use super::{
    branch_commits_since_default, canonicalize_repo, commit_history_page, list_submodule_repos,
    run_git, run_git_no_output,
};
use crate::api::{AgentKind, DiffTarget, RepoSession};
use std::collections::{HashMap, HashSet};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moonreview-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("failed to create temp test directory");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn diff_line_counts(patch: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;

    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }

    (added, removed)
}

fn init_test_repo(repo_root: &PathBuf) {
    fs::create_dir_all(repo_root).expect("failed to create repo directory");
    run_git_no_output(repo_root, &["init"]).expect("failed to init repo");
    run_git_no_output(repo_root, &["config", "user.email", "test@example.com"])
        .expect("failed to configure git email");
    run_git_no_output(repo_root, &["config", "user.name", "Test User"])
        .expect("failed to configure git user");
    run_git_no_output(repo_root, &["config", "commit.gpgsign", "false"])
        .expect("failed to disable git signing");
}

fn test_session(repo_root: PathBuf, active_commit: Option<String>) -> RepoSession {
    RepoSession {
        repo_path: repo_root,
        diff_target: DiffTarget::default(),
        active_commit,
        comments: HashMap::new(),
        comment_contexts: HashMap::new(),
        selected_agent: AgentKind::None,
        comment_dispatches: HashMap::new(),
    }
}

/// An SVG is reviewed as its picture, so however many `@@` runs git finds in the markup,
/// the file is one change - and that one hunk's patch is the whole file section, which
/// `git apply` takes as-is, so staging it stages the change whole.
#[test]
fn an_svg_is_one_hunk_however_many_places_it_changed_in() {
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    // Two edits far enough apart that git would otherwise split them into two hunks.
    let spacer = "  <rect width=\"1\" height=\"1\"/>\n".repeat(12);
    let svg = |first: &str, last: &str| {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\">\n  {first}\n{spacer}  {last}\n</svg>\n")
    };
    fs::write(
        repo_root.join("logo.svg"),
        svg("<g id=\"a\"/>", "<g id=\"z\"/>"),
    )
    .expect("failed to write the svg");
    run_git_no_output(&repo_root, &["add", "logo.svg"]).expect("failed to add the svg");
    run_git_no_output(&repo_root, &["commit", "-m", "Add the logo"]).expect("failed to commit");
    fs::write(
        repo_root.join("logo.svg"),
        svg("<g id=\"b\"/>", "<g id=\"y\"/>"),
    )
    .expect("failed to change the svg");

    let hunks =
        collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect the hunks");

    let svg_hunks: Vec<_> = hunks
        .iter()
        .filter(|hunk| hunk.file_path == "logo.svg")
        .collect();
    assert_eq!(
        svg_hunks.len(),
        1,
        "an image file is one change, not a card per run of markup"
    );
    let hunk = svg_hunks[0];
    assert_eq!(hunk.header, "Image changed");
    assert!(hunk.image_diff.is_some(), "the card shows the pictures");
    assert!(
        hunk.patch.matches("@@").count() >= 2,
        "both edits are in the one patch: {}",
        hunk.patch
    );

    super::apply_patch(&repo_root, &hunk.patch, true, false)
        .expect("the whole-file patch should stage cleanly");
    let staged = run_git(&repo_root, &["diff", "--cached", "--name-only"])
        .expect("failed to list staged files");
    assert!(
        staged.contains("logo.svg"),
        "staging the hunk stages the file"
    );
}

#[test]
fn collect_commit_hunks_returns_hunks_for_single_commit() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    let file_path = repo_root.join("example.txt");
    fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to write initial file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit initial file");

    fs::write(&file_path, "one\nTWO\nthree\n").expect("failed to write changed file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add change");
    run_git_no_output(&repo_root, &["commit", "-m", "change example"])
        .expect("failed to commit change");
    let commit = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");

    // Act
    let hunks =
        collect_commit_hunks(&repo_root, commit.trim()).expect("failed to collect commit hunks");

    // Assert
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].file_path, "example.txt");
    assert!(hunks[0].staged);
    assert_eq!(diff_line_counts(&hunks[0].patch), (1, 1));
}

#[test]
fn collect_hunks_compares_two_files_without_a_git_repository() {
    // Arrange
    let temp = TestDir::new();
    let before = temp.path.join("before.json");
    let after = temp.path.join("after.json");
    fs::write(&before, "{\"value\": 1}\n").expect("failed to write before file");
    fs::write(&after, "{\"value\": 2}\n").expect("failed to write after file");
    let target = DiffTarget {
        base: None,
        pathspec: None,
        comparison: Some([before.display().to_string(), after.display().to_string()]),
    };

    // Act
    let hunks = collect_hunks(&temp.path, &target).expect("failed to compare files");

    // Assert
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].file_path, after.display().to_string());
    assert!(!hunks[0].staged);
    assert_eq!(diff_line_counts(&hunks[0].patch), (1, 1));
}

#[test]
fn initial_staged_changes_are_available_before_first_commit() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    fs::write(repo_root.join("example.txt"), "initial contents\n")
        .expect("failed to write initial file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add initial file");

    // Act
    let hunks = collect_hunks(&repo_root, &DiffTarget::default())
        .expect("failed to collect initial staged hunks");
    let (base, branch_commits) =
        branch_commits_since_default(&repo_root).expect("failed to collect branch commits");
    let (history_commits, history_has_more) =
        commit_history_page(&repo_root, &HashSet::new(), 0, 50)
            .expect("failed to collect commit history");

    // Assert
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].file_path, "example.txt");
    assert!(hunks[0].staged);
    assert_eq!(diff_line_counts(&hunks[0].patch), (1, 0));
    assert_eq!(base, None);
    assert!(branch_commits.is_empty());
    assert!(history_commits.is_empty());
    assert!(!history_has_more);
}

/// `moonreview main..feature` reviews everything on one branch that is not on the other.
/// The range goes to git as it was typed; nothing here has to understand it.
#[test]
fn a_file_in_the_working_tree_can_be_written_back() {
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);
    fs::write(repo_root.join("lib.rs"), "fn one() {}\n").expect("failed to write");

    super::write_repo_file(&repo_root, "lib.rs", "fn two() {}\n").expect("expected the write");

    assert_eq!(
        fs::read_to_string(repo_root.join("lib.rs")).expect("failed to read back"),
        "fn two() {}\n"
    );
}

#[test]
fn writing_outside_the_repository_is_refused() {
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);
    fs::write(temp.path.join("outside.txt"), "secrets\n").expect("failed to write");

    let refused = super::write_repo_file(&repo_root, "../outside.txt", "changed\n");

    assert!(refused.is_err(), "a path out of the repo must be refused");
    assert_eq!(
        fs::read_to_string(temp.path.join("outside.txt")).expect("failed to read back"),
        "secrets\n",
        "and must not have written anything"
    );
}

#[test]
fn a_revision_range_collects_the_hunks_between_two_branches() {
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);
    fs::write(repo_root.join("lib.rs"), "fn one() {}\n").expect("failed to write");
    run_git_no_output(&repo_root, &["add", "-A"]).expect("failed to stage");
    run_git_no_output(&repo_root, &["commit", "-m", "first"]).expect("failed to commit");
    run_git_no_output(&repo_root, &["branch", "-M", "main"]).expect("failed to name main");
    run_git_no_output(&repo_root, &["checkout", "-b", "feature"]).expect("failed to branch");
    fs::write(repo_root.join("lib.rs"), "fn one() {}\nfn two() {}\n").expect("failed to write");
    run_git_no_output(&repo_root, &["add", "-A"]).expect("failed to stage");
    run_git_no_output(&repo_root, &["commit", "-m", "second"]).expect("failed to commit");

    let mut session = test_session(repo_root, None);
    session.diff_target = DiffTarget {
        base: Some("main..feature".to_string()),
        pathspec: None,
        comparison: None,
    };

    let hunks = collect_session_hunks(&session).expect("expected the range to diff");

    assert_eq!(hunks.len(), 1, "the branch adds one hunk");
    assert!(
        hunks[0].patch.contains("fn two()"),
        "the hunk should be the line the branch added, got:\n{}",
        hunks[0].patch
    );
}

#[test]
fn collect_session_hunks_uses_active_commit_when_present() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    let committed_path = repo_root.join("committed.txt");
    fs::write(&committed_path, "before\n").expect("failed to write committed file");
    run_git_no_output(&repo_root, &["add", "committed.txt"]).expect("failed to add file");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit initial file");

    fs::write(&committed_path, "after\n").expect("failed to change committed file");
    run_git_no_output(&repo_root, &["add", "committed.txt"]).expect("failed to add change");
    run_git_no_output(&repo_root, &["commit", "-m", "change committed"])
        .expect("failed to commit change");
    let commit = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");

    let local_path = repo_root.join("local.txt");
    fs::write(&local_path, "local\n").expect("failed to write local file");

    // Act
    let local_hunks = collect_session_hunks(&test_session(repo_root.clone(), None))
        .expect("failed to collect local hunks");
    let commit_hunks = collect_session_hunks(&test_session(
        repo_root.clone(),
        Some(commit.trim().to_string()),
    ))
    .expect("failed to collect active commit hunks");

    // Assert
    assert_eq!(local_hunks.len(), 1);
    assert_eq!(local_hunks[0].file_path, "local.txt");
    assert!(!local_hunks[0].staged);

    assert_eq!(commit_hunks.len(), 1);
    assert_eq!(commit_hunks[0].file_path, "committed.txt");
    assert!(commit_hunks[0].staged);
}

#[test]
fn branch_commits_since_default_prefers_origin_head_over_current_branch_upstream() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    let file_path = repo_root.join("example.txt");
    fs::write(&file_path, "base\n").expect("failed to write initial file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit initial file");
    let default_head = run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read HEAD");
    run_git_no_output(&repo_root, &["remote", "add", "origin", "."]).expect("failed to add remote");
    run_git_no_output(
        &repo_root,
        &["update-ref", "refs/remotes/origin/dev", default_head.trim()],
    )
    .expect("failed to create remote default ref");
    run_git_no_output(
        &repo_root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/dev",
        ],
    )
    .expect("failed to set remote default ref");

    run_git_no_output(&repo_root, &["checkout", "-b", "feature"])
        .expect("failed to create feature branch");
    fs::write(&file_path, "base\none\n").expect("failed to write first change");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add first change");
    run_git_no_output(&repo_root, &["commit", "-m", "first change"])
        .expect("failed to commit first change");
    fs::write(&file_path, "base\none\ntwo\n").expect("failed to write second change");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add second change");
    run_git_no_output(&repo_root, &["commit", "-m", "second change"])
        .expect("failed to commit second change");
    let feature_head =
        run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read feature HEAD");
    run_git_no_output(
        &repo_root,
        &[
            "update-ref",
            "refs/remotes/origin/feature",
            feature_head.trim(),
        ],
    )
    .expect("failed to create remote feature ref");
    run_git_no_output(&repo_root, &["config", "branch.feature.remote", "origin"])
        .expect("failed to configure upstream remote");
    run_git_no_output(
        &repo_root,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    )
    .expect("failed to configure upstream branch");

    // Act
    let (base, commits) =
        branch_commits_since_default(&repo_root).expect("failed to collect commits");

    // Assert
    assert_eq!(base.as_deref(), Some("origin/dev"));
    assert_eq!(
        commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["second change", "first change"]
    );
}

#[test]
fn branch_commits_since_default_falls_back_to_current_branch_upstream() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    let file_path = repo_root.join("example.txt");
    fs::write(&file_path, "base\n").expect("failed to write initial file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit initial file");

    run_git_no_output(&repo_root, &["remote", "add", "origin", "."]).expect("failed to add remote");
    run_git_no_output(&repo_root, &["checkout", "-b", "feature"])
        .expect("failed to create feature branch");
    fs::write(&file_path, "base\none\n").expect("failed to write feature change");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add feature change");
    run_git_no_output(&repo_root, &["commit", "-m", "feature change"])
        .expect("failed to commit feature change");
    let feature_head =
        run_git(&repo_root, &["rev-parse", "HEAD"]).expect("failed to read feature HEAD");
    run_git_no_output(
        &repo_root,
        &[
            "update-ref",
            "refs/remotes/origin/feature",
            feature_head.trim(),
        ],
    )
    .expect("failed to create remote feature ref");
    run_git_no_output(&repo_root, &["config", "branch.feature.remote", "origin"])
        .expect("failed to configure upstream remote");
    run_git_no_output(
        &repo_root,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    )
    .expect("failed to configure upstream branch");

    // Act
    let (base, commits) =
        branch_commits_since_default(&repo_root).expect("failed to collect commits");

    // Assert
    assert_eq!(base.as_deref(), Some("origin/feature"));
    assert!(commits.is_empty());
}

#[test]
fn collect_hunks_keeps_partially_staged_file_counts_separate() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    fs::create_dir_all(&repo_root).expect("failed to create repo directory");
    run_git_no_output(&repo_root, &["init"]).expect("failed to init repo");
    run_git_no_output(&repo_root, &["config", "user.email", "test@example.com"])
        .expect("failed to configure git email");
    run_git_no_output(&repo_root, &["config", "user.name", "Test User"])
        .expect("failed to configure git user");
    run_git_no_output(&repo_root, &["config", "commit.gpgsign", "false"])
        .expect("failed to disable git signing");

    let file_path = repo_root.join("example.txt");
    fs::write(&file_path, "one\ntwo\nthree\nfour\n").expect("failed to write initial file");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to add file");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit initial file");

    fs::write(&file_path, "one\nTWO staged\nthree\nfour\n").expect("failed to write staged change");
    run_git_no_output(&repo_root, &["add", "example.txt"]).expect("failed to stage change");
    fs::write(&file_path, "one\nTWO staged\nTHREE unstaged\nfour\n")
        .expect("failed to write unstaged change");

    // Act
    let hunks = collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
    let staged = hunks
        .iter()
        .filter(|hunk| hunk.file_path == "example.txt" && hunk.staged)
        .map(|hunk| diff_line_counts(&hunk.patch))
        .fold((0, 0), |sum, item| (sum.0 + item.0, sum.1 + item.1));
    let unstaged = hunks
        .iter()
        .filter(|hunk| hunk.file_path == "example.txt" && !hunk.staged)
        .map(|hunk| diff_line_counts(&hunk.patch))
        .fold((0, 0), |sum, item| (sum.0 + item.0, sum.1 + item.1));

    // Assert
    assert_eq!(staged, (1, 1));
    assert_eq!(unstaged, (1, 1));
}

#[test]
fn collect_hunks_skips_untracked_binary_files() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    fs::write(repo_root.join("note.txt"), "reviewable\ntext\n").expect("failed to write text file");
    fs::write(repo_root.join("asset.bin"), [0, 159, 146, 150, 255])
        .expect("failed to write binary file");

    // Act
    let hunks = collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");

    // Assert
    assert!(hunks.iter().any(|hunk| hunk.file_path == "note.txt"));
    assert!(!hunks.iter().any(|hunk| hunk.file_path == "asset.bin"));
}

#[test]
fn working_tree_pathspec_limits_hunks_and_status_summary() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    fs::create_dir_all(repo_root.join("src")).expect("failed to create src directory");
    fs::create_dir_all(repo_root.join("docs")).expect("failed to create docs directory");
    fs::write(repo_root.join("src/tracked.txt"), "before\n").expect("failed to write src file");
    fs::write(repo_root.join("docs/tracked.txt"), "before\n").expect("failed to write docs file");
    run_git_no_output(&repo_root, &["add", "src/tracked.txt", "docs/tracked.txt"])
        .expect("failed to add tracked files");
    run_git_no_output(&repo_root, &["commit", "-m", "initial"])
        .expect("failed to commit tracked files");

    fs::write(repo_root.join("src/tracked.txt"), "after\n").expect("failed to modify src file");
    fs::write(repo_root.join("docs/tracked.txt"), "after\n").expect("failed to modify docs file");
    fs::write(repo_root.join("src/new.txt"), "new\n").expect("failed to write src new file");
    fs::write(repo_root.join("docs/new.txt"), "new\n").expect("failed to write docs new file");

    // Act
    let hunks = collect_hunks(
        &repo_root,
        &DiffTarget {
            base: None,
            pathspec: Some("src".to_string()),
            comparison: None,
        },
    )
    .expect("failed to collect hunks");
    let summary = local_change_summary_from_status(&repo_root, Some("src"))
        .expect("failed to collect status summary");

    // Assert
    let paths = hunks
        .iter()
        .map(|hunk| hunk.file_path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"src/tracked.txt"));
    assert!(paths.contains(&"src/new.txt"));
    assert!(!paths.iter().any(|path| path.starts_with("docs/")));
    assert_eq!(summary.modified, 1);
    assert_eq!(summary.added, 1);
    assert_eq!(summary.deleted, 0);
}

#[test]
fn collect_hunks_handles_untracked_image_paths_with_non_ascii_characters() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    let image_path = "tmp-images-upload/502 chiné gris dos.webp";
    fs::create_dir_all(repo_root.join("tmp-images-upload"))
        .expect("failed to create image directory");
    fs::write(repo_root.join(image_path), b"RIFF\0\0\0\0WEBPVP8 ")
        .expect("failed to write image file");

    // Act
    let hunks = collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
    let hunk = hunks
        .iter()
        .find(|hunk| hunk.file_path == image_path)
        .expect("expected image hunk");

    // Assert
    assert_eq!(hunk.header, "Binary image added");
    assert!(
        hunk.image_diff
            .as_ref()
            .and_then(|image_diff| image_diff.after_src.as_deref())
            .is_some_and(|src| src.starts_with("data:image/webp;base64,"))
    );
}

#[test]
fn collect_hunks_includes_image_diff_for_unstaged_binary_image() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    init_test_repo(&repo_root);

    fs::write(repo_root.join("asset.png"), b"\x89PNG\r\n\x1a\n\0before")
        .expect("failed to write initial image");
    run_git_no_output(&repo_root, &["add", "asset.png"]).expect("failed to add image");
    run_git_no_output(&repo_root, &["commit", "-m", "initial image"])
        .expect("failed to commit initial image");
    fs::write(repo_root.join("asset.png"), b"\x89PNG\r\n\x1a\n\0after")
        .expect("failed to modify image");

    // Act
    let hunks = collect_hunks(&repo_root, &DiffTarget::default()).expect("failed to collect hunks");
    let hunk = hunks
        .iter()
        .find(|hunk| hunk.file_path == "asset.png")
        .expect("expected image hunk");

    // Assert
    let image_diff = hunk.image_diff.as_ref().expect("expected image diff");
    assert!(
        image_diff
            .before_src
            .as_deref()
            .is_some_and(|src| src.starts_with("data:image/png;base64,"))
    );
    assert!(
        image_diff
            .after_src
            .as_deref()
            .is_some_and(|src| src.starts_with("data:image/png;base64,"))
    );
    assert_eq!(hunk.header, "Binary image changed");
}

#[test]
fn canonicalize_repo_walks_up_to_git_root() {
    // Arrange
    let temp = TestDir::new();
    let repo_root = temp.path.join("repo");
    let nested = repo_root.join("src/components");
    fs::create_dir_all(repo_root.join(".git")).expect("failed to create fake git dir");
    fs::create_dir_all(&nested).expect("failed to create nested directory");

    // Act
    let resolved = canonicalize_repo(&nested).expect("expected repo root to resolve");

    // Assert
    assert_eq!(resolved, repo_root.canonicalize().unwrap());
}

#[test]
fn canonicalize_repo_errors_outside_git_repo() {
    // Arrange
    let temp = TestDir::new();
    let dir = temp.path.join("plain/nested");
    fs::create_dir_all(&dir).expect("failed to create plain directory");

    // Act
    let error = canonicalize_repo(&dir).expect_err("expected resolution failure");

    // Assert
    assert!(error.to_string().contains("is not inside a git repository"));
}

#[test]
fn parse_submodule_status_path_handles_plain_and_branch_lines() {
    // Arrange
    let plain_line = " 3f4a1c2 modules/libfoo";
    let branch_line = "+3f4a1c2 modules/libfoo (heads/main)";

    // Act
    let plain_path = super::parse_submodule_status_path(plain_line);
    let branch_path = super::parse_submodule_status_path(branch_line);

    // Assert
    assert_eq!(plain_path, Some("modules/libfoo"));
    assert_eq!(branch_path, Some("modules/libfoo"));
}

/// The hub lists every submodule, clean ones included, and says how many files are changed
/// in each - which is what decides the ones with a review to open.
#[test]
fn list_submodule_repos_counts_the_changes_in_every_submodule() {
    // Arrange
    let temp = TestDir::new();
    let parent = temp.path.join("parent");
    init_test_repo(&parent);
    fs::write(parent.join("README.md"), "parent\n").expect("failed to write the readme");
    run_git_no_output(&parent, &["add", "README.md"]).expect("failed to add the readme");
    run_git_no_output(&parent, &["commit", "-m", "Add the readme"]).expect("failed to commit");

    for name in ["clean", "dirty"] {
        let child = temp.path.join(name);
        init_test_repo(&child);
        fs::write(child.join("lib.rs"), "// lib\n").expect("failed to write the child file");
        run_git_no_output(&child, &["add", "lib.rs"]).expect("failed to add the child file");
        run_git_no_output(&child, &["commit", "-m", "Add lib"]).expect("failed to commit");
        run_git_no_output(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                child.to_str().expect("a utf-8 path"),
                name,
            ],
        )
        .expect("failed to add the submodule");
    }
    fs::write(parent.join("dirty/lib.rs"), "// changed\n").expect("failed to change the child");
    fs::write(parent.join("dirty/new.rs"), "// new\n").expect("failed to add a child file");

    // Act
    let submodules = list_submodule_repos(&parent).expect("failed to list the submodules");

    // Assert
    let counts: Vec<(String, usize)> = submodules
        .iter()
        .map(|submodule| {
            (
                submodule
                    .repo_path
                    .file_name()
                    .expect("a submodule directory")
                    .to_string_lossy()
                    .to_string(),
                submodule.changed_file_count,
            )
        })
        .collect();
    assert_eq!(
        counts,
        vec![("clean".to_string(), 0), ("dirty".to_string(), 2)]
    );
}
