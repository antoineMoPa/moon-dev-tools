//! The commit a board task wrote for a repo, put in the commit pane's box - drawn and driven the
//! way the window draws it.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    git::run_git_no_output,
    native::{
        commit_pane_tests::GIT_DEADLINE,
        panes::PaneKind,
        theme::ThemeMode,
        ui_tests::{Fixture, app_for, seeded_fixture},
    },
};

/// What a test watching the commit pane fill itself in reads, a frame at a time.
///
/// The window is drawn in a closure the harness owns, so what a test wants to assert on has to
/// be copied out of the app while it is in there. Every test below wants the same three things,
/// and has to wait on them in the same order - the pane opens, the board
/// poll answers, and only then is an empty box worth anything.
struct Watched {
    /// The panes that are open, which is how the test knows the commit pane arrived.
    panes: Arc<Mutex<Vec<PaneKind>>>,
    /// What is in the commit box, which is what these tests are about.
    message: Arc<Mutex<Option<String>>>,
    /// How many lines the board read out of the tasks' files. Zero until the poll answers.
    requests: Arc<Mutex<usize>>,
    /// What the pane found staged, once it has read the repo.
    staged: Arc<Mutex<Option<usize>>>,
}

impl Watched {
    fn panes(&self) -> Vec<PaneKind> {
        self.panes.lock().expect("expected the lock").clone()
    }

    fn message(&self) -> Option<String> {
        self.message.lock().expect("expected the lock").clone()
    }

    fn requests(&self) -> usize {
        *self.requests.lock().expect("expected the lock")
    }

    fn staged(&self) -> Option<usize> {
        *self.staged.lock().expect("expected the lock")
    }
}

/// Write a task on the fixture's board, in that column, whose `request_for_review.txt` is
/// those lines.
fn a_task_asking_for_review(fixture: &Fixture, task: &str, column: &str, lines: &str) {
    fixture.write(
        &format!(".moontasks/{task}/metadata.json"),
        &format!(
            "{{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"{column}\",\n  \
             \"created_at_unix\": 1700000000,\n  \"resources\": []\n}}\n"
        ),
    );
    fixture.write(&format!(".moontasks/{task}/request_for_review.txt"), lines);
}

/// The column a card being worked on is in, which is where these tasks are unless a test is
/// about a card that has been finished.
const IN_HAND: &str = "in_progress";

/// Open the fixture's review, press its commit button, and wait for the pane and for the board's
/// reading of the tasks' files - which is everything the message in the box depends on.
fn commit_pane_over_the_board(fixture: &Fixture) -> (Harness<'static>, Watched) {
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let watched = Watched {
        panes: Arc::new(Mutex::new(Vec::new())),
        message: Arc::new(Mutex::new(None)),
        requests: Arc::new(Mutex::new(0)),
        staged: Arc::new(Mutex::new(None)),
    };
    let panes = Arc::clone(&watched.panes);
    let message = Arc::clone(&watched.message);
    let requests = Arc::clone(&watched.requests);
    let staged = Arc::clone(&watched.staged);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            let session_id = app.model.root_session_id.clone();
            app.draw(ui);
            *requests.lock().expect("expected the lock") = app.model.review_requests.len();
            *staged.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .and_then(|pane| pane.staged_count_for_test());
            *panes.lock().expect("expected the lock") = app
                .model
                .layout
                .panes()
                .map(|(_, pane)| pane.kind())
                .collect();
            *message.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .map(|pane| pane.message.clone());
        });

    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline && !watched.panes().contains(&PaneKind::Commit) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
        if harness.query_by_label("[commit]").is_some() {
            harness.get_by_label("[commit]").click();
        }
    }

    // The poll that reads the board's requests is a worker thread, so a line lands a few frames
    // after the pane does.
    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline && watched.requests() == 0 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        watched.requests() > 0,
        "the board should have read the task's file"
    );

    (harness, watched)
}

/// Wait for the pane to have read the repo, which is what tells it which branch it is on - and
/// so the last thing the message in the box waits on. An empty box before this proves nothing.
fn wait_for_the_repo_to_be_read(harness: &mut Harness<'static>, watched: &Watched) {
    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline && watched.staged().is_none() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        watched.staged().is_some(),
        "the pane should have read the repo"
    );
}

/// What the fixture's task writes, and what the box should end up holding for it.
const REQUESTED_LINE: &str =
    ".#ship-it // feat: add the extra module\n  The fixture gains a module the review is of.\n";
const REQUESTED_MESSAGE: &str =
    "feat: add the extra module\n\nThe fixture gains a module the review is of.";

/// A commit an agent already wrote is the one the pane opens with.
///
/// When a task's `request_for_review.txt` names this repo and the branch it is on and says what
/// to commit there, that message is in the box the moment the pane opens - no `opencode`, no run,
/// nothing to press, and nothing staged yet either. It does not come from the diff, so it does
/// not wait on one: what is about to be committed is readable while the hunks are still being
/// picked next door.
#[test]
fn a_commit_a_task_asked_for_is_in_the_box_before_anything_is_staged() {
    let fixture = seeded_fixture("commit-message-requested");
    fixture.checkout_branch("ship-it");
    a_task_asking_for_review(&fixture, "deploy-the-thing-1111", IN_HAND, REQUESTED_LINE);

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);

    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline && watched.message().as_deref() != Some(REQUESTED_MESSAGE) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        watched.message().as_deref(),
        Some(REQUESTED_MESSAGE),
        "the commit the task asked for should be in the box with nothing staged"
    );
    assert!(
        harness.query_by_label("use").is_none(),
        "the message is in the box, so there is nothing to press to put it there"
    );
    // The pane's reading of the repo lands on its own clock, after the message did - so it is
    // waited for before it is asked what was staged.
    wait_for_the_repo_to_be_read(&mut harness, &watched);
    assert_eq!(
        watched.staged(),
        Some(0),
        "and it got there without a thing being staged"
    );

    // The header says the repo is on the branch the line asked for, so there is no pill saying
    // the commit belongs somewhere else.
    harness.snapshot("commit-pane-requested");
}

/// A line about a branch this repo is not on keeps its message to itself.
///
/// A line names the branch its commit belongs on. When that branch is checked out nowhere - the
/// usual reason being that it was merged and everyone moved off it - the line resolves back onto
/// the main checkout, and the next piece of work written there would open its commit pane on
/// someone else's finished message. The box stays empty, and the header says whose message it
/// was and why it is not there.
#[test]
fn a_commit_asked_for_on_another_branch_stays_out_of_the_box() {
    let fixture = seeded_fixture("commit-message-other-branch");
    fixture.checkout_branch("the-next-thing");
    a_task_asking_for_review(&fixture, "deploy-the-thing-1111", IN_HAND, REQUESTED_LINE);

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);
    wait_for_the_repo_to_be_read(&mut harness, &watched);

    assert_eq!(
        watched.message().as_deref(),
        Some(""),
        "a commit written for another branch should not be put in this one's box"
    );

    // The header carries the `asked for ship-it` pill, which is what says whose message this was
    // and why it is not in the box.
    harness.snapshot("commit-pane-asked-elsewhere");
}

/// The commit that lands in the box is the one written for the branch the repo is on.
///
/// Several tasks name the same repo, and a line naming a branch that is checked out nowhere
/// resolves to that repo all the same - so every one of them points at this one working copy.
/// The branch is what tells them apart: the pane is committing the work of the task whose branch
/// is out, and it is that task's message that belongs in the box, wherever its line sits among
/// the others - here first, which is where the repo alone would have found it.
#[test]
fn the_commit_in_the_box_is_the_one_written_for_the_branch_that_is_out() {
    let fixture = seeded_fixture("commit-message-two-lines");
    fixture.checkout_branch("ship-it");
    a_task_asking_for_review(
        &fixture,
        "aaa-earlier-task-1111",
        IN_HAND,
        ".#some-older-branch // feat: something else entirely\n  Finished a fortnight ago.\n",
    );
    a_task_asking_for_review(&fixture, "zzz-later-task-2222", IN_HAND, REQUESTED_LINE);

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);

    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline && watched.message().as_deref() != Some(REQUESTED_MESSAGE) {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        watched.message().as_deref(),
        Some(REQUESTED_MESSAGE),
        "the line for the branch the repo is on should be the one in the box"
    );
}

/// A line crossed off is finished with, and its message is not offered again.
///
/// Crossing off is how work that is committed and pushed stops asking to be looked at. The repo
/// going dirty again is the next piece of work, not that one coming back.
#[test]
fn a_line_crossed_off_keeps_its_message_out_of_the_box() {
    let fixture = seeded_fixture("commit-message-crossed-off");
    fixture.checkout_branch("ship-it");
    a_task_asking_for_review(
        &fixture,
        "deploy-the-thing-1111",
        IN_HAND,
        &format!("x {REQUESTED_LINE}"),
    );

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);
    wait_for_the_repo_to_be_read(&mut harness, &watched);

    assert_eq!(
        watched.message().as_deref(),
        Some(""),
        "a line crossed off should not put its message in the box"
    );
}

/// A card moved to the column that finishes a task stops offering what it asked for.
///
/// The line is still in the file, on the branch this repo is on, and would fill the box from any
/// other column - so what is being read here is where the card sits and nothing else.
#[test]
fn a_task_that_has_been_finished_keeps_its_message_out_of_the_box() {
    let fixture = seeded_fixture("commit-message-task-finished");
    fixture.checkout_branch("ship-it");
    a_task_asking_for_review(
        &fixture,
        "deploy-the-thing-1111",
        crate::moontasks::store::CLOSES_REVIEWS_IN,
        REQUESTED_LINE,
    );

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);
    wait_for_the_repo_to_be_read(&mut harness, &watched);

    assert_eq!(
        watched.message().as_deref(),
        Some(""),
        "a finished task should not put its message in the box"
    );
}

/// A repo gets one commit after another through the same pane, and each line asking for one
/// fills the box in its turn.
///
/// The pane is kept for as long as the window is open - the session it belongs to is the repo -
/// so the first commit it ever filled in is not the last. Once that one is made, its line is
/// still in the file and still names a branch, and it does not come back into the emptied box.
/// The next piece of work in the repo, on its own branch with its own line, is a different
/// commit, and its message goes in the box the way the first one's did.
#[test]
fn the_next_commit_a_task_asked_for_fills_the_box_once_the_last_one_is_made() {
    let fixture = seeded_fixture("commit-message-next-request");
    fixture.checkout_branch("ship-it");
    run_git_no_output(&fixture.root, &["add", "-A"]).expect("failed to stage the fixture");
    a_task_asking_for_review(&fixture, "deploy-the-thing-1111", IN_HAND, REQUESTED_LINE);

    let (mut harness, watched) = commit_pane_over_the_board(&fixture);
    let step_until = |harness: &mut Harness<'static>, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + GIT_DEADLINE;
        while Instant::now() < deadline && !done() {
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    // The commit button is off until the pane has seen what is staged.
    step_until(&mut harness, &|| {
        watched.message().as_deref() == Some(REQUESTED_MESSAGE)
            && watched.staged().is_some_and(|count| count > 0)
    });
    assert_eq!(watched.message().as_deref(), Some(REQUESTED_MESSAGE));

    harness.get_by_label("commit").click();
    let committed = || {
        crate::git::run_git(&fixture.root, &["log", "-1", "--format=%s"])
            .is_ok_and(|subject| subject.trim() == "feat: add the extra module")
    };
    step_until(&mut harness, &|| {
        committed() && watched.message().as_deref() == Some("")
    });
    assert!(committed(), "the requested commit should have been made");
    assert_eq!(
        watched.message().as_deref(),
        Some(""),
        "and the box emptied"
    );

    // The line just committed is still there, on the branch still out: it is not offered again.
    wait_for_the_repo_to_be_read(&mut harness, &watched);
    harness.run_steps(10);
    assert_eq!(
        watched.message().as_deref(),
        Some(""),
        "the commit just made should not be put back in the box"
    );

    // The next piece of work: its own branch, its own task, its own line.
    fixture.checkout_branch("the-next-thing");
    fixture.write("src/next.rs", "pub fn next() {}\n");
    a_task_asking_for_review(
        &fixture,
        "the-next-thing-2222",
        IN_HAND,
        ".#the-next-thing // feat: the next thing\n  Its own line, on its own branch.\n",
    );

    step_until(&mut harness, &|| {
        watched.message().as_deref()
            == Some("feat: the next thing\n\nIts own line, on its own branch.")
    });
    assert_eq!(
        watched.message().as_deref(),
        Some("feat: the next thing\n\nIts own line, on its own branch."),
        "the next commit a task asked for should be in the box"
    );
}
