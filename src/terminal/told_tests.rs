//! What a shell is told reaching the program in it: typed, sent, in order, and not while
//! somebody is in the way.
//!
//! A script stands in for an agent, started under its name the way the other pty tests start
//! one: it says back every line it is sent, so a line that was typed and never sent - or sent
//! in two halves - does not read as one that arrived.

#![cfg(unix)]

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::*;
use crate::terminal::tests::spawn_fake_claude;

/// Says each line it is sent back as `got[line]`.
const SAYS_BACK: &str =
    "#!/bin/sh\nwhile IFS= read -r line; do printf 'got[%s]\\n' \"$line\"; done\n";

/// The same, having first asked for a person the way an agent waiting on a permission does.
const ASKS_THEN_SAYS_BACK: &str = "#!/bin/sh\nprintf 'working\\033]9;Permission needs input\\007'\n\
     while IFS= read -r line; do printf 'got[%s]\\n' \"$line\"; done\n";

/// The same, having first said what Claude Code says once it has finished and nobody has
/// typed for a minute.
const WAITS_TO_BE_TYPED_AT_THEN_SAYS_BACK: &str = "#!/bin/sh\nprintf 'done\\033]9;Claude is waiting for your input\\007'\n\
     while IFS= read -r line; do printf 'got[%s]\\n' \"$line\"; done\n";

fn registry() -> Arc<TerminalRegistry> {
    Arc::new(TerminalRegistry::new(Arc::new(Mutex::new(Instant::now()))))
}

fn printed(session: &TerminalSession) -> String {
    String::from_utf8_lossy(&session.scrollback.lock().unwrap().replay()).to_string()
}

/// Wait for the shell to have printed this, and say whether it did.
fn prints(session: &TerminalSession, wanted: &str, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if printed(session).contains(wanted) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_told_line_is_typed_into_the_shell_and_sent() {
    let registry = registry();
    let terminal_id = spawn_fake_claude(&registry, SAYS_BACK);
    let session = registry.get(&terminal_id).expect("expected the shell");

    registry
        .tell(&terminal_id, "agent @bing-bong sent this message: hello")
        .expect("expected the shell to be told");

    assert!(
        prints(
            &session,
            "got[agent @bing-bong sent this message: hello]",
            Duration::from_secs(10)
        ),
        "the line should have been typed and sent, printed {:?}",
        printed(&session)
    );
    // Nobody at the window typed it.
    assert!(!session.has_been_typed_into());
    registry.remove(&terminal_id);
}

/// A shell asking for a person holds what it is told, and goes on asking: a told line is no
/// answer. Once somebody has answered, the lines wait out the moment after the keystroke as
/// well, and are then typed in the order they were told.
#[test]
fn told_lines_wait_while_somebody_is_in_the_way_and_keep_their_order() {
    let registry = registry();
    let terminal_id = spawn_fake_claude(&registry, ASKS_THEN_SAYS_BACK);
    let session = registry.get(&terminal_id).expect("expected the shell");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.attention(&terminal_id).is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        registry.attention(&terminal_id).is_some(),
        "expected the shell to be asking for a person"
    );

    for line in ["one", "two", "three"] {
        registry
            .tell(&terminal_id, line)
            .expect("expected the shell to be told");
    }

    assert!(
        !prints(&session, "got[one]", Duration::from_secs(1)),
        "a line was typed at a shell that is asking, printed {:?}",
        printed(&session)
    );
    assert!(
        registry.attention(&terminal_id).is_some(),
        "being told a line is not an answer"
    );

    let answered = Instant::now();
    session
        .write_input(b"y\n")
        .expect("expected the answer to be typed");
    assert!(prints(&session, "got[y]", Duration::from_secs(10)));
    assert!(
        !prints(&session, "got[one]", Duration::from_secs(1)),
        "a line was typed straight after a keystroke, printed {:?}",
        printed(&session)
    );

    assert!(
        prints(
            &session,
            "got[one]",
            WAITS_AFTER_TYPING + Duration::from_secs(10)
        ),
        "the lines were never typed, printed {:?}",
        printed(&session)
    );
    assert!(
        answered.elapsed() >= WAITS_AFTER_TYPING,
        "the first line was typed {:?} after a keystroke",
        answered.elapsed()
    );
    assert!(
        prints(&session, "got[three]", Duration::from_secs(10)),
        "the last line was never typed, printed {:?}",
        printed(&session)
    );
    let printed = printed(&session);
    let at = |line: &str| printed.find(line).expect("every line was printed");
    assert!(
        at("got[one]") < at("got[two]") && at("got[two]") < at("got[three]"),
        "the lines arrived out of order, printed {printed:?}"
    );
    registry.remove(&terminal_id);
}

/// An agent that has finished and said it is waiting for input is asking for exactly what a
/// told line is, so the line is typed - and the ask, having been answered, is taken off.
#[test]
fn an_agent_only_waiting_to_be_typed_at_is_told_and_stops_asking() {
    let registry = registry();
    let terminal_id = spawn_fake_claude(&registry, WAITS_TO_BE_TYPED_AT_THEN_SAYS_BACK);
    let session = registry.get(&terminal_id).expect("expected the shell");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.attention(&terminal_id).is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        registry.attention(&terminal_id).is_some(),
        "expected the agent to be saying it is waiting"
    );

    registry
        .tell(&terminal_id, "agent @bing-bong sent this message: done yet?")
        .expect("expected the shell to be told");

    assert!(
        prints(
            &session,
            "got[agent @bing-bong sent this message: done yet?]",
            Duration::from_secs(10)
        ),
        "an agent waiting to be typed at was not, printed {:?}",
        printed(&session)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.attention(&terminal_id).is_some() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        registry.attention(&terminal_id).is_none(),
        "the line was what the agent was waiting for"
    );
    registry.remove(&terminal_id);
}

#[test]
fn a_line_still_held_when_the_shell_ends_is_dropped() {
    let registry = registry();
    let terminal_id = spawn_fake_claude(&registry, ASKS_THEN_SAYS_BACK);
    let session = registry.get(&terminal_id).expect("expected the shell");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && registry.attention(&terminal_id).is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    registry
        .tell(&terminal_id, "never typed")
        .expect("expected the shell to be told");
    assert!(session.told.lock().unwrap().being_typed);

    registry.remove(&terminal_id);

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && session.told.lock().unwrap().being_typed {
        std::thread::sleep(Duration::from_millis(20));
    }
    let told = session.told.lock().unwrap();
    assert!(!told.being_typed, "the thread typing them should have gone");
    assert!(told.waiting.is_empty(), "the line should have been dropped");
    drop(told);
    assert!(!printed(&session).contains("got[never typed]"));
}

#[test]
fn only_one_line_of_text_is_told_and_only_to_a_shell_that_is_there() {
    let registry = registry();
    let terminal_id = spawn_fake_claude(&registry, SAYS_BACK);

    for not_a_line in ["", "one\ntwo", "sent early\r", "escape\u{1b}[2J"] {
        assert!(
            registry.tell(&terminal_id, not_a_line).is_err(),
            "{not_a_line:?} should be refused"
        );
    }
    assert!(registry.tell("terminal-nobody-0", "hello").is_err());

    let session = registry.get(&terminal_id).expect("expected the shell");
    assert!(session.told.lock().unwrap().waiting.is_empty());
    registry.remove(&terminal_id);
}
