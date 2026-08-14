//! Making a headless run readable while it is still running.
//!
//! An agent asked for one answer prints nothing until it has one. `claude -p` is the plain
//! case: minutes of work, then a paragraph — so a tab open on the run shows an empty screen the
//! whole time, and so does the run's log, which is what a hook reads to find out what happened.
//!
//! The agents that can say what they are doing as they do it say it as JSON, one object per
//! line, which is worse than nothing to look at. So the run is started that way and the lines
//! are turned back into something a person can read on their way past — here, once, so the tab,
//! the scrollback and the log all show the same thing.
//!
//! [`OutputStyle::AsPrinted`] is every other program: what it printed, unread.

use serde_json::Value;

/// How a program's output is read before anyone sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum OutputStyle {
    /// Pass it through. What a terminal has always done.
    #[default]
    AsPrinted,
    /// One JSON object per line, as `claude --output-format stream-json` prints.
    ClaudeStreamJson,
}

/// Reads a program's output, a chunk at a time, and answers with what to show.
pub(crate) struct Reader {
    style: OutputStyle,
    /// The last line, still arriving. A chunk boundary falls wherever the pipe says it does,
    /// which is not where a line ends.
    partial: Vec<u8>,
}

/// How long one line of what a run is doing may be before it is cut.
///
/// Long enough for a command or a sentence, short enough that a wall of file contents in a
/// tool result stays one line of the story rather than becoming the story.
const LINE_LIMIT: usize = 240;

/// The argument worth showing for a tool call, per tool.
///
/// A tool's input is a JSON object of whatever that tool takes, and one field of it is almost
/// always the thing a person reading along wants to see. Anything not named here shows its
/// input compactly.
const TOOL_ARGUMENTS: &[(&str, &str)] = &[
    ("Bash", "command"),
    ("Read", "file_path"),
    ("Edit", "file_path"),
    ("Write", "file_path"),
    ("NotebookEdit", "notebook_path"),
    ("Glob", "pattern"),
    ("Grep", "pattern"),
    ("WebFetch", "url"),
    ("WebSearch", "query"),
    ("Task", "description"),
];

impl Reader {
    pub(crate) fn new(style: OutputStyle) -> Self {
        Self {
            style,
            partial: Vec::new(),
        }
    }

    /// What to show for one chunk of what the program printed.
    pub(crate) fn read(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.style == OutputStyle::AsPrinted {
            return chunk.to_vec();
        }

        self.partial.extend_from_slice(chunk);
        let mut shown = Vec::new();
        // Only whole lines are read; the rest waits for the chunk that finishes it.
        while let Some(at) = self.partial.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=at).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                // A line the agent prints that is not one of its events — a warning on its way
                // out, say — is shown as it is rather than swallowed for not being JSON.
                Err(_) => shown.push(line.to_string()),
                Ok(event) => shown.extend(told_of(&event)),
            }
        }

        // A pty shows a line where it is told to, so every line ends where the last one did.
        shown
            .into_iter()
            .flat_map(|line| format!("{}\r\n", cut(&line)).into_bytes())
            .collect()
    }

    /// What is left when the program has stopped: a last line with no newline after it.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        let rest = std::mem::take(&mut self.partial);
        match rest.is_empty() {
            true => Vec::new(),
            false => self.read(b"\n"),
        }
    }
}

/// What one event of a run says it did, or nothing for an event with nothing to say.
fn told_of(event: &Value) -> Vec<String> {
    match event.get("type").and_then(Value::as_str).unwrap_or("") {
        // Only the one that says a run has begun. An agent says `system` for its own
        // housekeeping too — compacting its context, and whatever it grows next — and none of
        // that is what the run is doing.
        "system" if event.get("subtype").and_then(Value::as_str) == Some("init") => {
            vec![format!(
                "· started{}",
                match event.get("model").and_then(Value::as_str) {
                    Some(model) => format!(" · {model}"),
                    None => String::new(),
                }
            )]
        }
        "assistant" => blocks_of(event).iter().filter_map(said).collect(),
        "user" => blocks_of(event).iter().filter_map(answered).collect(),
        "result" => {
            let failed = event
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            vec![format!(
                "· {}",
                match failed {
                    true => "the run ended with an error",
                    false => "done",
                }
            )]
        }
        _ => Vec::new(),
    }
}

fn blocks_of(event: &Value) -> Vec<Value> {
    event
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// One thing the agent said or did.
fn said(block: &Value) -> Option<String> {
    match block.get("type").and_then(Value::as_str)? {
        "text" => {
            let text = block.get("text")?.as_str()?.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        "tool_use" => {
            let name = block.get("name")?.as_str()?;
            let input = block.get("input")?;
            Some(format!("→ {name}{}", argument_of(name, input)))
        }
        _ => None,
    }
}

/// What came back from something it did. Only whether it worked and the first line of it: the
/// whole of a tool result is usually a file, and this is a running account rather than a record.
fn answered(block: &Value) -> Option<String> {
    if block.get("type").and_then(Value::as_str)? != "tool_result" {
        return None;
    }
    if block
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Some("  ← failed".to_string());
    }
    let content = block.get("content")?;
    let text = match content.as_str() {
        Some(text) => text.to_string(),
        None => content
            .as_array()?
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
    };
    let first = text.lines().find(|line| !line.trim().is_empty())?;
    Some(format!("  ← {}", first.trim()))
}

fn argument_of(name: &str, input: &Value) -> String {
    let named = TOOL_ARGUMENTS
        .iter()
        .find(|(tool, _)| *tool == name)
        .and_then(|(_, argument)| input.get(*argument))
        .and_then(Value::as_str)
        .map(str::to_string);

    match named {
        Some(shown) => format!(" {}", shown.replace('\n', " ")),
        None => match input.as_object().is_some_and(|input| input.is_empty()) {
            true => String::new(),
            false => format!(" {}", input),
        },
    }
}

fn cut(line: &str) -> String {
    match line.char_indices().nth(LINE_LIMIT) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read(events: &[Value]) -> String {
        let mut reader = Reader::new(OutputStyle::ClaudeStreamJson);
        let mut shown = Vec::new();
        for event in events {
            shown.extend(reader.read(format!("{event}\n").as_bytes()));
        }
        shown.extend(reader.finish());
        String::from_utf8(shown).expect("expected text")
    }

    #[test]
    fn what_a_program_printed_is_passed_through_unread() {
        let mut reader = Reader::new(OutputStyle::AsPrinted);

        assert_eq!(reader.read(b"{ not an event }"), b"{ not an event }");
    }

    /// The point of the whole module: something to read while the run is still going.
    #[test]
    fn a_run_says_what_it_is_doing_as_it_does_it() {
        let shown = read(&[
            json!({ "type": "system", "subtype": "init", "model": "claude-opus-5" }),
            json!({
                "type": "assistant",
                "message": { "content": [
                    { "type": "text", "text": "Looking at the parser." },
                    { "type": "tool_use", "name": "Read", "input": { "file_path": "src/parse.rs" } },
                ] },
            }),
            json!({
                "type": "user",
                "message": { "content": [
                    { "type": "tool_result", "content": "one\ntwo\nthree" },
                ] },
            }),
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        ]);

        assert_eq!(
            shown,
            "· started · claude-opus-5\r\n\
             Looking at the parser.\r\n\
             → Read src/parse.rs\r\n\
             \x20 ← one\r\n\
             · done\r\n"
        );
    }

    /// A chunk ends where the pipe says it does, which is regularly mid-line.
    #[test]
    fn an_event_split_across_chunks_is_read_once_it_is_whole() {
        let mut reader = Reader::new(OutputStyle::ClaudeStreamJson);
        let event = json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "Half a line" },
        ] } })
        .to_string();
        let (first, second) = event.split_at(20);

        assert!(reader.read(first.as_bytes()).is_empty());
        let shown = reader.read(format!("{second}\n").as_bytes());

        assert_eq!(String::from_utf8_lossy(&shown), "Half a line\r\n");
    }

    /// An agent's own housekeeping is not what the run is doing, and saying "started" in the
    /// middle of one reads as a run that restarted itself.
    #[test]
    fn housekeeping_is_not_reported_as_the_run_starting_again() {
        let shown = read(&[
            json!({ "type": "system", "subtype": "init", "model": "claude-opus-5" }),
            json!({ "type": "system", "subtype": "compact_boundary" }),
        ]);

        assert_eq!(shown, "· started · claude-opus-5\r\n");
    }

    /// A run that failed has to say so where the person is looking.
    #[test]
    fn a_run_that_ended_badly_says_so() {
        let shown = read(&[json!({ "type": "result", "is_error": true })]);

        assert!(shown.contains("ended with an error"), "got {shown}");
    }

    /// Something the agent prints that is not one of its events is still worth showing.
    #[test]
    fn a_line_that_is_not_an_event_is_shown_as_it_is() {
        let mut reader = Reader::new(OutputStyle::ClaudeStreamJson);

        let shown = reader.read(b"warning: something\n");

        assert_eq!(String::from_utf8_lossy(&shown), "warning: something\r\n");
    }

    /// A tool result is one line of the story, not the story.
    #[test]
    fn a_very_long_line_is_cut() {
        let shown = read(&[json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": "x".repeat(1000) }] },
        })]);

        assert!(shown.chars().count() < LINE_LIMIT + 10, "got {shown:?}");
        assert!(shown.contains('…'));
    }
}
