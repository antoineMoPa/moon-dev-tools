//! The `inspect` button over a Minerve shell, and the conversation pane it opens.

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
            message(AgentMessageRole::Assistant, "It shouts now."),
        ],
        broken: None,
    }
}

#[test]
fn inspect_on_a_minerve_shell_opens_its_conversation_beside_it() {
    let fixture = seeded_fixture("inspect-minerve");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let opened = Arc::new(AtomicBool::new(false));
    let opened_in_ui = Arc::clone(&opened);
    let inspecting = Arc::new(AtomicBool::new(false));
    let inspecting_in_ui = Arc::clone(&inspecting);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui(move |ui| {
            if !opened_in_ui.load(Ordering::Relaxed)
                && matches!(app.model.stage, crate::native::model::Stage::Ready)
            {
                let frame = app.model.layout.frame_ids()[0];
                let pane = app.model.layout.add_pane(
                    frame,
                    Pane::Terminal {
                        terminal_id: TERMINAL_ID.to_string(),
                        command: Some(AgentKind::Minerve),
                        task_id: None,
                    },
                    None,
                );
                app.model.layout.focus_pane(pane);
                app.model.conversations.heard(TERMINAL_ID, conversation());
                opened_in_ui.store(true, Ordering::Relaxed);
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
        settle(&mut harness, || opened.load(Ordering::Relaxed)),
        "the window never got ready"
    );
    harness.step();
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
