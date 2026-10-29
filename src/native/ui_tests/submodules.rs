//! The submodule hub: every submodule of the repo, and the way into a review of a changed one.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable};

use crate::{
    git::run_git_no_output,
    native::{panes::OpenPaneRequest, theme::ThemeMode},
};

use super::{app_for, seeded_fixture};

/// A submodule of the fixture, cloned from a repo of its own beside it. Left alone it is
/// clean; `changed_files` written into it are what the hub counts.
fn add_submodule(fixture: &super::Fixture, name: &str, changed_files: &[&str]) {
    let child = fixture
        .root
        .parent()
        .expect("the fixture has an enclosing directory")
        .join(name.replace('/', "-"));
    std::fs::create_dir_all(&child).expect("failed to create the submodule repo");
    run_git_no_output(&child, &["init"]).expect("failed to init the submodule repo");
    for (key, value) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test User"),
        ("commit.gpgsign", "false"),
    ] {
        run_git_no_output(&child, &["config", key, value]).expect("failed to configure git");
    }
    std::fs::write(child.join("lib.rs"), "// lib\n").expect("failed to write the submodule file");
    run_git_no_output(&child, &["add", "-A"]).expect("failed to stage the submodule file");
    run_git_no_output(&child, &["commit", "-m", "Add lib"]).expect("failed to commit");

    run_git_no_output(
        &fixture.root,
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

    for file in changed_files {
        std::fs::write(fixture.root.join(name).join(file), "// changed\n")
            .expect("failed to change the submodule");
    }
}

/// The "N changes" labels on screen: one per repo with changed files.
fn app_changes_labels(harness: &Harness<'_>) -> Vec<String> {
    harness
        .query_all_by_label_contains(" change")
        .filter_map(|node| node.value())
        .filter(|label| label.ends_with(" changes") || label.ends_with(" change"))
        .filter(|label| label != "no changes")
        .collect()
}

#[test]
fn the_submodule_hub_lists_every_submodule_and_reviews_a_changed_one() {
    // Arrange
    let fixture = seeded_fixture("submodule-hub");
    add_submodule(&fixture, "crates/clean", &[]);
    add_submodule(&fixture, "crates/dirty", &["lib.rs", "new.rs"]);

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let listed = Arc::new(AtomicBool::new(false));
    let listed_in_ui = Arc::clone(&listed);
    // The tab titles of the panes open, read after each pass: the review of a submodule
    // arrives a session later than the click that asked for it.
    let tabs = Arc::new(Mutex::new(Vec::<String>::new()));
    let tabs_in_ui = Arc::clone(&tabs);
    // The tab in front, read after each pass: the repo's own row brings its review forward.
    let active_tab = Arc::new(Mutex::new(None::<String>));
    let active_tab_in_ui = Arc::clone(&active_tab);
    // Set to bring the hub back in front once its row has put the review there.
    let reopen = Arc::new(AtomicBool::new(false));
    let reopen_in_ui = Arc::clone(&reopen);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            // Only once the review has opened: opening it replaces the whole arrangement.
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Submodules);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            if reopen_in_ui.swap(false, Ordering::Relaxed) {
                app.open_pane(OpenPaneRequest::Submodules);
            }
            app.draw(ui);
            listed_in_ui.store(app.model.submodules.len() == 2, Ordering::Relaxed);
            *tabs_in_ui.lock().expect("the tab titles are readable") = app
                .model
                .layout
                .panes()
                .map(|(_, pane)| pane.tab_title())
                .collect();
            *active_tab_in_ui.lock().expect("the active tab is readable") = app
                .model
                .layout
                .active_pane()
                .map(|(_, pane)| pane.tab_title());
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if listed.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        listed.load(Ordering::Relaxed),
        "the hub never heard about the two submodules"
    );
    harness.run_steps(3);
    // The box takes the keyboard as the hub opens, and a blinking caret is a picture that
    // depends on which frame it was taken on. Held on, it is the same caret every run.
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
    });
    harness.run_steps(2);

    // Assert: the repo itself heads the list under its own folder name, with its changes
    // counted the way the submodules' are - adding two submodules changed it.
    let root_row = harness.get_by_label("repo").rect();
    let submodules_heading = harness.get_by_label("crates/").rect();
    assert!(
        root_row.bottom() <= submodules_heading.top(),
        "the repo's own row should sit above its submodules: {root_row:?} vs {submodules_heading:?}"
    );
    assert!(
        harness.query_by_label("no changes").is_some(),
        "the clean submodule should say it has no changes"
    );
    assert_eq!(
        app_changes_labels(&harness).len(),
        2,
        "the repo and the changed submodule should both count their changes"
    );

    // Assert: the clean submodule is listed too, and only the changed one offers a review.
    harness.get_by_label("clean");
    harness.get_by_label("dirty");

    // Assert: the box has the keyboard from the moment the hub opens, and what is typed into
    // it narrows the list to the paths that hold it - the folder in the heading included, so
    // "crates" keeps both rows while "dir" keeps only the one under it.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("dir".to_string()));
    harness.run_steps(3);
    assert!(
        harness.query_by_label("dirty").is_some(),
        "the typed query should have kept the submodule whose path holds it"
    );
    assert!(
        harness.query_by_label("clean").is_none(),
        "the typed query should have left out the submodule whose path does not hold it"
    );
    assert!(
        harness.query_by_label("repo").is_none(),
        "the typed query should have left out the repo itself, whose name does not hold it"
    );

    // Escape empties the box and the whole list comes back.
    super::press_key(&mut harness, egui::Key::Escape, egui::Modifiers::NONE);
    harness.run_steps(3);
    harness.get_by_label("clean");

    // The pointer on a row brings up its fill and border, which is what says the whole row
    // is one target. The fade takes a tenth of a second, so the picture is taken after it.
    let row = harness.get_by_label("dirty").rect().center();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(row));
    harness.run_steps(20);

    // Act: the repo's own row brings the review the window opened on back in front.
    assert_eq!(
        active_tab
            .lock()
            .expect("the active tab is readable")
            .as_deref(),
        Some("submodules"),
        "the hub is in front while it is being read"
    );
    harness.get_by_label("repo").click();
    harness.run_steps(3);
    assert_eq!(
        active_tab
            .lock()
            .expect("the active tab is readable")
            .as_deref(),
        Some("review"),
        "the repo's row should have brought its review forward"
    );
    // Back to the hub for the rest.
    reopen.store(true, Ordering::Relaxed);
    harness.run_steps(3);
    assert_eq!(
        active_tab
            .lock()
            .expect("the active tab is readable")
            .as_deref(),
        Some("submodules")
    );

    // Act: anywhere on the changed submodule's row opens a review of it.
    harness.get_by_label("dirty").click();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if tabs
            .lock()
            .expect("the tab titles are readable")
            .iter()
            .any(|title| title == "dirty")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Assert
    let tabs = tabs.lock().expect("the tab titles are readable").clone();
    assert!(
        tabs.iter().any(|title| title == "submodules"),
        "the hub tab is gone: {tabs:?}"
    );
    assert!(
        tabs.iter().any(|title| title == "dirty"),
        "the changed submodule never opened a review: {tabs:?}"
    );
}

#[test]
fn the_submodule_hub_marks_an_unpushed_submodule_and_opens_a_shell_in_it() {
    // Arrange: one submodule as it was cloned, and one with a commit its remote has not got.
    let fixture = seeded_fixture("submodule-hub-unpushed");
    add_submodule(&fixture, "crates/pushed", &[]);
    add_submodule(&fixture, "crates/ahead", &[]);
    run_git_no_output(
        &fixture.root.join("crates/ahead"),
        &[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test User",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "Not pushed",
        ],
    )
    .expect("failed to commit in the submodule");

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let listed = Arc::new(AtomicBool::new(false));
    let listed_in_ui = Arc::clone(&listed);
    let shells = Arc::new(Mutex::new(0usize));
    let shells_in_ui = Arc::clone(&shells);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .with_theme(egui::Theme::Dark)
        .wgpu()
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Submodules);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            listed_in_ui.store(app.model.submodules.len() == 2, Ordering::Relaxed);
            *shells_in_ui.lock().expect("the shell count is readable") = app
                .model
                .layout
                .panes()
                .filter(|(_, pane)| matches!(pane, crate::native::panes::Pane::Terminal { .. }))
                .count();
        });

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if listed.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        listed.load(Ordering::Relaxed),
        "the hub never heard about the two submodules"
    );
    harness.run_steps(3);
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = false;
    });
    harness.run_steps(2);

    // Assert: only the submodule ahead of its remote says so.
    let notes: Vec<String> = harness
        .query_all_by_label_contains("not pushed")
        .filter_map(|node| node.value())
        .collect();
    assert_eq!(notes, vec!["1 commit not pushed".to_string()]);
    let note = harness.get_by_label("1 commit not pushed").rect();
    let ahead = harness.get_by_label("ahead").rect();
    assert!(
        note.top() >= ahead.bottom()
            && note.top() - ahead.bottom() < 16.0
            && (note.left() - ahead.left()).abs() < 1.0,
        "the note should sit under the name of the submodule it is about: {note:?} vs {ahead:?}"
    );

    // Act: the shell button of the submodule's row opens a shell, not its review.
    harness.get_by_label("Open a shell in crates/ahead").click();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        harness.step();
        if *shells.lock().expect("the shell count is readable") > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Assert
    assert_eq!(
        *shells.lock().expect("the shell count is readable"),
        1,
        "the shell button should have opened one shell"
    );
}

/// A task that changed several repos offers them as one diff: a row over its pending reviews
/// opens a pane with every repo's hunks one under the other, so nothing has to be clicked
/// through a repo at a time.
#[test]
fn a_task_with_several_pending_reviews_opens_them_as_one_diff() {
    let fixture = seeded_fixture("task-review-all");
    add_submodule(&fixture, "crates/dirty", &["new.rs"]);
    fixture.write(
        ".moontasks/deploy-the-thing-1111/metadata.json",
        "{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        ".moontasks/deploy-the-thing-1111/request_for_review.txt",
        "crates/dirty/ // fix: the submodule\n. // chore: take the module forward\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let in_front = Arc::new(Mutex::new(None::<crate::native::panes::PaneKind>));
    let in_front_in_ui = Arc::clone(&in_front);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            *in_front_in_ui.lock().expect("poisoned") = app
                .front_pane
                .and_then(|id| app.model.layout.pane(id))
                .map(|pane| pane.kind());
        });

    let row = "review all 2 repos";
    assert!(
        wait_for_labels(&mut harness, &[row]),
        "the card should offer every pending review at once"
    );
    harness.get_by_label(row).click();
    assert!(
        super::settle(&mut harness, || *in_front.lock().expect("poisoned")
            == Some(crate::native::panes::PaneKind::TaskReview)),
        "the row should open the task's combined review"
    );
    // A file of each repo, in the one pane: the submodule's new file and the root's.
    assert!(
        wait_for_labels(&mut harness, &["new.rs", "src/extra.rs"]),
        "the combined review should show the changes of both repos"
    );
    // The submodule's one line and the root's twelve, in the header over both.
    assert!(
        wait_for_labels(&mut harness, &["+13"]),
        "the header should add up the lines of every repo"
    );
    let strip = crate::native::review::task_review::strip_id("deploy-the-thing-1111");
    assert!(
        harness.ctx.read_response(strip).is_some(),
        "the combined review should have a minimap down its scroll"
    );
}

/// A repo of the combined review is committed where it is read: its diff ends in a commit row
/// with the message its task wrote, and once committed it folds to a line offering the push.
#[test]
fn a_repo_of_the_combined_review_is_committed_under_its_diff() {
    let fixture = seeded_fixture("task-review-commit");
    add_submodule(&fixture, "crates/dirty", &["new.rs"]);
    let submodule = fixture.root.join("crates/dirty");
    run_git_no_output(&submodule, &["add", "-A"]).expect("failed to stage the submodule");
    fixture.write(
        ".moontasks/deploy-the-thing-1111/metadata.json",
        "{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        ".moontasks/deploy-the-thing-1111/request_for_review.txt",
        "crates/dirty/ // fix: the submodule\n. // chore: take the module forward\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });
    assert!(
        wait_for_labels(&mut harness, &["review all 2 repos"]),
        "the card should offer every pending review at once"
    );
    harness.get_by_label("review all 2 repos").click();
    assert!(
        wait_for_labels(&mut harness, &["fix: the submodule", "1 staged"]),
        "the submodule's commit row should carry its task's message and what is staged"
    );

    // Act: the first commit row is the submodule's, the first line of the request.
    harness
        .query_all_by_label("commit")
        .next()
        .expect("each repo has a commit row")
        .click();

    // Assert
    let committed = || {
        crate::git::run_git(&submodule, &["log", "-1", "--format=%s"])
            .is_ok_and(|subject| subject.trim() == "fix: the submodule")
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !committed() {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        committed(),
        "the submodule should have been committed with its task's message"
    );
    assert!(
        wait_for_labels(&mut harness, &["push"]),
        "the committed submodule should fold to its line, offering the push"
    );
}

/// ⌘F over the combined review searches every repo in it, counting the matches down the pane
/// in the order the repos stand, so stepping walks from one repo's diff into the next.
#[test]
fn find_over_the_combined_review_steps_from_one_repo_into_the_next() {
    let fixture = seeded_fixture("task-review-find");
    add_submodule(&fixture, "crates/dirty", &["new.rs"]);
    fixture.write("crates/dirty/new.rs", "// values\n");
    fixture.write(
        ".moontasks/deploy-the-thing-1111/metadata.json",
        "{\n  \"title\": \"Deploy the thing\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    fixture.write(
        ".moontasks/deploy-the-thing-1111/request_for_review.txt",
        "crates/dirty/ // fix: the submodule\n. // chore: take the module forward\n",
    );

    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    // How many matches the bar counts, and the repos whose review holds the current one.
    let seen = Arc::new(Mutex::new((0usize, Vec::<String>::new())));
    let seen_in_ui = Arc::clone(&seen);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(OpenPaneRequest::Tasks);
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
            let total = app.model.find.as_ref().map_or(0, |find| find.total);
            let holding: Vec<String> = app
                .model
                .review_sessions_of_repos
                .iter()
                .filter(|(_, session_id)| {
                    app.model
                        .review_ref(session_id)
                        .is_some_and(|review| review.find_match.is_some())
                })
                .map(|(repo_path, _)| repo_path.clone())
                .collect();
            *seen_in_ui.lock().expect("poisoned") = (total, holding);
        });
    assert!(
        wait_for_labels(&mut harness, &["review all 2 repos"]),
        "the card should offer every pending review at once"
    );
    harness.get_by_label("review all 2 repos").click();
    assert!(
        wait_for_labels(&mut harness, &["new.rs", "src/extra.rs"]),
        "the combined review should show the changes of both repos"
    );

    // Act
    super::press_key(&mut harness, egui::Key::F, egui::Modifiers::COMMAND);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("values".to_string()));
    harness.step();
    harness.run_steps(3);

    // Assert: the submodule's one match comes first, the line above the root's.
    let (total, holding) = seen.lock().expect("poisoned").clone();
    assert!(
        total >= 3,
        "the submodule's match and the root's should all be counted, got {total}"
    );
    assert_eq!(holding.len(), 1, "one repo holds the current match: {holding:?}");
    assert!(
        holding[0].ends_with("crates/dirty"),
        "the first match is the submodule's: {holding:?}"
    );

    super::press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    let (_, holding) = seen.lock().expect("poisoned").clone();
    assert_eq!(holding.len(), 1, "one repo holds the current match: {holding:?}");
    assert!(
        !holding[0].ends_with("crates/dirty"),
        "Enter should have stepped into the root's diff: {holding:?}"
    );
}

/// Step the window until every one of `labels` is on screen, for up to the time `settle` gives.
fn wait_for_labels(harness: &mut Harness<'_>, labels: &[&str]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        harness.step();
        if labels
            .iter()
            .all(|label| harness.query_all_by_label(label).next().is_some())
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
