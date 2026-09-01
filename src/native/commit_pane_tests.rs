//! The commit pane, drawn and driven the way the window draws it.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    commit_suggestion::CommitSuggestion,
    git::run_git_no_output,
    native::{
        panes::{Pane, PaneKind},
        theme::{Palette, ThemeMode},
        ui_tests::{app_for, harness_with_loaded_review, seeded_fixture},
    },
};

/// How long a test waits on git: a commit runs in a real pty, in a real repo.
const GIT_DEADLINE: Duration = Duration::from_secs(30);

/// The review's own button is the way in, and it opens beside the review rather than over it:
/// the point of committing here is to still see what is being committed.
#[test]
fn the_commit_button_opens_a_pane_beside_the_review() {
    let fixture = seeded_fixture("commit-button");
    let app = app_for(&fixture.root, ThemeMode::Dark);
    let mut harness = harness_with_loaded_review(app, ThemeMode::Dark);

    harness.get_by_label("[commit]").click();
    harness.run_steps(5);

    // The commit button is the pane's own, and unlike push it is there from the first frame:
    // push waits for something to have been committed.
    assert!(
        harness.query_by_label("commit").is_some(),
        "the commit pane should have opened"
    );
    assert!(
        harness.query_by_label("push").is_none(),
        "nothing has been committed, so there is nothing to push yet"
    );
}

/// The whole of it: what the review staged, committed with a message written in the pane, by
/// a real `git` in a real pty - which is what a signed commit needs.
#[test]
fn committing_from_the_pane_commits_what_is_staged() {
    let fixture = seeded_fixture("commit-run");
    run_git_no_output(&fixture.root, &["add", "-A"]).expect("failed to stage the fixture");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    // What the test does to the pane between frames: write the message, then press commit.
    let message_to_type = Arc::new(Mutex::new(None::<String>));
    let typing = Arc::clone(&message_to_type);
    let panes_open = Arc::new(Mutex::new(Vec::<PaneKind>::new()));
    let panes_in_ui = Arc::clone(&panes_open);
    let review_loaded = Arc::new(Mutex::new(false));
    let loaded_in_ui = Arc::clone(&review_loaded);
    let message_left = Arc::new(Mutex::new(None::<String>));
    let message_in_ui = Arc::clone(&message_left);
    let staged_count = Arc::new(Mutex::new(None::<usize>));
    let staged_in_ui = Arc::clone(&staged_count);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .build_ui(move |ui| {
            let session_id = app.model.root_session_id.clone();
            if let Some(message) = typing.lock().expect("expected the lock").take()
                && let Some(pane) = app.model.commit_panes.get_mut(&session_id)
            {
                pane.message = message;
            }
            app.draw(ui);
            *loaded_in_ui.lock().expect("expected the lock") = app
                .model
                .review_ref(&session_id)
                .is_some_and(|review| review.payload.is_some());
            *panes_in_ui.lock().expect("expected the lock") = app
                .model
                .layout
                .panes()
                .map(|(_, pane)| pane.kind())
                .collect();
            *message_in_ui.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .map(|pane| pane.message.clone());
            *staged_in_ui.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .and_then(|pane| pane.staged_count_for_test());
        });

    let step_until = |harness: &mut Harness<'_>, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + GIT_DEADLINE;
        while Instant::now() < deadline && !done() {
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    // The review's diff has to have arrived before its header is drawn at all.
    let review_open = {
        let loaded = Arc::clone(&review_loaded);
        move || *loaded.lock().expect("expected the lock")
    };
    step_until(&mut harness, &review_open);
    harness.run_steps(3);

    harness.get_by_label("[commit]").click();
    let commit_pane_open = {
        let panes = Arc::clone(&panes_open);
        move || panes.lock().expect("expected the lock").contains(&PaneKind::Commit)
    };
    step_until(&mut harness, &commit_pane_open);

    // The staged listing arrives from git, and the commit button is off until it has.
    let staged_listed = {
        let staged = Arc::clone(&staged_count);
        move || {
            staged
                .lock()
                .expect("expected the lock")
                .is_some_and(|count| count > 0)
        }
    };
    step_until(&mut harness, &staged_listed);
    assert!(
        staged_listed(),
        "the pane never saw what the fixture staged"
    );
    *message_to_type.lock().expect("expected the lock") = Some("Add the extra module".to_string());
    harness.run_steps(5);

    harness.get_by_label("commit").click();
    harness.run_steps(3);

    let committed = || {
        crate::git::run_git(&fixture.root, &["log", "-1", "--format=%s"])
            .is_ok_and(|subject| subject.trim() == "Add the extra module")
    };
    step_until(&mut harness, &committed);
    assert!(
        committed(),
        "the pane should have committed what the review staged, log says {:?}",
        crate::git::run_git(&fixture.root, &["log", "--oneline"])
    );

    // A message that has been used up is not left in the box to be committed twice.
    let cleared = {
        let message = Arc::clone(&message_left);
        move || {
            message
                .lock()
                .expect("expected the lock")
                .as_deref()
                .is_some_and(str::is_empty)
        }
    };
    step_until(&mut harness, &cleared);
    assert!(cleared(), "the message should have been cleared");

    // The fixture staged the whole working tree, so the commit took all of it: the review has
    // nothing left to show and closes itself, leaving the commit pane to push from.
    let review_closed = {
        let panes = Arc::clone(&panes_open);
        move || {
            let panes = panes.lock().expect("expected the lock");
            !panes.contains(&PaneKind::Review) && panes.contains(&PaneKind::Commit)
        }
    };
    // And the commit is what there is to push, so the button for it is there now.
    harness.run_steps(3);
    assert!(
        harness.query_by_label("push").is_some(),
        "the push button should have appeared once there was a commit to push"
    );

    step_until(&mut harness, &review_closed);
    assert!(
        review_closed(),
        "the review should have closed once its whole diff was committed, panes are {:?}",
        panes_open.lock().expect("expected the lock")
    );
}

/// What the pane looks like with something staged and a message written: the whole point is
/// that it reads at a glance beside the review it is committing.
#[test]
fn the_commit_pane_draws_what_it_would_commit() {
    let fixture = seeded_fixture("commit-pane-snapshot");
    run_git_no_output(&fixture.root, &["add", "-A"]).expect("failed to stage the fixture");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let typing = Arc::new(Mutex::new(Some(
        "Count the values\n\nAnd add the module that does it.".to_string(),
    )));
    let typing_in_ui = Arc::clone(&typing);
    let staged_count = Arc::new(Mutex::new(None::<usize>));
    let staged_in_ui = Arc::clone(&staged_count);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            let session_id = app.model.root_session_id.clone();
            // Written once the pane exists, and left alone after that. The written message
            // under the box is put there rather than asked for: what the agent would answer is
            // the same on every machine, and asking it is not.
            if app.model.commit_panes.contains_key(&session_id)
                && let Some(message) = typing_in_ui.lock().expect("expected the lock").take()
                && let Some(pane) = app.model.commit_panes.get_mut(&session_id)
            {
                pane.message = message;
                pane.set_suggestion_for_test(CommitSuggestion {
                    subject: "feat: count the values".to_string(),
                    paragraph: "Add the module that counts them, and call it from main."
                        .to_string(),
                });
            }
            app.draw(ui);
            *staged_in_ui.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .and_then(|pane| pane.staged_count_for_test());
        });

    let deadline = Instant::now() + GIT_DEADLINE;
    let mut opened = false;
    while Instant::now() < deadline {
        harness.step();
        if !opened && harness.query_by_label("[commit]").is_some() {
            harness.get_by_label("[commit]").click();
            opened = true;
        }
        if staged_count
            .lock()
            .expect("expected the lock")
            .is_some_and(|count| count > 0)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // The caret would blink through the snapshot otherwise.
    harness
        .ctx
        .all_styles_mut(|style| style.visuals.text_cursor.blink = false);
    harness.run_steps(5);
    harness.snapshot("commit-pane");
}

/// Pushing goes through the same pty as committing - an ssh remote asks for things too - and
/// a branch that tracks nothing gets its upstream from the push that first sends it.
#[test]
fn pushing_a_branch_that_tracks_nothing_sets_its_upstream() {
    use crate::{backend::Backend, committing::CommitAction};

    let fixture = seeded_fixture("push-upstream");
    fixture.commit("Add the rest");
    let remote = fixture
        .root
        .parent()
        .expect("expected an enclosing directory")
        .join("origin.git");
    std::fs::create_dir_all(&remote).expect("failed to make the remote");
    run_git_no_output(&remote, &["init", "--bare"]).expect("failed to init the remote");
    run_git_no_output(
        &fixture.root,
        &["remote", "add", "origin", &remote.display().to_string()],
    )
    .expect("failed to add the remote");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = crate::backend::local::LocalBackend::new(state);
    let opened = backend
        .open_session(crate::api::OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected a session");

    let before = backend
        .commit_state(&opened.session_id)
        .expect("expected a commit state");
    assert_eq!(before.upstream_ref, None, "the fixture tracks nothing yet");
    assert_eq!(before.push_ref, None, "so git has nowhere to send a plain push");

    let terminal_id = backend
        .start_commit_run(&opened.session_id, &CommitAction::Push)
        .expect("expected the push to start");

    let deadline = Instant::now() + GIT_DEADLINE;
    let mut outcome = None;
    while Instant::now() < deadline && outcome.is_none() {
        std::thread::sleep(Duration::from_millis(50));
        outcome = backend
            .commit_run_outcome(&opened.session_id, &terminal_id)
            .expect("expected an answer");
    }

    assert_eq!(outcome, Some(0), "the push should have gone through");
    let after = backend
        .commit_state(&opened.session_id)
        .expect("expected a commit state");
    assert_eq!(
        after.upstream_ref.as_deref(),
        Some("origin/main"),
        "the push should have set the upstream it sent to"
    );
    assert_eq!(after.ahead, 0, "the remote has everything this branch has");
}

/// The state `git switch -c work origin/main` leaves a branch in: it tracks the branch it was
/// started from, and git's default `push.default=simple` refuses a plain `git push` from
/// there. The pane sends the branch under its own name instead, and leaves it tracking that.
#[test]
fn pushing_a_branch_that_tracks_another_name_sends_it_under_its_own() {
    use crate::{backend::Backend, committing::CommitAction};

    let fixture = seeded_fixture("push-other-name");
    fixture.commit("Add the rest");
    let remote = fixture
        .root
        .parent()
        .expect("expected an enclosing directory")
        .join("origin.git");
    std::fs::create_dir_all(&remote).expect("failed to make the remote");
    run_git_no_output(&remote, &["init", "--bare"]).expect("failed to init the remote");
    run_git_no_output(
        &fixture.root,
        &["remote", "add", "origin", &remote.display().to_string()],
    )
    .expect("failed to add the remote");
    // Pinned in the repo, so a machine whose global config says `current` cannot make this
    // pass for the wrong reason.
    run_git_no_output(&fixture.root, &["config", "push.default", "simple"])
        .expect("failed to set push.default");
    run_git_no_output(&fixture.root, &["branch", "-M", "main"]).expect("failed to name the branch");
    run_git_no_output(&fixture.root, &["push", "-u", "origin", "main"])
        .expect("failed to push main");
    run_git_no_output(&fixture.root, &["switch", "-c", "work", "--track", "origin/main"])
        .expect("failed to branch off origin/main");
    fixture.write("src/more.rs", "pub const MORE: u32 = 1;\n");
    fixture.commit("Add more");

    let state = crate::server::build_state(Arc::new(Mutex::new(Instant::now())));
    let backend = crate::backend::local::LocalBackend::new(state);
    let opened = backend
        .open_session(crate::api::OpenSessionRequest {
            repo_path: fixture.root.display().to_string(),
            diff_target: None,
            active_commit: None,
        })
        .expect("expected a session");

    let before = backend
        .commit_state(&opened.session_id)
        .expect("expected a commit state");
    assert_eq!(before.branch_name.as_deref(), Some("work"));
    assert_eq!(
        before.upstream_ref.as_deref(),
        Some("origin/main"),
        "the branch tracks the one it was started from"
    );
    assert_eq!(
        before.push_ref, None,
        "push.default=simple has nowhere to send a plain push"
    );
    assert_eq!(before.ahead, 1, "the one commit since origin/main");

    let terminal_id = backend
        .start_commit_run(&opened.session_id, &CommitAction::Push)
        .expect("expected the push to start");

    let deadline = Instant::now() + GIT_DEADLINE;
    let mut outcome = None;
    while Instant::now() < deadline && outcome.is_none() {
        std::thread::sleep(Duration::from_millis(50));
        outcome = backend
            .commit_run_outcome(&opened.session_id, &terminal_id)
            .expect("expected an answer");
    }

    assert_eq!(outcome, Some(0), "the push should have gone through");
    let after = backend
        .commit_state(&opened.session_id)
        .expect("expected a commit state");
    assert_eq!(
        after.upstream_ref.as_deref(),
        Some("origin/work"),
        "the push should have left the branch tracking what it was sent as"
    );
    assert_eq!(
        after.push_ref.as_deref(),
        Some("origin/work"),
        "and a plain push now has somewhere to go"
    );
    assert_eq!(after.ahead, 0, "the remote has everything this branch has");
}

/// A push that worked took everything there was to send, so its button goes off rather than
/// offering to send it again - and a commit made after it turns the button back on.
#[test]
fn the_push_button_goes_off_once_the_push_has_gone_through() {
    use egui_kittest::kittest::NodeT;

    use crate::native::panes::OpenPaneRequest;

    let fixture = seeded_fixture("push-disables");
    let remote = fixture
        .root
        .parent()
        .expect("expected an enclosing directory")
        .join("origin.git");
    std::fs::create_dir_all(&remote).expect("failed to make the remote");
    run_git_no_output(&remote, &["init", "--bare"]).expect("failed to init the remote");
    run_git_no_output(
        &fixture.root,
        &["remote", "add", "origin", &remote.display().to_string()],
    )
    .expect("failed to add the remote");
    run_git_no_output(&fixture.root, &["push", "-u", "origin", "main"])
        .expect("failed to push main");
    // The commit the remote has not got, which is what the push button is there to send.
    fixture.commit("Add the rest");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    // The session opens on a worker thread, so its id arrives mid-run: the pane is opened by
    // the first frame that has one, rather than up front for a session that is not there yet.
    let mut commit_pane_opened = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .build_ui(move |ui| {
            let session_id = app.model.root_session_id.clone();
            if !commit_pane_opened && !session_id.is_empty() {
                app.open_pane(OpenPaneRequest::Commit { session_id });
                commit_pane_opened = true;
            }
            app.draw(ui);
        });

    let step_until = |harness: &mut Harness<'_>, done: &dyn Fn(&Harness<'_>) -> bool| {
        let deadline = Instant::now() + GIT_DEADLINE;
        while Instant::now() < deadline && !done(harness) {
            harness.step();
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let push_is_on = |harness: &Harness<'_>| {
        harness
            .query_by_label("push")
            .is_some_and(|button| !button.accesskit_node().is_disabled())
    };

    step_until(&mut harness, &push_is_on);
    assert!(
        push_is_on(&harness),
        "a branch ahead of its remote has something to push"
    );

    harness.get_by_label("push").click();
    // "pushed" beside the buttons is the run having worked; while it runs every button is off,
    // so the word is what says the button is off for being done rather than for being busy.
    step_until(&mut harness, &|harness| {
        harness.query_by_label("pushed").is_some()
    });
    assert!(
        harness.query_by_label("pushed").is_some(),
        "the push should have gone through"
    );
    assert!(
        harness
            .get_by_label("push")
            .accesskit_node()
            .is_disabled(),
        "everything is pushed, so the button has nothing left to send"
    );

    // A commit made after the push - here from outside the window - is something to send
    // again, and the next reading of the repo hands the button its job back.
    fixture.write("src/more.rs", "pub const MORE: u32 = 1;\n");
    fixture.commit("Add more");
    step_until(&mut harness, &push_is_on);
    assert!(
        push_is_on(&harness),
        "a commit made after the push should turn the button back on"
    );
}

/// The pull request is the last thing the pane is for: once `gh` has opened it in the browser
/// there is no button left worth pressing, so the pane goes the way pressing `close` sends it -
/// and the review beside it goes with it, rather than being left as clutter.
#[test]
fn the_commit_pane_of_a_review_is_closed_by_the_session_it_belongs_to() {
    use crate::native::panes::OpenPaneRequest;

    let fixture = seeded_fixture("commit-pane-close");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let session_id = app.model.root_session_id.clone();
    app.open_pane(OpenPaneRequest::Commit {
        session_id: session_id.clone(),
    });

    let kinds = |app: &crate::native::app::App| -> Vec<PaneKind> {
        app.model.layout.panes().map(|(_, pane)| pane.kind()).collect()
    };
    assert!(
        kinds(&app).contains(&PaneKind::Commit),
        "the commit pane should have opened, panes are {:?}",
        kinds(&app)
    );

    app.model.close_commit_pane(&session_id);

    assert!(
        !kinds(&app).contains(&PaneKind::Commit),
        "the commit pane should have closed, panes are {:?}",
        kinds(&app)
    );
}

/// Where the pane lands: down the right of the workspace, so the review it is committing stays
/// on screen - and among the tabs already down there rather than in a column of its own, which
/// would take another slice off that review for a tab that closes when the commit is made.
#[test]
fn the_commit_pane_joins_the_column_already_down_the_right() {
    use crate::native::panes::OpenPaneRequest;
    use egui_frames::{DropSide, Layout};

    let fixture = seeded_fixture("commit-pane-column");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let session_id = app.model.root_session_id.clone();
    let review = Pane::Review {
        session_id: session_id.clone(),
        title: "review".to_string(),
    };

    let frame_of_commit = |app: &crate::native::app::App| {
        app.model
            .layout
            .find_pane(|pane| pane.kind() == PaneKind::Commit)
            .and_then(|(pane, _)| app.model.layout.frame_of(pane))
    };

    // A workspace that is only the review: the pane has to make the column it stands in, since
    // joining the review's own tabs would put the review out of sight.
    app.model.layout = Layout::with_pane(review.clone());
    let review_frame = app.model.layout.active_frame();
    app.open_pane(OpenPaneRequest::Commit {
        session_id: session_id.clone(),
    });
    assert_eq!(
        app.model.layout.frame_count(),
        2,
        "the first commit pane should have made a column beside the review"
    );
    assert_ne!(
        frame_of_commit(&app),
        Some(review_frame),
        "and it should not have landed on top of the review it is committing"
    );

    // The same workspace with something already beside the review - a shell, a task, whatever
    // the user has open down there. The pane joins those tabs.
    app.model.layout = Layout::with_pane(review);
    let beside = app.model.layout.add_pane_against_edge(
        DropSide::Right,
        egui_frames::DEFAULT_EDGE_SHARE,
        Pane::Agents,
    );
    let column = app
        .model
        .layout
        .frame_of(beside)
        .expect("expected a column down the right");
    app.open_pane(OpenPaneRequest::Commit { session_id });

    assert_eq!(
        frame_of_commit(&app),
        Some(column),
        "the commit pane should have joined the column already down the right"
    );
    assert_eq!(
        app.model.layout.frame_count(),
        2,
        "and the workspace should not have been split again for it"
    );
}

/// A pane restored from a saved arrangement is a commit pane like any other.
#[test]
fn a_commit_pane_names_its_tab() {
    let pane = Pane::Commit {
        session_id: "session".to_string(),
    };

    assert_eq!(pane.kind(), PaneKind::Commit);
    assert_eq!(pane.tab_title(), "commit");
}

/// The message the agent writes shows up under the box, and `[use]` is what moves it in: the
/// box is left alone until it is pressed, and the row is done with once it has been.
#[test]
fn a_written_message_goes_in_the_box_when_use_is_pressed() {
    let fixture = seeded_fixture("commit-message-use");
    run_git_no_output(&fixture.root, &["add", "-A"]).expect("failed to stage the fixture");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    // What the agent would have answered with, put where its answer lands.
    let to_write = Arc::new(Mutex::new(None::<CommitSuggestion>));
    let writing = Arc::clone(&to_write);
    let panes_open = Arc::new(Mutex::new(Vec::<PaneKind>::new()));
    let panes_in_ui = Arc::clone(&panes_open);
    let message_left = Arc::new(Mutex::new(None::<String>));
    let message_in_ui = Arc::clone(&message_left);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 940.0))
        .build_ui(move |ui| {
            let session_id = app.model.root_session_id.clone();
            if let Some(suggestion) = writing.lock().expect("expected the lock").take()
                && let Some(pane) = app.model.commit_panes.get_mut(&session_id)
            {
                pane.set_suggestion_for_test(suggestion);
            }
            app.draw(ui);
            *panes_in_ui.lock().expect("expected the lock") = app
                .model
                .layout
                .panes()
                .map(|(_, pane)| pane.kind())
                .collect();
            *message_in_ui.lock().expect("expected the lock") = app
                .model
                .commit_panes
                .get(&session_id)
                .map(|pane| pane.message.clone());
        });

    let deadline = Instant::now() + GIT_DEADLINE;
    while Instant::now() < deadline
        && !panes_open
            .lock()
            .expect("expected the lock")
            .contains(&PaneKind::Commit)
    {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
        if harness.query_by_label("[commit]").is_some() {
            harness.get_by_label("[commit]").click();
        }
    }
    harness.run_steps(3);

    *to_write.lock().expect("expected the lock") = Some(CommitSuggestion {
        subject: "feat: add the extra module".to_string(),
        paragraph: "The fixture gains a module the review is of.".to_string(),
    });
    harness.run_steps(3);

    assert_eq!(
        message_left.lock().expect("expected the lock").as_deref(),
        Some(""),
        "a written message stays out of the box until it is used"
    );

    harness.get_by_label("use").click();
    harness.run_steps(3);

    assert_eq!(
        message_left.lock().expect("expected the lock").as_deref(),
        Some("feat: add the extra module\n\nThe fixture gains a module the review is of."),
        "pressing use should put the whole written message in the box"
    );
    assert!(
        harness.query_by_label("use").is_none(),
        "the message is in the box now, so there is nothing left to use"
    );
}

/// The row with no message to show - on a machine without `opencode`, or before one has been
/// written - is not a row at all: nothing to read and nothing to press.
#[test]
fn nothing_is_drawn_where_there_is_no_message_to_show() {
    let mut harness = Harness::new_ui(|ui| {
        crate::native::commit_pane::draw_suggested_message(
            ui,
            &Palette::of(ThemeMode::Dark),
            None,
            None,
            false,
        );
    });
    harness.run();

    assert!(harness.query_by_label("use").is_none());
}

/// A message that would not come is said once, in its own words, with nothing to press: the
/// commit is written in the box either way.
#[test]
fn a_message_that_would_not_come_says_why() {
    let mut harness = Harness::new_ui(|ui| {
        crate::native::commit_pane::draw_suggested_message(
            ui,
            &Palette::of(ThemeMode::Dark),
            None,
            Some("opencode failed (status 1): Error: UnknownError"),
            false,
        );
    });
    harness.run();

    assert!(
        harness
            .query_by_label("opencode failed (status 1): Error: UnknownError")
            .is_some(),
        "the pane should say why no message came"
    );
    assert!(
        harness.query_by_label("use").is_none(),
        "there is no message to use"
    );
}

