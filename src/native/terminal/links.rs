//! Finding the URL under the pointer, so it can be underlined and opened.
//!
//! Two kinds of link end up on a terminal grid. A program can mark one explicitly with an
//! OSC 8 escape, which Ghostty records against the cells it covers; far more often it just
//! prints the URL as text, and the terminal is expected to notice. Both are looked for here,
//! the explicit one first because it says exactly which cells it covers.

use std::ops::Range;

use libghostty_vt::{
    Terminal,
    terminal::{Point, PointCoordinate},
};

/// What a printed URL has to start with to be treated as one. Anything else on a terminal
/// line — a bare `www.` or a path — is far more likely to be output than a link.
const SCHEMES: &[&str] = &["https://", "http://", "file://", "ftp://"];

/// Trailing characters a URL gives back: a link at the end of a sentence should not swallow
/// the full stop, and one inside brackets should not swallow the closing bracket.
const GIVEN_BACK: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '\'', '"'];

/// A URL on the grid: which cells of which row it covers, and where it points.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Link {
    pub(crate) row: u16,
    pub(crate) columns: Range<u16>,
    pub(crate) url: String,
}

/// The link covering a cell of the viewport, if there is one.
pub(crate) fn link_at(terminal: &Terminal<'_, '_>, cols: u16, x: u16, y: u16) -> Option<Link> {
    if x >= cols {
        return None;
    }
    marked_link_at(terminal, cols, x, y).or_else(|| printed_link_at(terminal, cols, x, y))
}

/// A link the program marked with OSC 8. Its extent is however far the same URI runs.
fn marked_link_at(terminal: &Terminal<'_, '_>, cols: u16, x: u16, y: u16) -> Option<Link> {
    let url = hyperlink_uri(terminal, x, y)?;

    let mut start = x;
    while start > 0 && hyperlink_uri(terminal, start - 1, y).as_deref() == Some(url.as_str()) {
        start -= 1;
    }
    let mut end = x + 1;
    while end < cols && hyperlink_uri(terminal, end, y).as_deref() == Some(url.as_str()) {
        end += 1;
    }

    Some(Link {
        row: y,
        columns: start..end,
        url,
    })
}

fn hyperlink_uri(terminal: &Terminal<'_, '_>, x: u16, y: u16) -> Option<String> {
    let grid_ref = terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x,
            y: u32::from(y),
        }))
        .ok()?;

    let mut buffer = [0u8; 2048];
    let written = grid_ref.hyperlink_uri(&mut buffer).ok()?;
    if written == 0 {
        return None;
    }
    String::from_utf8(buffer[..written].to_vec()).ok()
}

/// A URL the program simply printed, found by reading the row back as text.
fn printed_link_at(terminal: &Terminal<'_, '_>, cols: u16, x: u16, y: u16) -> Option<Link> {
    let row = row_text(terminal, cols, y);
    let found = find_url(&row, usize::from(x))?;

    Some(Link {
        row: y,
        columns: found.start as u16..found.end as u16,
        url: row[found].iter().collect(),
    })
}

/// One row as one char per cell, so a column of the grid is an index of the row.
///
/// A cell holding a grapheme cluster contributes only its first character. That keeps the
/// indexing honest, and a URL is made of characters that stand alone anyway.
fn row_text(terminal: &Terminal<'_, '_>, cols: u16, y: u16) -> Vec<char> {
    let mut out = Vec::with_capacity(usize::from(cols));
    let mut graphemes = [' '; 8];

    for x in 0..cols {
        let character = terminal
            .grid_ref(Point::Viewport(PointCoordinate {
                x,
                y: u32::from(y),
            }))
            .ok()
            .and_then(|grid_ref| {
                let written = grid_ref.graphemes(&mut graphemes).ok()?;
                (written > 0).then(|| graphemes[0])
            })
            .unwrap_or(' ');
        out.push(character);
    }
    out
}

/// The span of the URL covering `at`, if the row has one there.
fn find_url(row: &[char], at: usize) -> Option<Range<usize>> {
    for scheme in SCHEMES {
        let mut from = 0;
        while let Some(start) = find_from(row, from, scheme) {
            let end = url_end(row, start + scheme.chars().count());
            if (start..end).contains(&at) {
                return Some(start..end);
            }
            from = start + 1;
        }
    }
    None
}

fn find_from(row: &[char], from: usize, needle: &str) -> Option<usize> {
    let width = needle.chars().count();
    (from..row.len().saturating_sub(width) + 1)
        .find(|start| row[*start..start + width].iter().copied().eq(needle.chars()))
}

/// A URL runs to the first space or control character, minus whatever punctuation it ends on.
fn url_end(row: &[char], from: usize) -> usize {
    let mut end = from;
    while end < row.len() && !row[end].is_whitespace() && !row[end].is_control() {
        end += 1;
    }
    while end > from && GIVEN_BACK.contains(&row[end - 1]) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    fn terminal_showing(text: &str) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 60,
            rows: 4,
            max_scrollback: 100,
        })
        .expect("expected a terminal");
        terminal.vt_write(text.as_bytes());
        terminal
    }

    /// The whole way through: a URL printed by a program, read back off the real grid.
    #[test]
    fn a_url_printed_to_the_grid_is_found_under_the_pointer() {
        let terminal = terminal_showing("go to https://example.com/docs for more");

        let link = link_at(&terminal, 60, 10, 0).expect("expected a link");
        assert_eq!(link.url, "https://example.com/docs");
        assert_eq!(link.row, 0);
        assert_eq!(link.columns, 6..30);

        assert!(link_at(&terminal, 60, 2, 0).is_none(), "before the url");
        assert!(link_at(&terminal, 60, 40, 0).is_none(), "after it");
        assert!(link_at(&terminal, 60, 10, 1).is_none(), "a blank row");
    }

    /// An OSC 8 link says where it points regardless of what it shows, so the cells it
    /// covers have to come from the escape rather than from the text.
    #[test]
    fn an_osc_8_link_is_found_by_the_uri_it_carries() {
        let terminal =
            terminal_showing("see \x1b]8;;https://example.com/deep\x1b\\click here\x1b]8;;\x1b\\ ok");

        let link = link_at(&terminal, 60, 6, 0).expect("expected a link");
        assert_eq!(link.url, "https://example.com/deep");
        assert_eq!(
            link.columns,
            4..14,
            "the link covers exactly the cells between the escapes"
        );
        assert!(link_at(&terminal, 60, 1, 0).is_none(), "outside the escape");
    }

    #[test]
    fn a_column_past_the_end_of_the_grid_has_no_link() {
        let terminal = terminal_showing("https://example.com");

        assert!(link_at(&terminal, 60, 60, 0).is_none());
    }

    fn row(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn a_url_is_found_from_any_cell_it_covers() {
        let line = row("see https://example.com/x now");
        let span = find_url(&line, 4).expect("the first character of it");

        assert_eq!(line[span.clone()].iter().collect::<String>(), "https://example.com/x");
        assert_eq!(find_url(&line, 10), Some(span.clone()), "from the middle");
        assert_eq!(find_url(&line, span.end - 1), Some(span), "from the last one");
    }

    #[test]
    fn a_cell_outside_the_url_has_no_link() {
        let line = row("see https://example.com now");
        assert!(find_url(&line, 0).is_none(), "before it");
        assert!(find_url(&line, 25).is_none(), "after it");
    }

    #[test]
    fn punctuation_a_sentence_put_there_is_not_part_of_the_link() {
        let line = row("read https://example.com/page.");
        let span = find_url(&line, 10).expect("a url");

        assert_eq!(
            line[span].iter().collect::<String>(),
            "https://example.com/page"
        );
    }

    #[test]
    fn a_bracketed_url_keeps_its_own_path_but_not_the_bracket() {
        let line = row("(https://example.com/a(b))");
        let span = find_url(&line, 5).expect("a url");

        assert_eq!(
            line[span].iter().collect::<String>(),
            "https://example.com/a(b"
        );
    }

    #[test]
    fn a_row_with_no_scheme_has_no_link() {
        assert!(find_url(&row("example.com is not a link"), 2).is_none());
        assert!(find_url(&row("           "), 4).is_none());
    }

    /// A row that ends mid-URL still yields what is there, rather than running off the end.
    #[test]
    fn a_url_running_to_the_edge_of_the_row_is_still_found() {
        let line = row("https://example.com/very/long");
        let span = find_url(&line, 28).expect("a url");

        assert_eq!(span, 0..line.len());
    }

    #[test]
    fn the_second_url_on_a_row_is_found_too() {
        let line = row("http://one.example http://two.example");
        let span = find_url(&line, 25).expect("a url");

        assert_eq!(
            line[span].iter().collect::<String>(),
            "http://two.example"
        );
    }
}
