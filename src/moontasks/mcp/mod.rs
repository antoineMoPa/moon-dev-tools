//! The tools an agent is given to work the board with.

#[cfg(test)]
mod tool_tests;
mod tools;

use anyhow::Result;
use serde_json::{Value, json};

use crate::api::AppState;

/// The protocol revision to answer with when a client does not name one.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// What one request knows about itself: which board it is working, and where.
pub(crate) struct Board<'a> {
    pub(crate) state: &'a AppState,
    pub(crate) session_id: String,
}

/// Answer one JSON-RPC message. `None` means it was a notification, which gets no reply.
pub(crate) fn handle(board: Result<Board<'_>>, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "moontasks",
                "version": env!("MOONREVIEW_VERSION"),
            },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            // A tool that fails is reported inside the result rather than as a protocol
            // error: the agent is meant to read what went wrong and try something else, and
            // most of these failures are refusals it is expected to handle.
            let answer = board.and_then(|board| tools::call(&board, name, &arguments));
            Ok(match answer {
                Ok(text) => json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false,
                }),
                Err(error) => json!({
                    "content": [{ "type": "text", "text": format!("{error}") }],
                    "isError": true,
                }),
            })
        }
        unknown => Err(format!("unknown method {unknown}")),
    };

    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message },
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_board() -> Result<Board<'static>> {
        anyhow::bail!("no board here")
    }

    #[test]
    fn initialize_answers_with_the_protocol_the_agent_asked_for() {
        let response = handle(
            no_board(),
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2025-06-18" },
            }),
        )
        .expect("expected a response");

        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(response["result"]["serverInfo"]["name"], "moontasks");
    }

    #[test]
    fn a_client_that_names_no_protocol_is_answered_with_one_anyway() {
        let response = handle(
            no_board(),
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
        .expect("expected a response");

        assert_eq!(
            response["result"]["protocolVersion"],
            DEFAULT_PROTOCOL_VERSION
        );
    }

    #[test]
    fn a_notification_gets_no_answer() {
        let response = handle(
            no_board(),
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );

        assert!(response.is_none());
    }

    #[test]
    fn a_method_this_server_does_not_have_is_a_protocol_error() {
        let response = handle(
            no_board(),
            &json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" }),
        )
        .expect("expected a response");

        assert_eq!(response["error"]["code"], -32601);
    }

    /// A board that could not be opened is still an answer the agent can read, rather than a
    /// dead connection it can only guess about.
    #[test]
    fn a_tool_called_without_a_board_says_so_in_the_result() {
        let response = handle(
            no_board(),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "list_tasks", "arguments": {} },
            }),
        )
        .expect("expected a response");

        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("expected text")
                .contains("no board")
        );
    }
}
