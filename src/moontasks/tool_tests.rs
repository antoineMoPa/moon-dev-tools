//! The tools, driven the way a hook script drives them.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use serde_json::{Value, json};

use super::tools::{self, Board};
use crate::{api::AppState, git::run_git_no_output, moontasks::store};

/// A repo with one commit and one card on its board, and a session open on it.
///
/// Shared with the hook tests next door, which need the same board and drive it through Rhai
/// rather than through [`tools::call`].
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) state: AppState,
    pub(crate) session_id: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        // The worktrees a test made live outside the repo, under the per-test root.
        if let Some(worktrees) = crate::settings::worktrees_root() {
            let _ = fs::remove_dir_all(worktrees);
        }
    }
}

impl Fixture {
    pub(crate) fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "moonreview-tools-{}-{name}-{}",
            std::process::id(),
            store::new_uuid()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("expected the fixture directory");
        run_git_no_output(&root, &["init"]).expect("expected a repo");
        for (key, value) in [
            ("user.email", "test@example.com"),
            ("user.name", "Test User"),
            ("commit.gpgsign", "false"),
        ] {
            run_git_no_output(&root, &["config", key, value]).expect("expected git configured");
        }
        fs::write(root.join("a.txt"), "one\n").expect("expected the fixture file");
        run_git_no_output(&root, &["add", "-A"]).expect("expected the fixture staged");
        run_git_no_output(&root, &["commit", "-m", "first"])
            .expect("expected the fixture committed");

        // Written by hand, the way the board's own docs say a task may be: this is also the
        // only way to make one, since no tool creates a task.
        let task_dir = root.join(".moontasks").join("fix-the-thing-1111");
        fs::create_dir_all(&task_dir).expect("expected a task folder");
        fs::write(
            task_dir.join("metadata.json"),
            "{ \"title\": \"Fix the thing\", \"status\": \"todo\", \"created_at_unix\": 1700000000, \
             \"resources\": [] }\n",
        )
        .expect("expected the task written");

        let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
        let opened = crate::service::open_session(
            &state,
            crate::api::OpenSessionRequest {
                repo_path: root.display().to_string(),
                diff_target: None,
                active_commit: None,
            },
        )
        .expect("expected a session");

        Self {
            root,
            state,
            session_id: opened.session_id,
        }
    }

    fn board(&self) -> Board<'_> {
        Board {
            state: &self.state,
            session_id: self.session_id.clone(),
        }
    }

    /// Call a tool by name, and say whether it refused.
    pub(crate) fn call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        tools::call(&self.board(), name, &arguments).map_err(|error| format!("{error}"))
    }

    pub(crate) fn task(&self) -> store::TaskMetadata {
        store::read_task(&self.root, TASK).expect("expected the task")
    }

    pub(crate) fn worktree_path(&self) -> PathBuf {
        PathBuf::from(&self.task().worktree.expect("expected a worktree").path)
    }

    pub(crate) fn run_git(&self, path: &PathBuf, args: &[&str]) {
        run_git_no_output(path, args).expect("expected git to succeed");
    }
}

pub(crate) const TASK: &str = "fix-the-thing-1111";

/// Cards come from the person. A script that could make work could make work for itself.
#[test]
fn no_tool_creates_a_task() {
    let names: Vec<&str> = tools::TOOLS.iter().map(|tool| tool.name).collect();

    assert!(!names.is_empty());
    assert!(
        !names.iter().any(|name| name.contains("create_task")),
        "no tool may create a card, got {names:?}"
    );
}

/// Reviewing checks a card's branch out in the repo itself, over whatever the person has open.
/// That is a thing a person asks for, so nothing unattended can reach it.
#[test]
fn no_tool_starts_a_review() {
    let names: Vec<&str> = tools::TOOLS.iter().map(|tool| tool.name).collect();

    assert!(
        !names.iter().any(|name| name.contains("review")),
        "reviewing stays a button, got {names:?}"
    );
}

/// Every tool says what it is for and what its arguments hold, because that is the whole of
/// the reference a person writing a hook reads.
#[test]
fn every_tool_says_what_it_is_for_and_what_it_takes() {
    for tool in tools::TOOLS {
        assert!(
            tool.description.len() > 40,
            "{} needs a description worth reading",
            tool.name
        );
        assert!(
            !tool.answers.is_empty(),
            "{} has to say what it answers with",
            tool.name
        );
        for param in tool.params {
            assert!(
                !param.about.is_empty(),
                "{}'s {} has to say what it holds",
                tool.name,
                param.name
            );
        }
    }
}

/// Arguments are passed by position, so a call can only leave off what is at the end. An
/// optional argument in front of a required one would be a tool nobody can call correctly.
#[test]
fn every_optional_argument_comes_after_every_required_one() {
    for tool in tools::TOOLS {
        let first_optional = tool.params.iter().position(|param| param.optional);
        let last_required = tool.params.iter().rposition(|param| !param.optional);
        if let (Some(first_optional), Some(last_required)) = (first_optional, last_required) {
            assert!(
                first_optional > last_required,
                "{}'s arguments are out of order",
                tool.name
            );
        }
    }
}

/// The tag that decides which cards are picked up is not one a script may write — neither onto
/// a card nor off one it has finished with.
#[test]
fn the_autopilot_tag_is_refused_both_ways() {
    let fixture = Fixture::new("autopilot-tag");

    for spelling in ["autopilot", "Autopilot", "  AUTOPILOT  "] {
        let refusal = fixture
            .call("add_task_tag", json!({ "task_id": TASK, "tag": spelling }))
            .expect_err("adding the autopilot tag must be refused");
        assert!(refusal.contains("the person's"), "got {refusal}");
    }

    // And off a card that really has it, which is the case that matters: a script that could
    // clear the tag could drop a card off the board without anybody deciding to.
    let mut metadata = fixture.task();
    metadata.tags = vec!["autopilot".to_string()];
    store::write_task(&fixture.root, TASK, &metadata).expect("expected the tag written");

    let refusal = fixture
        .call(
            "remove_task_tag",
            json!({ "task_id": TASK, "tag": "autopilot" }),
        )
        .expect_err("removing the autopilot tag must be refused");
    assert!(refusal.contains("the person's"), "got {refusal}");
    assert_eq!(
        fixture.task().tags,
        ["autopilot"],
        "the card must still be marked"
    );
}

/// Any other tag is ordinary, so the refusal is about that one tag rather than about tagging.
#[test]
fn any_other_tag_goes_on_and_comes_off() {
    let fixture = Fixture::new("ordinary-tag");

    let tags = fixture
        .call(
            "add_task_tag",
            json!({ "task_id": TASK, "tag": "Needs Tests" }),
        )
        .expect("expected the tag to go on");
    assert_eq!(tags, json!(["needs-tests"]));
    assert_eq!(fixture.task().tags, ["needs-tests"]);

    let tags = fixture
        .call(
            "remove_task_tag",
            json!({ "task_id": TASK, "tag": "needs-tests" }),
        )
        .expect("expected the tag to come off");
    assert_eq!(tags, json!([]));
    assert!(fixture.task().tags.is_empty());
}

/// Taking a tag off a card that never had it is how a script clears a mark it may not have
/// left, so it is an ordinary answer rather than a refusal.
#[test]
fn removing_a_tag_a_card_never_had_is_not_a_refusal() {
    let fixture = Fixture::new("absent-tag");

    fixture
        .call(
            "remove_task_tag",
            json!({ "task_id": TASK, "tag": "stuck" }),
        )
        .expect("expected clearing an absent tag to be allowed");
}

/// Notes are how a hook leaves an account of what happened, so reading and adding to them has
/// to be about the card rather than about a file that may not be there yet.
#[test]
fn notes_are_read_and_added_to() {
    let fixture = Fixture::new("notes");

    let empty = fixture
        .call("read_task_notes", json!({ "task_id": TASK }))
        .expect("expected an answer");
    assert_eq!(empty, json!(""));

    for line in ["The run could not build it.", "Left for a person."] {
        fixture
            .call(
                "append_task_notes",
                json!({ "task_id": TASK, "text": line }),
            )
            .expect("expected the note written");
    }

    let notes = fixture
        .call("read_task_notes", json!({ "task_id": TASK }))
        .expect("expected the notes");
    // A blank line between entries, because notes are read as markdown on the card.
    assert_eq!(
        notes,
        json!("The run could not build it.\n\nLeft for a person.\n")
    );
}

/// A task id that names nothing is said plainly rather than answered with emptiness, which
/// would read as a card with nothing in it.
#[test]
fn a_card_that_is_not_there_is_said_to_be_missing() {
    let fixture = Fixture::new("missing-card");

    let refusal = fixture
        .call("read_task_notes", json!({ "task_id": "no-such-card" }))
        .expect_err("expected a refusal");
    assert!(!refusal.is_empty());
}

/// A card's checkout is what lets a run work without touching what the person has open.
#[test]
fn a_checkout_is_made_on_the_cards_own_branch_and_given_back() {
    let fixture = Fixture::new("worktree");

    let made = fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect("expected a worktree");
    assert_eq!(made["branch"], json!("moontask/fix-the-thing-1111"));
    let path = fixture.worktree_path();
    assert!(path.join("a.txt").exists(), "expected a working copy");

    fixture
        .call("discard_worktree", json!({ "task_id": TASK }))
        .expect("expected the checkout given back");
    assert!(fixture.task().worktree.is_none());
    assert!(!path.exists());
}

/// A run started as a conversation keeps no transcript, and saying so is more use than an
/// empty answer that reads as a run which printed nothing.
#[test]
fn reading_a_run_that_kept_no_transcript_says_so() {
    let fixture = Fixture::new("no-transcript");

    let refusal = fixture
        .call(
            "read_task_run",
            json!({ "task_id": TASK, "run_id": "nope" }),
        )
        .expect_err("expected a refusal");
    assert!(refusal.contains("nothing was recorded"), "got {refusal}");
}

/// Reviewing puts a card's branch in the repo, so it refuses anything that would lose work —
/// and each refusal leaves the world exactly as it found it.
///
/// Driven through [`crate::moontasks::worktrees`] rather than a tool, because there is no tool:
/// the card's `[start review]` button is the only way in.
#[test]
fn reviewing_refuses_rather_than_moving_work_out_from_under_anyone() {
    let fixture = Fixture::new("review-refusals");
    fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect("expected a worktree");
    let worktree = fixture.worktree_path();
    let on_branch_before = crate::git::current_branch_name(&fixture.root)
        .expect("expected a branch")
        .expect("expected a named branch");

    // Nothing committed on the branch: reviewing would show an empty diff and cost the card
    // its checkout on the way.
    let refusal = fixture
        .review()
        .expect_err("an empty branch must be refused");
    assert!(refusal.contains("nothing on"), "got {refusal}");

    // Work in the checkout but not committed: there would be nothing on the branch to move.
    fs::write(worktree.join("a.txt"), "worked on\n").expect("expected the run's work");
    let refusal = fixture
        .review()
        .expect_err("an uncommitted worktree must be refused");
    assert!(refusal.contains("commit the work"), "got {refusal}");

    fixture.run_git(&worktree, &["add", "-A"]);
    fixture.run_git(&worktree, &["commit", "-m", "did the work"]);

    // The repo itself has local changes: switching branches under them is the one thing that
    // must never happen.
    fs::write(fixture.root.join("a.txt"), "mine\n").expect("expected local changes");
    let refusal = fixture.review().expect_err("a dirty repo must be refused");
    assert!(refusal.contains("local changes"), "got {refusal}");

    assert_eq!(
        crate::git::current_branch_name(&fixture.root).expect("expected a branch"),
        Some(on_branch_before.clone()),
        "a refusal must leave the repo where it was"
    );
    assert!(
        fixture.task().worktree.is_some(),
        "a refusal must leave the card its checkout"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("a.txt")).expect("expected the file"),
        "mine\n",
        "a refusal must leave the person's own work alone"
    );
}

/// And when nothing is in the way, the work arrives in the repo where it can be built and QA'd.
#[test]
fn reviewing_brings_the_work_into_the_repo_and_gives_the_checkout_back() {
    let fixture = Fixture::new("review-succeeds");
    fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect("expected a worktree");
    let worktree = fixture.worktree_path();
    let branch = fixture.task().worktree.expect("expected a worktree").branch;

    fs::write(worktree.join("a.txt"), "worked on\n").expect("expected the run's work");
    fixture.run_git(&worktree, &["add", "-A"]);
    fixture.run_git(&worktree, &["commit", "-m", "did the work"]);

    fixture
        .review()
        .expect("expected the review to be prepared");

    assert_eq!(
        crate::git::current_branch_name(&fixture.root).expect("expected a branch"),
        Some(branch),
        "the repo should be on the card's branch"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("a.txt")).expect("expected the file"),
        "worked on\n",
        "the work should be in the repo"
    );
    assert!(
        fixture.task().worktree.is_none() && !worktree.exists(),
        "the checkout has done its job and should be gone"
    );
}

/// A card sent back for more work after a review gets another checkout on the branch it
/// already has — but not while the repo is still sitting on that branch, because git will not
/// have one branch in two working trees and the repo is one.
#[test]
fn a_card_sent_back_gets_another_checkout_once_the_repo_is_off_its_branch() {
    let fixture = Fixture::new("second-checkout");
    fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect("expected a worktree");
    let worktree = fixture.worktree_path();
    fs::write(worktree.join("a.txt"), "worked on\n").expect("expected the run's work");
    fixture.run_git(&worktree, &["add", "-A"]);
    fixture.run_git(&worktree, &["commit", "-m", "did the work"]);
    fixture
        .review()
        .expect("expected the review to be prepared");

    // The review left the repo on the card's branch, which is the whole point of it.
    let refusal = fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect_err("the branch is in the repo, so it cannot also be in a checkout");
    assert!(
        refusal.contains("switch it to another branch"),
        "got {refusal}"
    );

    // Once the person has moved off it — which is what finishing a review looks like — the
    // card can be worked again, and the second checkout carries what was already done.
    fixture.run_git(&fixture.root, &["checkout", "-"]);
    fixture
        .call("create_worktree", json!({ "task_id": TASK }))
        .expect("a card sent back needs another checkout");

    assert_eq!(
        fs::read_to_string(fixture.worktree_path().join("a.txt")).expect("expected the file"),
        "worked on\n",
        "the second checkout should carry the work already done"
    );
}

impl Fixture {
    /// What the card's `[start review]` button does.
    fn review(&self) -> Result<crate::moontasks::TaskReviewPayload, String> {
        crate::moontasks::worktrees::review(&self.state, &self.session_id, TASK)
            .map_err(|error| format!("{error}"))
    }
}
