//! The command list in sections, with what the two searches find streaming into them.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::Harness;

use crate::{
    api::{AgentKind, AgentOption, ContentMatch, SessionPayload},
    native::{
        app::App,
        palette::{CommandAction, PaletteMode, Search, SearchRequest},
        panes::OpenPaneRequest,
        theme::ThemeMode,
    },
};

use super::{app_for, press_key, seeded_fixture};

/// Matches arriving in the files section, above the line the highlight is on, move that line
/// down the list - and the highlight with it: Enter still opens the line that was picked.
#[test]
fn the_highlight_stays_on_its_row_as_matches_arrive_above_it() {
    let fixture = seeded_fixture("palette-sections");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let files_arrive = Arc::new(AtomicBool::new(false));
    let files_arrive_in_ui = Arc::clone(&files_arrive);
    // The highlighted row, and the file and line Enter opened, once it has.
    let highlight = Arc::new(Mutex::new((0usize, None::<(String, usize)>)));
    let highlight_in_ui = Arc::clone(&highlight);
    let mut frame = 0;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .wgpu()
        .build_ui(move |ui| {
            let palette = &mut app.model.palette;
            if frame == 0 {
                palette.show_commands("searched-session".to_string());
                palette.input.open_on("wl");
            }
            // Both searches answered for what is typed, as if typing had paused: the files'
            // has found nothing yet, the content search one line.
            let asked = SearchRequest {
                query: "wl".to_string(),
                scope: palette.search_scope,
            };
            if frame == 1 {
                palette.files = Search {
                    searched: Some(asked.clone()),
                    ticket: 7,
                    ..Search::default()
                };
                palette.contents = Search {
                    searched: Some(asked),
                    ticket: 7,
                    matches: vec![ContentMatch {
                        file_path: "src/log.rs".to_string(),
                        line_number: 3,
                        line: "fn wl() {}".to_string(),
                    }],
                    ..Search::default()
                };
            }
            if files_arrive_in_ui.swap(false, Ordering::Relaxed) {
                palette.files.matches = vec!["wl.rs".to_string(), "src/wl.rs".to_string()];
            }
            frame += 1;
            crate::native::palette::draw(&mut app, ui.ctx());
            assert_eq!(app.model.palette.mode, PaletteMode::Commands);
            let opened = match app.pending_action.take() {
                Some(CommandAction::OpenPane(OpenPaneRequest::File {
                    file_path,
                    at: Some(at),
                    ..
                })) => Some((file_path, at.line)),
                Some(_) => panic!("Enter should have opened a file"),
                None => None,
            };
            let mut seen = highlight_in_ui.lock().expect("poisoned");
            seen.0 = app.model.palette.highlighted();
            seen.1 = seen.1.take().or(opened);
        });

    harness.run_steps(3);
    // To the last row, the line the content search found.
    for _ in 0..20 {
        press_key(&mut harness, egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    let (on_line, _) = highlight.lock().expect("poisoned").clone();

    files_arrive.store(true, Ordering::Relaxed);
    harness.run_steps(2);

    let (after, _) = highlight.lock().expect("poisoned").clone();
    assert_eq!(
        after,
        on_line + 2,
        "the two files listed above the line should have moved the highlight down with it"
    );
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        highlight.lock().expect("poisoned").1,
        Some(("src/log.rs".to_string(), 3)),
        "Enter should have opened the line that was picked"
    );
}

/// Give the window's review a payload that offers `agents`, with `selected` the window's
/// selected agent: what the palette's task row is made of.
pub(crate) fn offer_agents(app: &mut App, agents: &[AgentKind], selected: AgentKind) {
    let session_id = app.model.root_session_id.clone();
    app.model.review(&session_id).payload = Some(Arc::new(SessionPayload {
        repo_name: "repo".to_string(),
        branch_name: None,
        commit_base: None,
        commits: Vec::new(),
        history_commits: Vec::new(),
        history_has_more: false,
        local_change_summary: Default::default(),
        active_commit: None,
        repo_path: "/repo".to_string(),
        read_only: false,
        patch_preview_line_limit: 500,
        available_agents: agents
            .iter()
            .map(|kind| AgentOption {
                kind: *kind,
                label: kind.label().to_string(),
                available: true,
                logged_in: None,
                log_in_command: None,
            })
            .collect(),
        selected_agent: selected,
        full_file_path: None,
        hunks: Vec::new(),
        review_comments: Vec::new(),
        export_text: String::new(),
    }));
}

/// ← and → on the task row are the query line's, to move along what is typed; Enter starts
/// the task with the agent its row has picked.
#[test]
fn arrows_leave_the_task_rows_agent_and_enter_starts_it() {
    let fixture = seeded_fixture("palette-task-chips");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    offer_agents(
        &mut app,
        &[AgentKind::Claude, AgentKind::Codex, AgentKind::Pi],
        AgentKind::Codex,
    );
    let started = Arc::new(Mutex::new(None::<AgentKind>));
    let started_in_ui = Arc::clone(&started);
    let mut frame = 0;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 760.0))
        .build_ui(move |ui| {
            if frame == 0 {
                // No review to search: the task row is the only row.
                app.model.palette.show_commands(String::new());
                app.model.palette.input.open_on("zqx fix the login");
            }
            frame += 1;
            crate::native::palette::draw(&mut app, ui.ctx());
            match app.pending_action.take() {
                Some(CommandAction::StartQuickTask { title, agent }) => {
                    assert_eq!(title, "zqx fix the login");
                    *started_in_ui.lock().expect("poisoned") = Some(agent);
                }
                Some(_) => panic!("Enter should have started the quick task"),
                None => {}
            }
        });

    harness.run_steps(2);
    press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::ArrowRight, egui::Modifiers::NONE);
    press_key(&mut harness, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        *started.lock().expect("poisoned"),
        Some(AgentKind::Codex),
        "the arrows should have moved the caret, not the agent"
    );
}
