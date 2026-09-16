//! What in a Codex answer is a visualization, and which file it is - by Codex's own rules.
//!
//! This is `codex-rs/tui/src/inline_visualization.rs` without the terminal half: the same two
//! prefixes, the same reading of a directive line, the same lines left alone inside code
//! blocks, and the same checks on the file a directive names. Where Codex's TUI turns a
//! directive into a link to a viewer page, moon hands back the fragment's path and shows it in
//! a pane. Kept close to Codex on purpose: a model writes what Codex taught it, so a directive
//! Codex would show is one moon shows, and one Codex would call unavailable moon ignores.

use std::{
    borrow::Cow,
    fs,
    ops::Range,
    path::{Component, Path, PathBuf},
};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use uuid::Uuid;

pub(crate) const DIRECTIVE_PREFIX: &str = "::codex-inline-vis{";
pub(crate) const CONTENT_REFERENCE_PREFIX: &str = "\u{e200}visualize\u{e202}";
pub(crate) const CONTENT_REFERENCE_SUFFIX: char = '\u{e201}';
/// The largest fragment Codex shows, and so the largest moon does.
pub(crate) const MAX_FRAGMENT_BYTES: u64 = 2 * 1024 * 1024;

/// Where one Codex thread's visualizations are written: its own folder, under
/// `visualizations/` in the Codex home, filed by the UTC day the thread was started on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ThreadVisualizations {
    visualizations_dir: PathBuf,
    thread_dir: PathBuf,
}

impl ThreadVisualizations {
    /// The folder of the thread with this id. `None` for an id that is not a UUID carrying a
    /// timestamp - every thread id Codex makes is a UUIDv7 - or a Codex home that is not there.
    pub(crate) fn new(codex_home: &Path, thread_id: &str) -> Option<Self> {
        let codex_home = fs::canonicalize(codex_home).ok()?;
        let uuid = Uuid::parse_str(thread_id).ok()?;
        let (seconds, _) = uuid.get_timestamp()?.to_unix();
        let created_at =
            time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).ok()?).ok()?;
        let visualizations_dir = codex_home.join("visualizations");
        let thread_dir = visualizations_dir
            .join(format!(
                "{:04}/{:02}/{:02}",
                created_at.year(),
                u8::from(created_at.month()),
                created_at.day()
            ))
            .join(thread_id);
        Some(Self {
            visualizations_dir,
            thread_dir,
        })
    }

    #[cfg(test)]
    pub(crate) fn thread_dir(&self) -> &Path {
        &self.thread_dir
    }

    /// The fragment a directive's file stands for, if Codex would show it: a single `.html`
    /// name inside the thread's folder - or an absolute path to one - that is a file of at
    /// most [`MAX_FRAGMENT_BYTES`], reached without leaving the folder through a link.
    pub(crate) fn fragment_for(&self, file: &str) -> Option<PathBuf> {
        let path = Path::new(file);
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.thread_dir).ok()?
        } else {
            path
        };
        if relative
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("html")
            || !matches!(
                relative.components().collect::<Vec<_>>().as_slice(),
                [Component::Normal(_)]
            )
        {
            return None;
        }

        let visualizations_dir = fs::canonicalize(&self.visualizations_dir).ok()?;
        let thread_dir = fs::canonicalize(&self.thread_dir).ok()?;
        if !thread_dir.starts_with(&visualizations_dir) {
            return None;
        }
        let fragment_path = fs::canonicalize(thread_dir.join(relative)).ok()?;
        if !fragment_path.starts_with(&thread_dir) {
            return None;
        }
        let metadata = fragment_path.metadata().ok()?;
        (metadata.is_file() && metadata.len() <= MAX_FRAGMENT_BYTES).then_some(fragment_path)
    }
}

/// Whether a path is a fragment some thread's folder holds, as [`ThreadVisualizations::fragment_for`]
/// would have handed it back. What a page is only ever built from: the path arrives from the
/// window, and a server must not read whatever file it is asked for.
pub(crate) fn is_thread_fragment(codex_home: &Path, fragment_path: &Path) -> bool {
    let (Ok(visualizations_dir), Ok(fragment_path)) = (
        fs::canonicalize(codex_home.join("visualizations")),
        fs::canonicalize(fragment_path),
    ) else {
        return false;
    };
    fragment_path
        .extension()
        .is_some_and(|extension| extension == "html")
        && fragment_path
            .parent()
            .is_some_and(|thread_dir| is_visualization_thread_dir(&visualizations_dir, thread_dir))
        && fragment_path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_FRAGMENT_BYTES)
}

/// `YYYY/MM/DD/<uuid>` under the visualizations folder.
fn is_visualization_thread_dir(visualizations_dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(visualizations_dir) else {
        return false;
    };
    let components = relative.components().collect::<Vec<_>>();
    matches!(
        components.as_slice(),
        [
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(thread_id)
        ] if Uuid::parse_str(&thread_id.to_string_lossy()).is_ok()
    )
}

pub(crate) fn contains_inline_visualization(markdown: &str) -> bool {
    markdown.contains(DIRECTIVE_PREFIX) || markdown.contains(CONTENT_REFERENCE_PREFIX)
}

/// The file every complete directive of an answer names, in the order they are written.
///
/// A directive is a line of its own, outside any code block, starting with one of the two
/// prefixes. A line that starts like one and does not parse - half streamed, or a path that is
/// not absolute - names nothing, as Codex shows nothing for it.
pub(crate) fn directive_files(markdown: &str) -> Vec<String> {
    if !contains_inline_visualization(markdown) {
        return Vec::new();
    }

    let mut code_block_ranges = Vec::<Range<usize>>::new();
    let mut code_block_start = None;
    for (event, range) in Parser::new_ext(markdown, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    code_block_ranges.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    if let Some(start) = code_block_start {
        code_block_ranges.push(start..markdown.len());
    }

    let mut files = Vec::new();
    let mut source_offset = 0;
    for source_line in markdown.split_inclusive('\n') {
        let line_start = source_offset;
        source_offset += source_line.len();
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let trimmed = line.trim();
        let is_code = code_block_ranges
            .iter()
            .any(|range| range.start < source_offset && line_start < range.end);
        if is_code
            || (!trimmed.starts_with(DIRECTIVE_PREFIX)
                && !trimmed.starts_with(CONTENT_REFERENCE_PREFIX))
        {
            continue;
        }
        if let Some(file) = parse_directive_file(trimmed) {
            files.push(file.into_owned());
        }
    }
    files
}

fn parse_directive_file(directive: &str) -> Option<Cow<'_, str>> {
    if let Some(attributes) = directive.strip_prefix(DIRECTIVE_PREFIX) {
        let attributes = attributes.strip_suffix('}')?.trim();
        let value = attributes.strip_prefix("file=\"")?.strip_suffix('"')?;
        return (!value.is_empty() && !value.contains('"')).then_some(Cow::Borrowed(value));
    }

    let payload = directive
        .strip_prefix(CONTENT_REFERENCE_PREFIX)?
        .strip_suffix(CONTENT_REFERENCE_SUFFIX)?;
    let payload = serde_json::from_str::<serde_json::Value>(payload).ok()?;
    let path = payload.get("path")?.as_str()?;
    Path::new(path)
        .is_absolute()
        .then(|| Cow::Owned(path.to_string()))
}

#[cfg(test)]
#[path = "directives_tests.rs"]
mod tests;
