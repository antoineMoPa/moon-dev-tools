//! The MCP server an agent working in a task talks to.
//!
//! `moonreview mcp` is started by the agent, not by a user: moonreview writes an MCP config
//! into the task folder naming this executable, and the environment it starts the agent with
//! says which task and which review server the run belongs to. The tools here are the ones an
//! agent needs to run its own card across the board — mostly, to say it is done.
//!
//! It speaks newline-delimited JSON-RPC over stdio, and does its work through the same HTTP
//! API the web frontend uses, so it works just as well against a server on another machine.

use std::{
    io::{BufRead, Write},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::moontasks::{
    SERVER_URL_ENV_VAR, SESSION_ID_ENV_VAR, TASK_ID_ENV_VAR, TaskStatusRequest, store::TaskStatus,
};

/// The protocol revision to answer with when a client does not name one.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// What the run was told about itself, which is the whole of this server's configuration.
struct Run {
    server_url: String,
    session_id: String,
    task_id: String,
    client: Client,
}

impl Run {
    fn from_environment() -> Result<Self> {
        let read = |name: &str| {
            std::env::var(name).with_context(|| {
                format!("{name} is not set — moonreview starts this server, agents do not")
            })
        };

        Ok(Self {
            server_url: read(SERVER_URL_ENV_VAR)?
                .trim_end_matches('/')
                .to_string(),
            session_id: read(SESSION_ID_ENV_VAR)?,
            task_id: read(TASK_ID_ENV_VAR)?,
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("failed to build the HTTP client")?,
        })
    }

    fn set_status(&self, status: TaskStatus) -> Result<()> {
        let response = self
            .client
            .post(format!(
                "{}/api/session/{}/tasks/{}/status",
                self.server_url, self.session_id, self.task_id
            ))
            .json(&TaskStatusRequest { status })
            .send()
            .context("failed to reach the moonreview server")?;
        if !response.status().is_success() {
            bail!(
                "the moonreview server refused the move: {}",
                response.text().unwrap_or_default()
            );
        }
        Ok(())
    }

    fn task(&self) -> Result<Value> {
        let tasks: Value = self
            .client
            .get(format!(
                "{}/api/session/{}/tasks",
                self.server_url, self.session_id
            ))
            .send()
            .context("failed to reach the moonreview server")?
            .error_for_status()
            .context("the moonreview server refused to list the board")?
            .json()
            .context("could not decode the board")?;

        tasks
            .as_array()
            .and_then(|tasks| {
                tasks
                    .iter()
                    .find(|task| task.get("id").and_then(Value::as_str) == Some(&self.task_id))
            })
            .cloned()
            .ok_or_else(|| anyhow!("this task is no longer on the board"))
    }
}

/// The tools an agent is offered, and what each one is for.
fn tool_definitions() -> Value {
    let statuses: Vec<&str> = crate::moontasks::store::BOARD_COLUMNS
        .iter()
        .map(|status| status.wire_name())
        .collect();

    json!([
        {
            "name": "moontasks_set_status",
            "description": "Move the task you are working on to another column of the moonreview \
                            board. Call this with in_local_review when you have finished the work \
                            and it is ready to be looked at.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": statuses,
                        "description": "The column to move the task to.",
                    }
                },
                "required": ["status"],
            },
        },
        {
            "name": "moontasks_get_task",
            "description": "Read the task you are working on: its title, its column, and the \
                            shells and agent runs it has.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ])
}

fn call_tool(run: &Run, name: &str, arguments: &Value) -> Result<String> {
    match name {
        "moontasks_set_status" => {
            let requested = arguments
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("status is required"))?;
            let status = TaskStatus::from_wire_name(requested)
                .ok_or_else(|| anyhow!("{requested} is not a column of the board"))?;
            run.set_status(status)?;
            Ok(format!("the task is now in {}", status.label()))
        }
        "moontasks_get_task" => Ok(serde_json::to_string_pretty(&run.task()?)?),
        unknown => bail!("{unknown} is not a tool of this server"),
    }
}

/// Answer one request. `None` means the message was a notification, which gets no reply.
fn handle(run: &Run, message: &Value) -> Option<Value> {
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
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // A tool that fails is reported inside the result, not as a protocol error: the
            // agent is meant to read what went wrong and try something else.
            match call_tool(run, name, &arguments) {
                Ok(text) => Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false,
                })),
                Err(error) => Ok(json!({
                    "content": [{ "type": "text", "text": format!("{error}") }],
                    "isError": true,
                })),
            }
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

/// Serve on stdio until the agent closes it.
pub(crate) fn run() -> Result<()> {
    let run = Run::from_environment()?;
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    for line in stdin.lines() {
        let line = line.context("failed to read from the agent")?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(response) = handle(&run, &message) else {
            continue;
        };
        writeln!(stdout, "{response}").context("failed to answer the agent")?;
        stdout.flush()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_for_tests() -> Run {
        Run {
            server_url: "http://127.0.0.1:1".to_string(),
            session_id: "session".to_string(),
            task_id: "task".to_string(),
            client: Client::builder()
                .timeout(Duration::from_millis(50))
                .build()
                .expect("expected a client"),
        }
    }

    #[test]
    fn initialize_answers_with_the_protocol_the_agent_asked_for() {
        let response = handle(
            &run_for_tests(),
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
    fn a_notification_gets_no_answer() {
        let response = handle(
            &run_for_tests(),
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        );

        assert!(response.is_none());
    }

    #[test]
    fn every_column_of_the_board_is_offered_as_a_status() {
        let tools = tool_definitions();
        let statuses = tools[0]["inputSchema"]["properties"]["status"]["enum"]
            .as_array()
            .expect("expected the status enum")
            .clone();

        assert_eq!(statuses.len(), crate::moontasks::store::BOARD_COLUMNS.len());
        assert!(statuses.contains(&json!("in_local_review")));
    }

    #[test]
    fn a_column_that_does_not_exist_is_reported_to_the_agent_rather_than_the_protocol() {
        let response = handle(
            &run_for_tests(),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "moontasks_set_status", "arguments": { "status": "blocked" } },
            }),
        )
        .expect("expected a response");

        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("expected text")
                .contains("not a column")
        );
    }
}
