//! Ported from `codex-rs/tui/src/inline_visualization_tests.rs`: the cases that are about
//! what a directive is and which files it may name, with Codex's "unavailable" fallback read
//! as naming no fragment.

use super::*;

/// A thread started on 2026-09-16 at 15:08:29 UTC.
const THREAD_ID: &str = "01a0aac3-1f62-7012-bcea-58307a8d2586";

/// A Codex home of its own, with this thread's folder made and one fragment written in it.
fn context_with_fragment(fragment: &str) -> (PathBuf, ThreadVisualizations) {
    let codex_home = std::env::temp_dir().join(format!(
        "moon-codex-home-{}",
        crate::moontasks::store::new_uuid()
    ));
    fs::create_dir_all(&codex_home).expect("create codex home");
    let context = ThreadVisualizations::new(&codex_home, THREAD_ID)
        .expect("UUIDv7 thread id should provide a timestamp");
    fs::create_dir_all(context.thread_dir()).expect("create visualization directory");
    fs::write(context.thread_dir().join("chart.html"), fragment).expect("write fragment");
    (codex_home, context)
}

#[test]
fn thread_dir_is_filed_by_the_utc_day_of_the_thread_id() {
    let (codex_home, context) = context_with_fragment("<div>chart</div>");

    assert_eq!(
        context.thread_dir(),
        fs::canonicalize(&codex_home)
            .unwrap()
            .join("visualizations/2026/09/16")
            .join(THREAD_ID)
    );
}

#[test]
fn a_thread_id_without_a_timestamp_has_no_folder() {
    let (codex_home, _) = context_with_fragment("<div>chart</div>");

    // A version 4 UUID carries no time.
    assert_eq!(
        ThreadVisualizations::new(&codex_home, "0f7c1a52-4f0e-4f6d-9a57-3b1c1f0e2d11"),
        None
    );
    assert_eq!(ThreadVisualizations::new(&codex_home, "not-a-uuid"), None);
}

#[test]
fn rewrites_complete_directive_to_trusted_static_file_placeholder() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");

    let files = directive_files("Before\n::codex-inline-vis{file=\"chart.html\"}\nAfter");

    assert_eq!(files, ["chart.html"]);
    assert_eq!(
        context.fragment_for(&files[0]),
        Some(context.thread_dir().join("chart.html"))
    );
}

#[test]
fn hides_incomplete_streaming_directive() {
    assert!(directive_files("Before\n::codex-inline-vis{file=\"chart").is_empty());
}

#[test]
fn hides_incomplete_streaming_content_reference() {
    for reference in [
        "Before\n\u{e200}visualize\u{e202}{\"path\":\"/tmp/chart",
        "Before\n\u{e200}visualize\u{e202}{\"path\":\"/tmp/chart.html\"}",
    ] {
        assert!(directive_files(reference).is_empty());
    }
}

#[test]
fn unavailable_or_invalid_content_reference_has_explicit_fallback() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");
    let outside = std::env::temp_dir().join(format!(
        "moon-outside-{}",
        crate::moontasks::store::new_uuid()
    ));
    fs::create_dir_all(&outside).expect("outside visualization directory");
    let outside_path = outside.join("chart.html");
    fs::write(&outside_path, "<div>outside</div>").expect("write outside fragment");

    let outside_reference = format!(
        "\u{e200}visualize\u{e202}{}\u{e201}",
        serde_json::json!({ "path": outside_path })
    );
    let files = directive_files(&outside_reference);
    assert_eq!(files.len(), 1);
    assert_eq!(context.fragment_for(&files[0]), None);

    for payload in [
        serde_json::json!({ "path": "chart.html" }).to_string(),
        "{\"path\":".to_string(),
    ] {
        let reference = format!("\u{e200}visualize\u{e202}{payload}\u{e201}");
        assert!(directive_files(&reference).is_empty());
    }
}

#[test]
fn unavailable_artifact_has_explicit_fallback() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");

    assert_eq!(context.fragment_for("missing.html"), None);
}

#[test]
fn rejects_parent_path_and_non_html_file() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");
    fs::write(context.thread_dir().join("chart.svg"), "<svg/>").expect("write svg");

    for file in ["../chart.html", "chart.svg", "nested/chart.html"] {
        let files = directive_files(&format!("::codex-inline-vis{{file=\"{file}\"}}"));
        assert_eq!(files, [file]);
        assert_eq!(context.fragment_for(file), None, "{file}");
    }
}

#[test]
fn rejects_oversized_fragment() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");
    let fragment = fs::OpenOptions::new()
        .write(true)
        .open(context.thread_dir().join("chart.html"))
        .expect("open fragment");
    fragment
        .set_len(MAX_FRAGMENT_BYTES + 1)
        .expect("enlarge fragment");

    assert_eq!(context.fragment_for("chart.html"), None);
}

#[test]
fn finalized_agent_cell_replays_visualization_link() {
    let (_codex_home, context) = context_with_fragment("<div>chart</div>");
    let fragment_path = context.thread_dir().join("chart.html");

    let files = directive_files(&format!(
        "Before\n\n\u{e200}visualize\u{e202}{}\u{e201}\n\nAfter",
        serde_json::json!({ "path": fragment_path })
    ));

    assert_eq!(files.len(), 1);
    assert_eq!(context.fragment_for(&files[0]), Some(fragment_path));
}

#[test]
fn agent_code_blocks_preserve_visualization_directive_literals() {
    let files = directive_files(
        "Fenced:\n\n```text\n::codex-inline-vis{file=\"chart.html\"}\n```\n\nIndented:\n\n    ::codex-inline-vis{file=\"chart.html\"}",
    );

    assert!(files.is_empty());
}

#[test]
fn a_fragment_is_servable_only_from_a_thread_folder() {
    let (codex_home, context) = context_with_fragment("<div>chart</div>");
    let fragment_path = context.thread_dir().join("chart.html");

    assert!(is_thread_fragment(&codex_home, &fragment_path));
    assert!(!is_thread_fragment(
        &codex_home,
        &codex_home.join("config.toml")
    ));
    let loose = codex_home.join("visualizations/loose.html");
    fs::write(&loose, "<div>loose</div>").expect("write loose fragment");
    assert!(!is_thread_fragment(&codex_home, &loose));
}
