//! The `inspect` button on a Minerve run's row of the board, and the conversation pane it
//! opens.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable as _};

use crate::{
    agent_conversation::{
        AgentConversation, AgentMessage, AgentMessageRole, AgentState, AgentToolCall,
        AgentToolFunction,
    },
    api::AgentKind,
    native::{
        panes::{Pane, PaneKind},
        theme::ThemeMode,
    },
};

use super::{app_for, seeded_fixture, settle};

const TERMINAL_ID: &str = "terminal-of-a-minerve";

fn message(role: AgentMessageRole, content: &str) -> AgentMessage {
    AgentMessage {
        role,
        content: Some(content.to_string()),
        tool_calls: Vec::new(),
        tool_call_id: None,
    }
}

/// A turn that read a file, then edited it.
fn conversation() -> AgentConversation {
    let call = |id: &str, name: &str, arguments: &str| AgentToolCall {
        id: id.to_string(),
        function: AgentToolFunction {
            name: name.to_string(),
            arguments: arguments.to_string(),
        },
    };
    let answer = |id: &str, output: &str| AgentMessage {
        tool_call_id: Some(id.to_string()),
        ..message(AgentMessageRole::Tool, output)
    };
    AgentConversation {
        session_id: Some("5b7e".to_string()),
        state: Some(AgentState::WaitingForInput),
        messages: vec![
            message(AgentMessageRole::System, "You are Minerve"),
            message(AgentMessageRole::User, "greet louder"),
            AgentMessage {
                tool_calls: vec![call("c1", "read", r#"{"path":"src/lib.rs"}"#)],
                ..message(AgentMessageRole::Assistant, "")
            },
            answer("c1", "pub fn greet(name: &str) -> String {"),
            AgentMessage {
                tool_calls: vec![call(
                    "c2",
                    "edit",
                    r#"{"path":"src/lib.rs","edits":[{"oldText":"hello","newText":"HELLO"}]}"#,
                )],
                ..message(AgentMessageRole::Assistant, "")
            },
            answer("c2", "Edited src/lib.rs"),
            AgentMessage {
                tool_calls: vec![call(
                    "c3",
                    "linear__get_issue",
                    r#"{"id":"MOON-12","include_comments":true,"limit":20}"#,
                )],
                ..message(AgentMessageRole::Assistant, "")
            },
            answer("c3", "Error: MCP tool 'linear__get_issue': not signed in"),
            message(AgentMessageRole::Assistant, "It shouts now."),
        ],
        broken: None,
    }
}

/// A Minerve going for a task, as the board's answer would carry it.
fn running_minerve() -> crate::moontasks::TaskResourceView {
    crate::moontasks::TaskResourceView {
        id: "run-of-a-minerve".to_string(),
        kind: crate::moontasks::TaskResourceKind::Agent,
        agent: AgentKind::Minerve,
        label: "minerve - 1".to_string(),
        file_path: None,
        terminal_id: Some(TERMINAL_ID.to_string()),
        running: true,
        going_elsewhere_in: None,
        quiet_for_secs: None,
        attention: None,
        resumable: true,
        started_at_unix: 1700000001,
        started_by: None,
        work_tree: None,
    }
}

/// The board's own Minerve has no card, so its shell's header is titled for the board it is on,
/// and still opens its conversation.
#[test]
fn the_board_agents_header_is_titled_for_its_board() {
    let fixture = seeded_fixture("inspect-board-agent");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let project = fixture
        .root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .expect("the fixture is in a folder of its own");
    let on_board = Arc::new(AtomicBool::new(false));
    let on_board_in_ui = Arc::clone(&on_board);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !on_board_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.model.conversations.heard(TERMINAL_ID, conversation());
                let frame = app.model.layout.frame_ids()[0];
                let pane = app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: TERMINAL_ID.to_string(),
                        command: Some(AgentKind::Minerve),
                        task_id: Some("board-task".to_string()),
                    },
                    None,
                );
                app.model.layout.focus_pane(pane);
                on_board_in_ui.store(true, Ordering::Relaxed);
            }
            app.draw(ui);
        });
    harness.run_steps(3);
    harness.get_by_label(&format!("Board {project} agent"));
    assert_eq!(harness.query_all_by_label("inspect").count(), 1);
}

#[test]
fn inspect_in_a_minerve_shell_header_opens_its_conversation() {
    let fixture = seeded_fixture("inspect-minerve");
    fixture.write(
        ".moontasks/greet-louder-1111/metadata.json",
        "{\n  \"title\": \"Greet louder\",\n  \"status\": \"todo\",\n  \
         \"created_at_unix\": 1700000000,\n  \"resources\": []\n}\n",
    );
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let on_board = Arc::new(AtomicBool::new(false));
    let on_board_in_ui = Arc::clone(&on_board);
    let inspecting = Arc::new(AtomicBool::new(false));
    let inspecting_in_ui = Arc::clone(&inspecting);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                app.open_pane(crate::native::panes::OpenPaneRequest::Tasks);
                app.model.conversations.heard(TERMINAL_ID, conversation());
                opened_in_ui.store(true, Ordering::Relaxed);
            }
            // The fixture has no shell behind the run, so the run is put on the card here,
            // again after any answer of the board's that would take it off.
            if let Some(task) = app.model.board.tasks.first_mut()
                && task.resources.is_empty()
            {
                task.resources.push(running_minerve());
                let task_id = task.id.clone();
                if !on_board_in_ui.swap(true, Ordering::Relaxed) {
                    // Its shell, in a tab of its own under the task's header.
                    let frame = app.model.layout.frame_ids()[0];
                    let pane = app.model.layout.add_pane(
                        frame,
                        Pane::Terminal {
                            terminal_id: TERMINAL_ID.to_string(),
                            command: Some(AgentKind::Minerve),
                            task_id: Some(task_id),
                        },
                        None,
                    );
                    app.model.layout.focus_pane(pane);
                }
            }
            app.draw(ui);
            inspecting_in_ui.store(
                app.model
                    .layout
                    .find_pane(|pane| pane.kind() == PaneKind::Conversation)
                    .is_some(),
                Ordering::Relaxed,
            );
        });

    assert!(
        settle(&mut harness, || on_board.load(Ordering::Relaxed)),
        "the board never read the task out of .moontasks"
    );
    harness.run_steps(3);
    // In the shell's task header, and not on the run's row of the card.
    assert_eq!(harness.query_all_by_label("inspect").count(), 1);
    harness.get_by_label("inspect").click();
    assert!(
        settle(&mut harness, || inspecting.load(Ordering::Relaxed)),
        "inspect opened no conversation"
    );
    harness.step();
    harness.get_by_label("greet louder");
    harness.get_by_label("It shouts now.");
    harness.get_by_label("waiting for input");
    // A tool call is headed by what it is about.
    harness.get_by_label("read  src/lib.rs");
    // Opened, an edit shows the text it takes out and the text it puts in.
    harness.get_by_label("edit  src/lib.rs").click();
    harness.run_steps(3);
    harness.get_by_label("hello");
    harness.get_by_label("HELLO");
    // Anything else shows its arguments, as JSON.
    harness.get_by_label("linear__get_issue").click();
    harness.run_steps(3);
    harness.get_by_label_contains("\"include_comments\": true");

    // A prompt typed in the pane goes to the agent. This one has no server behind it, and
    // the pane says so rather than dropping it.
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text("louder still");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    let mut refused = false;
    for _ in 0..500 {
        harness.step();
        if harness
            .query_by_label_contains("could not send the prompt")
            .is_some()
        {
            refused = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(refused, "the prompt was never sent");
    // Kept, to be sent again.
    assert!(harness.query_all_by_value("louder still").next().is_some());
}
