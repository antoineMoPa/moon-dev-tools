//! What can be checked without a language server, and one test that starts a real one.
//!
//! Everything but the last two is arithmetic and tables, so it runs in the normal suite.
//! The last two start a real server and wait for it to index, which is seconds to minutes
//! and depends on what is installed on the machine, so they are `#[ignore]`d and run by
//! hand.

use std::time::{Duration, Instant};

use serde_json::json;

use super::{
    framing::{self, Frames},
    languages,
    process::{PositionEncoding, Working, follow_progress},
    protocol,
};
use crate::api::LspStatus;

#[test]
fn a_message_goes_out_with_its_length_in_bytes_ahead_of_it() {
    assert_eq!(
        String::from_utf8(framing::frame(r#"{"id":1}"#)).expect("expected a text frame"),
        "Content-Length: 8\r\n\r\n{\"id\":1}"
    );
}

#[test]
fn a_frame_length_counts_bytes_rather_than_characters() {
    let framed = String::from_utf8(framing::frame(r#"{"a":"é"}"#)).expect("expected a text frame");
    assert!(
        framed.starts_with("Content-Length: 10\r\n"),
        "expected the two bytes of é to count twice, got {framed}"
    );
}

#[test]
fn a_message_split_across_reads_is_read_out_once_all_of_it_has_arrived() {
    let framed = framing::frame(r#"{"id":7,"result":null}"#);
    let (front, back) = framed.split_at(12);

    let mut frames = Frames::default();
    frames.push(front);
    assert!(
        frames.next_message().is_none(),
        "half a message is not a message"
    );
    frames.push(back);
    assert_eq!(
        frames.next_message().expect("expected the whole message"),
        r#"{"id":7,"result":null}"#
    );
    assert!(
        frames.next_message().is_none(),
        "there was only one message"
    );
}

#[test]
fn two_messages_arriving_together_are_read_out_in_order() {
    let mut arrived = framing::frame(r#"{"id":1}"#);
    arrived.extend_from_slice(&framing::frame(r#"{"id":2}"#));

    let mut frames = Frames::default();
    frames.push(&arrived);
    assert_eq!(frames.next_message().as_deref(), Some(r#"{"id":1}"#));
    assert_eq!(frames.next_message().as_deref(), Some(r#"{"id":2}"#));
    assert_eq!(frames.next_message(), None);
}

#[test]
fn a_header_with_a_content_type_beside_the_length_is_still_read() {
    let mut frames = Frames::default();
    frames.push(b"Content-Type: application/vscode-jsonrpc\r\nContent-Length: 8\r\n\r\n{\"id\":1}");
    assert_eq!(frames.next_message().as_deref(), Some(r#"{"id":1}"#));
}

/// The one that matters. `héllo` is five characters, six bytes and five UTF-16 units, so a
/// column past the accent is a different number depending on what is counting - and a client
/// that hands a UTF-16 server its byte columns points at the wrong place on every line
/// holding anything but ASCII.
#[test]
fn a_column_past_an_accent_is_counted_in_the_units_the_server_agreed_to() {
    let line = "let héllo = 1;";
    // The byte column of the `=`: `let ` is 4, `héllo` is 6 bytes, the space is one.
    let byte_column = 11;
    assert_eq!(&line[byte_column..byte_column + 1], "=");

    assert_eq!(
        protocol::lsp_character(line, byte_column, PositionEncoding::Utf8),
        11,
        "a server counting bytes wants the column unchanged"
    );
    assert_eq!(
        protocol::lsp_character(line, byte_column, PositionEncoding::Utf16),
        10,
        "the two bytes of é are one UTF-16 unit"
    );
}

#[test]
fn a_column_landing_inside_a_character_belongs_to_the_start_of_it() {
    let line = "héllo";
    // Between the two bytes of é, which is not a place a character starts.
    assert_eq!(protocol::lsp_character(line, 2, PositionEncoding::Utf16), 1);
    assert_eq!(protocol::lsp_character(line, 2, PositionEncoding::Utf8), 1);
}

#[test]
fn a_column_past_the_end_of_a_line_is_the_end_of_it() {
    assert_eq!(
        protocol::lsp_character("ab", 99, PositionEncoding::Utf16),
        2
    );
}

#[test]
fn an_emoji_is_two_utf16_units_and_four_bytes() {
    let line = "x = \"🌙\";";
    let after_the_moon = 4 + 1 + "🌙".len();
    assert_eq!(
        protocol::lsp_character(line, after_the_moon, PositionEncoding::Utf16),
        7,
        "five ASCII characters and the two units of the moon"
    );
    assert_eq!(
        protocol::lsp_character(line, after_the_moon, PositionEncoding::Utf8),
        9
    );
}

#[test]
fn a_position_is_taken_against_the_line_it_falls_on() {
    let text = "fn main() {}\nlet héllo = 1;\n";
    let at = crate::api::LspPosition {
        line: 1,
        column: 11,
    };
    assert_eq!(
        protocol::position_in(text, &at, PositionEncoding::Utf16)
            .expect("expected a position on the second line"),
        10
    );
}

#[test]
fn a_position_past_the_end_of_the_file_is_refused() {
    let at = crate::api::LspPosition { line: 9, column: 0 };
    let error = protocol::position_in("one line\n", &at, PositionEncoding::Utf8)
        .expect_err("expected a line past the end to be refused");
    assert!(error.to_string().contains("past the end"), "{error}");
}

/// A `$/progress` as rust-analyzer really sends one: the token names the work, and what is
/// worth reading is inside `value`.
#[test]
fn a_progress_notification_says_what_the_server_is_doing_and_how_far_through_it_is() {
    let note = protocol::progress_note(&json!({
        "jsonrpc": "2.0",
        "method": "$/progress",
        "params": {
            "token": "rustAnalyzer/Indexing",
            "value": {
                "kind": "report",
                "message": "12/57 (serde)",
                "percentage": 21,
            },
        },
    }));

    assert_eq!(note.title, None, "only a begin carries a title");
    assert_eq!(note.message.as_deref(), Some("12/57 (serde)"));
    assert_eq!(note.percentage, Some(21));
}

/// The ordinary case, and the one the status bar has to read well: a server that says what
/// it is doing and nothing at all about how long it will take.
#[test]
fn a_piece_of_work_that_reports_no_percentage_still_says_what_it_is_doing() {
    let note = protocol::progress_note(&json!({
        "jsonrpc": "2.0",
        "method": "$/progress",
        "params": {
            "token": "rustAnalyzer/Fetching",
            "value": { "kind": "begin", "title": "Fetching", "cancellable": true },
        },
    }));

    assert_eq!(note.title.as_deref(), Some("Fetching"));
    assert_eq!(note.message, None);
    assert_eq!(note.percentage, None, "nothing said is not nought per cent");
}

/// The title only ever comes with the `begin`, so the reports that follow have to be read
/// against the work already in hand rather than on their own.
#[test]
fn a_report_keeps_the_title_its_begin_gave_it_and_an_end_leaves_the_server_doing_nothing() {
    let notification = |value: serde_json::Value| {
        json!({ "jsonrpc": "2.0", "method": "$/progress", "params": { "token": "t", "value": value } })
    };
    let mut working: Option<Working> = None;

    let begun = notification(json!({ "kind": "begin", "title": "Indexing", "percentage": 0 }));
    follow_progress(
        &mut working,
        protocol::progress_kind(&begun),
        protocol::progress_note(&begun),
    );
    assert_eq!(
        working,
        Some(Working {
            title: "Indexing".to_string(),
            detail: None,
            percentage: Some(0),
        })
    );

    let reported = notification(json!({ "kind": "report", "message": "12/57", "percentage": 21 }));
    follow_progress(
        &mut working,
        protocol::progress_kind(&reported),
        protocol::progress_note(&reported),
    );
    assert_eq!(
        working,
        Some(Working {
            title: "Indexing".to_string(),
            detail: Some("12/57".to_string()),
            percentage: Some(21),
        })
    );

    let ended = notification(json!({ "kind": "end", "message": "57/57" }));
    follow_progress(
        &mut working,
        protocol::progress_kind(&ended),
        protocol::progress_note(&ended),
    );
    assert_eq!(working, None, "work that has ended is not work");
}

/// A report with no begin behind it is a piece of work this client never saw start, and
/// making a nameless line out of it would be the window inventing what a server is doing.
#[test]
fn a_report_with_no_work_behind_it_is_left_alone() {
    let reported = json!({
        "jsonrpc": "2.0",
        "method": "$/progress",
        "params": { "token": "t", "value": { "kind": "report", "message": "12/57" } },
    });
    let mut working: Option<Working> = None;

    follow_progress(
        &mut working,
        protocol::progress_kind(&reported),
        protocol::progress_note(&reported),
    );

    assert_eq!(working, None);
}

#[test]
fn a_rust_file_is_served_by_rust_analyzer() {
    let language = languages::for_file("src/main.rs").expect("expected rust to be in the table");
    assert_eq!(language.language_id, "rust");
    assert_eq!(language.server.command, "rust-analyzer");
    assert!(language.server.args.is_empty());
}

#[test]
fn typescript_and_its_react_flavour_are_two_languages_on_one_server() {
    let typescript = languages::for_file("app.ts").expect("expected .ts to be in the table");
    let react = languages::for_file("app.tsx").expect("expected .tsx to be in the table");

    assert_eq!(typescript.language_id, "typescript");
    assert_eq!(react.language_id, "typescriptreact");
    assert_eq!(typescript.server.command, "typescript-language-server");
    assert_eq!(typescript.server.args, &["--stdio"]);
    assert_eq!(
        typescript.server.name, react.server.name,
        "one project's .ts and .tsx are indexed by one server"
    );
}

#[test]
fn an_extension_nothing_serves_has_no_language_behind_it() {
    assert!(languages::for_file("README.md").is_none());
    assert!(languages::for_file("Makefile").is_none());
}

#[test]
fn a_language_whose_server_is_not_installed_reads_as_unavailable() {
    let python = languages::for_file("main.py").expect("expected python to be in the table");
    assert_eq!(
        super::status_without_a_server(Some(python), false),
        LspStatus::Unavailable,
        "a server that is not installed is no server at all"
    );
    assert_eq!(
        super::status_without_a_server(None, false),
        LspStatus::Unavailable,
        "and neither is an extension nothing serves"
    );
    assert_eq!(
        super::status_without_a_server(Some(python), true),
        LspStatus::Starting,
        "an installed server has at least started"
    );
}

/// How this case was found: a `rust-analyzer` on PATH that is really a rustup shim for a
/// component nobody added exits the moment it is spoken to. Being on PATH is not the same as
/// running, and a file behind a server like that must not read as forever starting.
#[test]
fn a_server_that_is_installed_and_will_not_start_is_given_up_on() {
    let registry = super::LspRegistry::new();
    let key = ("session".to_string(), languages::RUST_ANALYZER.name);
    assert!(!registry.gave_up_on(&key));

    registry.give_up_on(&key);
    assert!(registry.gave_up_on(&key));
    assert_eq!(
        super::status_without_a_server(languages::for_file("main.rs"), false),
        LspStatus::Unavailable,
        "a server that would not start reads the same as one that is not there"
    );
}

#[test]
fn a_server_that_asked_for_utf8_and_was_answered_with_nothing_is_taken_to_count_utf16() {
    let silent = json!({ "capabilities": {} });
    assert_eq!(
        protocol::agreed_encoding(&silent).expect("expected a readable reply"),
        PositionEncoding::Utf16,
        "the protocol's default is what a server that agreed to nothing means"
    );

    let agreed = json!({ "capabilities": { "positionEncoding": "utf-8" } });
    assert_eq!(
        protocol::agreed_encoding(&agreed).expect("expected a readable reply"),
        PositionEncoding::Utf8
    );
}

#[test]
fn a_definition_is_read_as_a_repo_path_and_a_line_counted_from_one() {
    let repo_root = std::path::Path::new("/tmp/a repo");
    let answer = json!({
        "uri": "file:///tmp/a%20repo/src/main.rs",
        "range": {
            "start": { "line": 41, "character": 3 },
            "end": { "line": 41, "character": 8 },
        },
    });

    let locations =
        protocol::locations_from(answer, repo_root).expect("expected a readable definition");
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].file_path, "src/main.rs");
    assert_eq!(
        locations[0].line_number, 42,
        "the protocol counts lines from zero and the panes from one"
    );
}

#[test]
fn a_definition_outside_the_repo_keeps_the_path_that_names_it() {
    let answer = json!([{
        "uri": "file:///home/dev/.cargo/registry/serde/lib.rs",
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 1 },
        },
    }]);
    let locations = protocol::locations_from(answer, std::path::Path::new("/home/dev/repo"))
        .expect("expected a readable definition");
    assert_eq!(
        locations[0].file_path,
        "/home/dev/.cargo/registry/serde/lib.rs"
    );
}

#[test]
fn a_file_uri_escapes_what_a_path_may_hold_and_reads_back_the_same() {
    let path = std::path::Path::new("/tmp/a repo/src/café.rs");
    let uri = protocol::file_uri(path);
    assert_eq!(uri, "file:///tmp/a%20repo/src/caf%C3%A9.rs");
    assert_eq!(
        protocol::path_from_file_uri(&uri).expect("expected the path back"),
        path
    );
}

#[test]
fn a_uri_that_names_nothing_on_disk_is_no_place_to_open() {
    assert!(protocol::path_from_file_uri("untitled:Untitled-1").is_none());
    assert!(protocol::path_from_file_uri("jdt://contents/rt.jar").is_none());
}

#[test]
fn a_server_asking_for_its_configuration_is_answered_one_entry_per_item() {
    let asked = json!({
        "id": 3,
        "method": "workspace/configuration",
        "params": { "items": [{ "section": "typescript" }, { "section": "javascript" }] },
    });
    let reply = protocol::reply_to_server_request("workspace/configuration", &asked);
    assert_eq!(
        reply.as_array().map(Vec::len),
        Some(2),
        "a server handed the wrong number of settings stops asking"
    );
    assert!(protocol::reply_to_server_request("client/registerCapability", &asked).is_null());
}

/// The only proof the whole path works: a real server, a real project, and the place a
/// symbol is actually defined.
///
/// `#[ignore]`d because it starts `typescript-language-server` and waits for it to be
/// ready, which is seconds rather than milliseconds and depends on what is installed:
/// `cargo test -- --ignored --test-threads=1 a_real_language_server_says_where_a_symbol_is_defined`.
#[test]
#[ignore]
fn a_real_language_server_says_where_a_symbol_is_defined() {
    let root = fixture_root("typescript");
    let source = "export function greet(name: string): string {\n    return `hi ${name}`;\n}\n\nconst message = greet(\"world\");\nconsole.log(message);\n";
    std::fs::write(root.join("main.ts"), source).expect("failed to write the fixture file");

    let state =
        crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(Instant::now())));
    let session_id = session_on(&state, &root);
    super::did_open(&state, &session_id, "main.ts", source).expect("failed to open the document");
    wait_until_ready(&state, &session_id, "main.ts", Duration::from_secs(60));

    // The `greet` of `const message = greet("world")`, on the fifth line.
    let at = crate::api::LspPosition {
        line: 4,
        column: 18,
    };
    let locations = super::definition(&state, &session_id, "main.ts", at)
        .expect("failed to ask where greet is defined");
    println!(
        "definition: {:?}",
        locations
            .iter()
            .map(|location| (&location.file_path, location.line_number))
            .collect::<Vec<_>>()
    );
    assert_eq!(locations.len(), 1, "expected one definition");
    assert_eq!(locations[0].file_path, "main.ts");
    assert_eq!(
        locations[0].line_number, 1,
        "greet is defined on the first line"
    );

    let completions = super::completion(&state, &session_id, "main.ts", at)
        .expect("failed to ask what could be typed");
    println!("completions: {}", completions.len());
    assert!(
        completions
            .iter()
            .any(|completion| completion.label == "greet"),
        "expected greet among what can be typed here"
    );

    super::did_close(&state, &session_id, "main.ts").expect("failed to close the document");
    let _ = std::fs::remove_dir_all(&root);
}

/// Somewhere throwaway to put a fixture repo, one directory per test so two of these can be
/// run in the same process without treading on each other.
fn fixture_root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("moonreview-lsp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("failed to create the fixture directory");
    crate::git::run_git_no_output(&root, &["init"]).expect("failed to init the fixture repo");
    root
}

/// Open a review on a fixture, the way the window opens one.
fn session_on(state: &crate::api::AppState, root: &std::path::Path) -> String {
    crate::service::open_session(
        state,
        crate::api::OpenSessionRequest {
            repo_path: root.display().to_string(),
            diff_target: None,
            active_commit: None,
        },
    )
    .expect("failed to open a session on the fixture")
    .session_id
}

/// Wait for a server to finish starting, printing what it says on the way so the
/// Starting -> Ready transition is visible in the test's output.
fn wait_until_ready(
    state: &crate::api::AppState,
    session_id: &str,
    file_path: &str,
    deadline: Duration,
) {
    let giving_up_at = Instant::now() + deadline;
    while Instant::now() < giving_up_at {
        let status = super::status(state, session_id, file_path).expect("failed to read status");
        println!("status: {status:?}");
        assert_ne!(
            status,
            LspStatus::Unavailable,
            "the server for {file_path} has to be installed to run this test"
        );
        if status == LspStatus::Ready {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("the server was still starting after {deadline:?}");
}

/// The other half of the proof: a server that agreed to count columns in bytes, and a
/// position with several non-ASCII characters to the left of the name being asked about.
///
/// rust-analyzer answers `initialize` with `positionEncoding: "utf-8"`, which is the branch
/// the typescript test cannot reach - that server ignores the request and is converted for.
/// And the five `ñ` before the call put its byte column five ahead of its UTF-16 column, so
/// a conversion applied the wrong way round lands on a different token and the definition
/// comes back as something else or as nothing, rather than passing by luck.
///
/// `#[ignore]`d, and slower than the typescript one: rust-analyzer reads the manifest and
/// indexes the crate before it is ready. Run it with
/// `cargo test --lib -- --ignored --test-threads=1 --nocapture a_utf8_server`.
#[test]
#[ignore]
fn a_utf8_server_resolves_a_position_with_accents_to_the_left_of_it() {
    let root = fixture_root("rust");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("failed to write the fixture manifest");
    std::fs::create_dir_all(root.join("src")).expect("failed to create the fixture source folder");

    let source = "pub fn café_size() -> u32 { 3 }\npub fn total() -> u32 { let ñññññ = 0; café_size() + ñññññ }\n";
    std::fs::write(root.join("src/lib.rs"), source).expect("failed to write the fixture file");

    let state =
        crate::server::build_state(std::sync::Arc::new(std::sync::Mutex::new(Instant::now())));
    let session_id = session_on(&state, &root);
    super::did_open(&state, &session_id, "src/lib.rs", source)
        .expect("failed to open the document");

    // A minute is not enough on a cold cargo registry; this waits for the indexing rather
    // than for the initialize reply, which comes back long before the answers are any good.
    wait_until_ready(&state, &session_id, "src/lib.rs", Duration::from_secs(300));

    let encoding = state
        .lsp
        .agreed_encoding(&session_id, languages::RUST_ANALYZER.name)
        .expect("expected a running rust server");
    println!("agreed encoding: {encoding:?}");
    assert_eq!(
        encoding,
        PositionEncoding::Utf8,
        "rust-analyzer agrees to count columns in bytes, which is the branch this test is for"
    );

    // The call on the second line. The column is worked out rather than written down: the
    // point of the test is that the two counts differ, so a number in the source would say
    // nothing about which of them it is.
    let call_site = source.lines().nth(1).expect("expected a second line");
    let byte_column = call_site
        .find("café_size() + ")
        .expect("expected the call on the second line")
        + 1;
    let utf16_column = call_site[..byte_column].encode_utf16().count();
    assert_eq!(
        byte_column - utf16_column,
        5,
        "the five ñ to the left take two bytes and one UTF-16 unit each, so the two counts \
         are five apart at the call - which is what makes getting this backwards visible"
    );
    println!("byte column {byte_column}, utf-16 column {utf16_column}");

    let locations = super::definition(
        &state,
        &session_id,
        "src/lib.rs",
        crate::api::LspPosition {
            line: 1,
            column: byte_column,
        },
    )
    .expect("failed to ask where café_size is defined");
    println!(
        "definition: {:?}",
        locations
            .iter()
            .map(|location| (&location.file_path, location.line_number))
            .collect::<Vec<_>>()
    );

    assert_eq!(locations.len(), 1, "expected one definition");
    assert_eq!(locations[0].file_path, "src/lib.rs");
    assert_eq!(
        locations[0].line_number, 1,
        "café_size is defined on the first line"
    );

    super::did_close(&state, &session_id, "src/lib.rs").expect("failed to close the document");
    let _ = std::fs::remove_dir_all(&root);
}
