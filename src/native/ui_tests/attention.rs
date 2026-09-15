//! A shell asking for a person reaches the messages once, and the card - never the desktop.

use super::{Fixture, app_for};
use crate::{api::TerminalAttentionView, native::theme::ThemeMode};

fn asking(terminal_id: &str, message: &str, at_unix: u64) -> TerminalAttentionView {
    TerminalAttentionView {
        terminal_id: terminal_id.to_string(),
        name: Some("write the parser claude - 1".to_string()),
        message: message.to_string(),
        at_unix,
    }
}

/// Every poll carries the same ask until it is answered, and the messages get it once. A
/// new ask from the same shell - answered, then asked again - is news again.
#[test]
fn an_ask_reaches_the_messages_once() {
    let fixture = Fixture::new("attention-messages");
    let mut app = app_for(&fixture.root, ThemeMode::Dark);
    let before = app.model.messages.len();

    app.model
        .take_attention(vec![asking("terminal-1", "Permission needs input", 100)]);
    app.model
        .take_attention(vec![asking("terminal-1", "Permission needs input", 100)]);
    assert_eq!(app.model.messages.len(), before + 1, "one ask, one message");
    let posted = app
        .model
        .messages
        .latest()
        .expect("expected the message")
        .text
        .clone();
    assert_eq!(
        posted,
        "write the parser claude - 1: Permission needs input"
    );
    assert_eq!(app.model.shells_wanting_attention.len(), 1);

    // Answered: the poll no longer carries it, and nothing is posted about that.
    app.model.take_attention(Vec::new());
    assert_eq!(app.model.messages.len(), before + 1);
    assert!(app.model.shells_wanting_attention.is_empty());

    // Asked again later, and a second shell with it: two more lines.
    app.model.take_attention(vec![
        asking("terminal-1", "Session done", 160),
        asking("terminal-2", "rang its bell", 161),
    ]);
    assert_eq!(app.model.messages.len(), before + 3);
}
