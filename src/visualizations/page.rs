//! The page a visualization is shown on, built from its fragment the way Codex builds its
//! viewer - `codex-rs/tui/src/inline_visualization/viewer.rs`.
//!
//! An outer page holding one iframe, sandboxed to scripts only - no same origin, so the
//! fragment cannot reach the page around it - whose `srcdoc` is the fragment inside Codex's
//! stylesheet and runtime, both under the same content security policy Codex sets. Codex writes
//! that page to a file and opens it in a browser; moon hands it to the pane's webview as a
//! string, so no file access rules come into it.

use std::path::Path;

use anyhow::{Context, Result};

use super::directives::MAX_FRAGMENT_BYTES;

const FRAGMENT_PLACEHOLDER: &str = "<!--__INLINE_VISUALIZATION_FRAGMENT__-->";

const VIEWER_STYLESHEET: &str = include_str!("../../assets/codex_visualization/visualize.css");
const VIEWER_RUNTIME: &str = include_str!("../../assets/codex_visualization/visualize.html");

const FRAME_CSP: &str = "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob: data: https://cdnjs.cloudflare.com https://cdn.jsdelivr.net https://esm.sh https://fonts.bunny.net https://fonts.googleapis.com https://fonts.gstatic.com https://unpkg.com; style-src 'unsafe-inline' blob: data: https://cdnjs.cloudflare.com https://cdn.jsdelivr.net https://esm.sh https://fonts.bunny.net https://fonts.googleapis.com https://fonts.gstatic.com https://unpkg.com; img-src blob: data: https://cdnjs.cloudflare.com https://cdn.jsdelivr.net https://esm.sh https://fonts.bunny.net https://fonts.googleapis.com https://fonts.gstatic.com https://unpkg.com; font-src blob: data: https://cdnjs.cloudflare.com https://cdn.jsdelivr.net https://esm.sh https://fonts.bunny.net https://fonts.googleapis.com https://fonts.gstatic.com https://unpkg.com; media-src blob: data:; worker-src blob:; connect-src blob: data:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const SHELL_STYLE: &str = ":root{color-scheme:light dark;background:light-dark(rgb(255 255 255), rgb(24 24 24))}html,body{margin:0}body{box-sizing:border-box;padding:1rem;background:inherit}iframe{display:block;width:100%;max-width:736px;height:calc(100vh - 2rem);margin:0 auto;border:0}";

/// The page for the fragment at this path, titled by its file name as Codex titles it.
///
/// The fragment has already been checked to be a thread's - see
/// [`super::directives::is_thread_fragment`] - but it is read again here, and may have grown
/// past the limit since.
pub(crate) fn page_for(fragment_path: &Path) -> Result<String> {
    let metadata = fragment_path
        .metadata()
        .with_context(|| format!("could not read {}", fragment_path.display()))?;
    anyhow::ensure!(
        metadata.len() <= MAX_FRAGMENT_BYTES,
        "{} is larger than a visualization may be",
        fragment_path.display()
    );
    let fragment = std::fs::read_to_string(fragment_path)
        .with_context(|| format!("could not read {}", fragment_path.display()))?;
    Ok(render_fragment(&fragment, &title_of(fragment_path)))
}

/// What a visualization is called: its file's stem, with dashes read as spaces.
pub(crate) fn title_of(fragment_path: &Path) -> String {
    fragment_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Visualization")
        .replace('-', " ")
}

fn render_fragment(fragment: &str, title: &str) -> String {
    let runtime = VIEWER_RUNTIME.replacen(FRAGMENT_PLACEHOLDER, fragment, 1);
    let escaped_title = escape_html(title);
    let frame = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{FRAME_CSP}\"><title>{escaped_title}</title><style>{VIEWER_STYLESHEET}\nhtml>body{{padding:0}}</style></head><body>{runtime}</body></html>"
    );

    // A srcdoc frame inherits its parent's CSP, so the shell must grant every
    // resource type that the stricter frame CSP may use. The frame itself stays
    // sandboxed without allow-same-origin.
    let shell_csp = FRAME_CSP.replace("frame-src 'none'", "frame-src 'self'");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Content-Security-Policy\" content=\"{shell_csp}\"><title>{escaped_title}</title><style>{SHELL_STYLE}</style></head><body><iframe sandbox=\"allow-scripts\" referrerpolicy=\"no-referrer\" title=\"{escaped_title}\" srcdoc=\"{}\"></iframe></body></html>",
        escape_html(&frame)
    )
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `viewer_materializes_sandboxed_static_document`, from Codex: the page as a string rather
    /// than a file, with the same things checked in it.
    #[test]
    fn viewer_materializes_sandboxed_static_document() {
        let document = render_fragment(
            "<div id=\"widget\"><div class=\"viz-controls\">controls</div><canvas id=\"chart\"></canvas></div><script>globalThis.chartRendered = true;</script>",
            "chart",
        );

        assert!(document.contains("sandbox=\"allow-scripts\""));
        assert!(!document.contains("allow-same-origin"));
        assert!(document.contains("script-src 'unsafe-inline' 'unsafe-eval'"));
        assert!(document.contains(".viz-controls"));
        assert!(document.contains("https://unpkg.com/@floating-ui/dom@1.7.4"));
        assert!(document.contains("https://unpkg.com/lucide@1.17.0"));
        assert!(document.contains("&lt;canvas id=&quot;chart&quot;&gt;&lt;/canvas&gt;"));
        assert!(document.contains("globalThis.chartRendered = true"));
        assert!(document.contains("Content-Security-Policy"));
        // The shell may hold the frame; the frame may hold nothing.
        let (shell, frame) = document.split_once(" srcdoc=").expect("viewer shell");
        assert!(shell.contains("frame-src 'self'"));
        assert!(frame.contains("frame-src &#39;none&#39;"));
    }

    #[test]
    fn a_visualization_is_titled_by_its_file_name() {
        assert_eq!(
            title_of(Path::new("/v/2026/09/16/t/compound-interest-explorer.html")),
            "compound interest explorer"
        );
    }
}
