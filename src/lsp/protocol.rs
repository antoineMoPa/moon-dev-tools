//! The messages themselves: what is sent to a server, what is made of its answers, and the
//! one place a position is turned from the editor's units into the server's.
//!
//! [`lsp_types`] is used for the answers, where the protocol has three shapes for a
//! definition and two for a completion list, and plain JSON for what goes out, which is
//! short enough to read as the message it is.

use anyhow::{Context, Result, bail};
use lsp_types::{
    CompletionItem, CompletionResponse, GotoDefinitionResponse, InitializeResult, Location,
    PositionEncodingKind, ServerCapabilities,
};
use serde_json::{Value, json};

use super::process::PositionEncoding;
use crate::api::{LspCompletion, LspLocation, LspPosition};

/// What is asked of a server as it starts: the repo as the workspace, and - the point of
/// this - a request to count columns in bytes.
///
/// Bytes are what the editor has. The protocol's default is UTF-16 code units, so without
/// this every position on a line holding anything outside ASCII is wrong by however many
/// bytes the multi-byte characters before it take. 3.17 lets a client ask for another
/// encoding, and what the server answers with is read back by [`agreed_encoding`] rather
/// than assumed - a server free to ignore the request is a server that will.
pub(crate) fn initialize_params(repo_root: &std::path::Path) -> Value {
    let root_uri = file_uri(repo_root);
    json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "moonreview" },
        "rootUri": root_uri,
        "workspaceFolders": [{
            "uri": root_uri,
            "name": repo_root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace"),
        }],
        "capabilities": {
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "window": { "workDoneProgress": true },
            "textDocument": {
                "synchronization": { "dynamicRegistration": false },
                "definition": { "linkSupport": true },
                "completion": {
                    "completionItem": { "snippetSupport": false },
                    "contextSupport": false,
                },
            },
        },
    })
}

/// What the server actually agreed to count columns in. A server that says nothing has
/// agreed to nothing, which the protocol says means UTF-16 - and then every position is
/// converted rather than passed through.
pub(crate) fn agreed_encoding(initialize_reply: &Value) -> Result<PositionEncoding> {
    let result: InitializeResult = serde_json::from_value(initialize_reply.clone())
        .context("the language server's initialize reply could not be read")?;
    Ok(encoding_of(&result.capabilities))
}

fn encoding_of(capabilities: &ServerCapabilities) -> PositionEncoding {
    match capabilities.position_encoding.as_ref() {
        Some(kind) if *kind == PositionEncodingKind::UTF8 => PositionEncoding::Utf8,
        _ => PositionEncoding::Utf16,
    }
}

/// Answer one of the server's own requests. Only the shape matters: a server that asked for
/// its configuration and is handed one entry per item it asked about carries on, and one
/// left waiting stops.
pub(crate) fn reply_to_server_request(method: &str, message: &Value) -> Value {
    match method {
        "workspace/configuration" => {
            let asked_for = message
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; asked_for])
        }
        // `window/workDoneProgress/create`, `client/registerCapability` and the rest are
        // all served by "yes, fine".
        _ => Value::Null,
    }
}

/// Which end of a piece of work a `$/progress` notification is - `begin`, `report` or
/// `end`. This is what readiness is counted from; see [`super::process`].
pub(crate) fn progress_kind(message: &Value) -> Option<&str> {
    message.pointer("/params/value/kind")?.as_str()
}

/// What the same notification says the server is actually doing, which is what the window's
/// status bar reads out - see [`crate::native::status_bar`].
///
/// Every field is optional because the protocol makes them so, and the bar is written to
/// read well without any of them: only a `begin` carries a `title`, a `report` may carry
/// nothing but a `message`, and plenty of work reports no percentage at all - rust-analyzer
/// fetches a project's metadata without ever saying how far through it is. Carrying the
/// title forward from the `begin` is [`super::process::Working`]'s business, since it is the
/// only thing that sees the notifications of one piece of work in a row.
#[derive(Default, PartialEq, Eq, Debug)]
pub(crate) struct ProgressNote {
    /// What the work is called: "Indexing", "Fetching metadata".
    pub(crate) title: Option<String>,
    /// The line under the title: which crate is being read, how many files are left.
    pub(crate) message: Option<String>,
    /// How far through, 0 to 100.
    pub(crate) percentage: Option<u8>,
}

pub(crate) fn progress_note(message: &Value) -> ProgressNote {
    let Some(value) = message.pointer("/params/value") else {
        return ProgressNote::default();
    };
    ProgressNote {
        title: text_at(value, "title"),
        message: text_at(value, "message"),
        // The protocol calls this an unsigned integer, but it is a JSON number on the wire
        // and servers have sent it as a float, so it is read as one and rounded. Out of
        // range is held to the ends: a bar drawn past its own width is worse than a bar that
        // sits full while the last of the work finishes.
        percentage: value
            .get("percentage")
            .and_then(Value::as_f64)
            .map(|percentage| percentage.round().clamp(0.0, 100.0) as u8),
    }
}

/// One string field of a progress value, if it is there and is not empty. An empty title is
/// a title the bar would draw a blank space for.
fn text_at(value: &Value, field: &str) -> Option<String> {
    let text = value.get(field)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub(crate) fn did_open_params(uri: &str, language_id: &str, text: &str) -> Value {
    json!({
        "textDocument": {
            "uri": uri,
            "languageId": language_id,
            "version": 1,
            "text": text,
        }
    })
}

/// The whole text again. Full-text sync: the alternative is keeping a version number and a
/// list of edits in step with an editor that does not report them, and every server here
/// accepts the whole document.
pub(crate) fn did_change_params(uri: &str, text: &str) -> Value {
    json!({
        "textDocument": { "uri": uri, "version": 2 },
        "contentChanges": [{ "text": text }],
    })
}

pub(crate) fn did_close_params(uri: &str) -> Value {
    json!({ "textDocument": { "uri": uri } })
}

pub(crate) fn position_params(uri: &str, line: usize, character: u32) -> Value {
    json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
    })
}

/// The places a definition answer names, as the file pane opens them.
///
/// A definition outside the repo - a dependency's source, the standard library - keeps its
/// absolute path, because that is the only thing that names it. The pane decides whether it
/// can open one; making it a repo-relative path it is not would be a lie.
pub(crate) fn locations_from(
    answer: Value,
    repo_root: &std::path::Path,
) -> Result<Vec<LspLocation>> {
    if answer.is_null() {
        return Ok(Vec::new());
    }
    let response: GotoDefinitionResponse =
        serde_json::from_value(answer).context("the definition answer could not be read")?;
    let places: Vec<(String, u32)> = match response {
        GotoDefinitionResponse::Scalar(location) => vec![place_of(&location)],
        GotoDefinitionResponse::Array(locations) => locations.iter().map(place_of).collect(),
        GotoDefinitionResponse::Link(links) => links
            .iter()
            .map(|link| {
                (
                    link.target_uri.as_str().to_string(),
                    link.target_selection_range.start.line,
                )
            })
            .collect(),
    };

    Ok(places
        .into_iter()
        .filter_map(|(uri, line)| {
            Some(LspLocation {
                file_path: path_in_repo(&path_from_file_uri(&uri)?, repo_root),
                // The protocol counts lines from zero and the panes count from one.
                line_number: line as usize + 1,
            })
        })
        .collect())
}

fn place_of(location: &Location) -> (String, u32) {
    (location.uri.as_str().to_string(), location.range.start.line)
}

/// What a completion answer offers. `insertText` is what a server means to be typed and the
/// label is what it means to be read; where there is no insert text the two are the same.
pub(crate) fn completions_from(answer: Value) -> Result<Vec<LspCompletion>> {
    if answer.is_null() {
        return Ok(Vec::new());
    }
    let response: CompletionResponse =
        serde_json::from_value(answer).context("the completion answer could not be read")?;
    let items = match response {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };
    Ok(items.iter().map(completion_of).collect())
}

fn completion_of(item: &CompletionItem) -> LspCompletion {
    LspCompletion {
        insert: item
            .insert_text
            .clone()
            .unwrap_or_else(|| item.label.clone()),
        label: item.label.clone(),
        detail: item.detail.clone(),
    }
}

/// The column the server wants, from the byte column the editor gives, against the line it
/// falls on.
///
/// This is the whole of the encoding question, in one function, on purpose: it is the bug
/// every LSP client has, it is invisible in a file of ASCII, and the only defence is that
/// there is one place to get it right. A column past the end of the line is clamped to the
/// end, which is what the protocol says to do with one.
pub(crate) fn lsp_character(line: &str, byte_column: usize, encoding: PositionEncoding) -> u32 {
    let byte_column = byte_column.min(line.len());
    // A column landing inside a character belongs to that character's start.
    let cut = (0..=byte_column)
        .rev()
        .find(|index| line.is_char_boundary(*index))
        .unwrap_or(0);
    match encoding {
        PositionEncoding::Utf8 => cut as u32,
        PositionEncoding::Utf16 => line[..cut].encode_utf16().count() as u32,
    }
}

/// One line of a document, by its number counted from zero. `None` says the position is
/// past the end of the file, which is a caller working from a stale copy of it.
pub(crate) fn line_of(text: &str, line: usize) -> Option<&str> {
    text.split('\n')
        .nth(line)
        .map(|line| line.trim_end_matches('\r'))
}

/// The position a request goes out with, in the server's own units.
pub(crate) fn position_in(text: &str, at: &LspPosition, encoding: PositionEncoding) -> Result<u32> {
    let Some(line) = line_of(text, at.line) else {
        bail!("line {} is past the end of the file", at.line + 1);
    };
    Ok(lsp_character(line, at.column, encoding))
}

/// A path as a `file://` URI. Everything outside the unreserved set is escaped, so a repo
/// under a folder with a space or an accent in its name is named correctly.
pub(crate) fn file_uri(path: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                uri.push(*byte as char);
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The path a `file://` URI names. `None` for anything else - a server may answer with a
/// `jdt:` or `untitled:` URI, which names nothing on disk.
pub(crate) fn path_from_file_uri(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///path` has an empty authority; anything else is a host we cannot open.
    let path = rest.strip_prefix('/').map(|path| format!("/{path}"))?;
    let mut decoded = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    Some(std::path::PathBuf::from(String::from_utf8(decoded).ok()?))
}

/// A path as the panes name files: relative to the repo when it is inside it, and as it
/// stands when it is not.
fn path_in_repo(path: &std::path::Path, repo_root: &std::path::Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}
