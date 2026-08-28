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
            app.draw(ui);
            listed_in_ui.store(app.model.submodules.len() == 2, Ordering::Relaxed);
            *tabs_in_ui.lock().expect("the tab titles are readable") = app
                .model
                .layout
                .panes()
                .map(|(_, pane)| pane.tab_title())
                .collect();
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
    harness.snapshot("submodule-hub");

    // Assert: the clean submodule is listed too, and only the changed one offers a review.
    harness.get_by_label("crates/");
    harness.get_by_label("clean");
    harness.get_by_label("dirty");
    harness.get_by_label("no changes");

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
    harness.snapshot("submodule-hub-hovered");

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
