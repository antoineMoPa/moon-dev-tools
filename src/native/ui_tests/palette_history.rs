//! The palette's history above it: ↑ from the first row goes into it, and Enter runs an entry
//! again - a quick task's bringing its agent's shell to the front.

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;

use crate::{
    api::AgentKind,
    moontasks::{ColumnId, TaskResourceKind, TaskResourceView, TaskView},
    native::{palette::CommandAction, panes::OpenPaneRequest, theme::ThemeMode},
    palette_history::{HistoryEntry, HistoryThing},
};

use super::{app_for, palette_sections::offer_agents, press_key, seeded_fixture};

fn entry(thing: HistoryThing, minutes_ago: u64) -> HistoryEntry {
    HistoryEntry {
        thing,
        ran_at_unix: crate::native::messages::now_unix() - minutes_ago * 60,
    }
}

/// The card of a quick task, with its agent running in `terminal-1`.
fn quick_task_card() -> TaskView {
    TaskView {
        id: "task-1".to_string(),
        title: "uber palette".to_string(),
        status: ColumnId::new("in_progress"),
        created_at_unix: 0,
        entered_column_at_unix: None,
        dir_path: String::new(),
        repo_path: "/repo".to_string(),
        tags: Vec::new(),
        remote_task_tracker_url: String::new(),
        parent: None,
        notes: String::new(),
        attachments: Vec::new(),
        resources: vec![TaskResourceView {
            id: "run".to_string(),
            kind: TaskResourceKind::Agent,
            agent: AgentKind::Claude,
            label: "claude - 1".to_string(),
            file_path: None,
            terminal_id: Some("terminal-1".to_string()),
            running: true,
            going_elsewhere_in: None,
            quiet_for_secs: None,
            attention: None,
            resumable: true,
            started_at_unix: 0,
            started_by: None,
            work_tree: None,
        }],
    }
}

/// ↑ on the first row puts the keyboard in the history on its newest entry, ↑ again goes one
/// older, and Enter there - on a quick task - brings its agent's shell to the front rather
/// than starting another.
#[test]
fn up_from_the_first_row_runs_an_earlier_quick_task_again() {
    let fixture = seeded_fixture("palette-history");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    offer_agents(
        &mut app,
        &[AgentKind::Claude, AgentKind::Codex],
        AgentKind::Claude,
    );
    let root = app.model.root_session_id.clone();
    // Oldest first.
    app.model.palette.history.seed(
        vec![
            entry(
                HistoryThing::File {
                    repo_path: "/repo".to_string(),
                    file_path: "src/settings.rs".to_string(),
                },
                4 * 24 * 60,
            ),
            entry(
                HistoryThing::Line {
                    repo_path: "/repo".to_string(),
                    file_path: "src/api.rs".to_string(),
                    line_number: 179,
                    text: "pub(crate) enum AgentKind {".to_string(),
                },
                120,
            ),
            entry(
                HistoryThing::Command {
                    action: "OpenWorkLog".to_string(),
                    title: "wl".to_string(),
                    description: "Open work log".to_string(),
                },
                60,
            ),
            entry(
                HistoryThing::Shell {
                    line: "git status".to_string(),
                },
                40,
            ),
            entry(
                HistoryThing::QuickTask {
                    task_id: "task-1".to_string(),
                    title: "uber palette".to_string(),
                    agent: AgentKind::Claude,
                },
                20,
            ),
            entry(
                HistoryThing::File {
                    repo_path: "/repo".to_string(),
                    file_path: "src/native/palette/sections.rs".to_string(),
                },
                12,
            ),
        ],
        vec![quick_task_card()],
    );
    let seen = Arc::new(Mutex::new((false, None::<String>)));
    let seen_in_ui = Arc::clone(&seen);
    let mut frame = 0;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .wgpu()
        .build_ui(move |ui| {
            if frame == 0 {
                app.model.palette.show_commands(root.clone());
            }
            frame += 1;
            // The window's ground, which the faint history is seen against.
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, app.palette_of().bg);
            crate::native::palette::draw(&mut app, ui.ctx());
            let mut seen = seen_in_ui.lock().expect("poisoned");
            seen.0 = app.model.palette.history.in_use();
            match app.pending_action.take() {
                Some(CommandAction::OpenPane(OpenPaneRequest::AttachTerminal {
                    terminal_id,
                    ..
                })) => seen.1 = Some(terminal_id),
                Some(other) => panic!("Enter should have opened the agent's shell, not {other:?}"),
                None => {}
            }
        });

    harness.run_steps(2);
    assert!(
        !seen.lock().expect("poisoned").0,
        "the history is faint at rest"
    );
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert!(
        seen.lock().expect("poisoned").0,
        "↑ on the first row should go into the history"
    );
    press_key(&mut harness, egui::Key::ArrowUp, egui::Modifiers::NONE);
    // Past the 90 ms the history takes to come up.
    harness.run_steps(12);
    if let Ok(path) = std::env::var("MOON_PALETTE_HISTORY_SNAPSHOT") {
        harness
            .render()
            .expect("the window renders")
            .save(path)
            .expect("the picture is written");
    }
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);

    let (in_use, opened) = seen.lock().expect("poisoned").clone();
    assert_eq!(opened.as_deref(), Some("terminal-1"));
    assert!(!in_use, "running an entry puts the history back to faint");
}

/// Enter on `! ...` runs the line at the repo's root under the palette, which stays open with
/// the output in a box under it; editing the line away from what ran throws the output away.
#[test]
fn a_line_of_shell_runs_under_the_palette() {
    let fixture = seeded_fixture("palette-shell");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    offer_agents(&mut app, &[AgentKind::Claude], AgentKind::Claude);
    let root = app.model.root_session_id.clone();
    // The line runs where the review's repo is.
    std::sync::Arc::get_mut(
        app.model
            .review(&root)
            .payload
            .as_mut()
            .expect("the payload was just given"),
    )
    .expect("nothing else holds the payload yet")
    .repo_path = fixture.root.display().to_string();
    // Whether the palette is open, what the box under it holds, and what to type next.
    let seen = Arc::new(Mutex::new((false, Vec::<String>::new(), None::<String>)));
    let seen_in_ui = Arc::clone(&seen);
    let mut frame = 0;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .wgpu()
        .build_ui(move |ui| {
            if frame == 0 {
                app.model.palette.show_commands(root.clone());
                app.model.palette.input.open_on("! ls && echo oops >&2");
            }
            frame += 1;
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, app.palette_of().bg);
            let mut seen = seen_in_ui.lock().expect("poisoned");
            if let Some(typed) = seen.2.take() {
                app.model.palette.input.set_query(&typed);
            }
            crate::native::palette::draw(&mut app, ui.ctx());
            seen.0 = app.model.palette.is_open();
            seen.1 = app
                .model
                .palette
                .shell
                .as_ref()
                .map(crate::native::palette::ShellRun::printed)
                .unwrap_or_default();
        });

    harness.run_steps(2);
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    for _ in 0..200 {
        if seen
            .lock()
            .expect("poisoned")
            .1
            .iter()
            .any(|line| line == "oops")
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        harness.step();
    }
    let (open, printed, _) = seen.lock().expect("poisoned").clone();
    assert!(open, "the palette stays open over the line's output");
    assert!(
        printed.iter().any(|line| line == "README.md") && printed.iter().any(|line| line == "oops"),
        "both streams should be in the box, got {printed:?}"
    );
    if let Ok(path) = std::env::var("MOON_PALETTE_SHELL_SNAPSHOT") {
        harness.run_steps(3);
        harness
            .render()
            .expect("the window renders")
            .save(path)
            .expect("the picture is written");
    }

    seen.lock().expect("poisoned").2 = Some("! ls -a".to_string());
    harness.run_steps(2);
    assert!(
        seen.lock().expect("poisoned").1.is_empty(),
        "a line edited away from what ran throws its output away"
    );
}
