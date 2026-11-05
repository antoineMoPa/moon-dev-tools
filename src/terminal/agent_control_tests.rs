use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    sync::Weak,
    time::{Duration, Instant},
};

use super::*;
use crate::agent_conversation::AgentMessageRole;

/// An agent on the other end of a fresh control socket, the way Minerve connects to it.
fn connected_agent() -> (Arc<AgentControl>, UnixStream) {
    let (control, listener) = AgentControl::listen().unwrap();
    control.read_in_background(listener, Weak::new());
    let agent = UnixStream::connect(control.socket_path()).unwrap();
    agent
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    (control, agent)
}

fn write_lines(agent: &mut UnixStream, lines: &[&str]) {
    for line in lines {
        agent.write_all(line.as_bytes()).unwrap();
        agent.write_all(b"\n").unwrap();
    }
}

fn wait_for(
    control: &AgentControl,
    done: impl Fn(&AgentConversation) -> bool,
) -> AgentConversation {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let conversation = control.conversation();
        if done(&conversation) {
            return conversation;
        }
        assert!(
            Instant::now() < deadline,
            "never got there: {conversation:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_session_asks_for_the_history_and_messages_after_it_are_added() {
    let (control, mut agent) = connected_agent();
    write_lines(
        &mut agent,
        &[
            r#"{"type":"hello","version":1}"#,
            r#"{"type":"session","session_id":"s1"}"#,
        ],
    );
    let mut asked = String::new();
    BufReader::new(agent.try_clone().unwrap())
        .read_line(&mut asked)
        .unwrap();
    assert_eq!(asked, "{\"type\":\"get_history\"}\n");

    write_lines(
        &mut agent,
        &[
            r#"{"type":"history","session_id":"s1","messages":[{"role":"system","content":"be nice"}]}"#,
            r#"{"type":"state","state":"working"}"#,
            r#"{"type":"message","message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}}"#,
            r#"{"type":"message","message":{"role":"tool","content":"hello","tool_call_id":"c1"}}"#,
        ],
    );
    let conversation = wait_for(&control, |conversation| conversation.messages.len() == 3);
    assert_eq!(conversation.session_id.as_deref(), Some("s1"));
    assert_eq!(conversation.state, Some(AgentState::Working));
    assert_eq!(
        conversation.messages[1].tool_calls[0].function.name,
        "read_file"
    );
    assert_eq!(conversation.messages[2].role, AgentMessageRole::Tool);
    assert_eq!(conversation.messages[2].tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn the_socket_is_gone_once_the_agent_connected() {
    let (control, _agent) = connected_agent();
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.socket_path().exists() {
        assert!(Instant::now() < deadline, "the socket is still on disk");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn another_protocol_version_stops_the_reading_and_says_why() {
    let (control, mut agent) = connected_agent();
    write_lines(&mut agent, &[r#"{"type":"hello","version":2}"#]);
    let conversation = wait_for(&control, |conversation| conversation.broken.is_some());
    assert!(conversation.broken.unwrap().contains("version 2"));
}

#[test]
fn an_unreadable_line_stops_the_reading_and_says_why() {
    let (control, mut agent) = connected_agent();
    write_lines(&mut agent, &[r#"{"type":"dance"}"#]);
    let conversation = wait_for(&control, |conversation| conversation.broken.is_some());
    assert!(conversation.broken.unwrap().contains("dance"));
}
