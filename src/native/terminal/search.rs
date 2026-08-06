//! Finding text in a shell, screen and scrollback alike.
//!
//! Ghostty has no search of its own, but it will hand over everything it is holding as one
//! formatted string — which is what `select_all` and the selection formatter are for. Rows of
//! that string are rows of the screen counting from the top of the scrollback, which is the
//! same coordinate `scroll_viewport` takes, so a match found in the text can be shown without
//! any further translation.

use libghostty_vt::{
    Terminal,
    selection::{FormatOptions, Selection},
    terminal::{Point, PointCoordinate, ScrollViewport},
};

/// Where the query turned up: the row from the top of the scrollback, and the columns it
/// covers on that row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Match {
    pub(crate) row: usize,
    pub(crate) column: u16,
    pub(crate) width: u16,
}

/// Every occurrence of the query, top to bottom, matched without regard for case.
pub(crate) fn find_all(terminal: &Terminal<'_, '_>, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let Some(text) = whole_text(terminal) else {
        return Vec::new();
    };
    let needle = query.to_lowercase();

    let mut found = Vec::new();
    for (row, line) in text.lines().enumerate() {
        // Column is a count of characters, because that is what a cell of the grid is. The
        // haystack is lowercased per line so the two indexes stay in step.
        let characters: Vec<char> = line.chars().collect();
        let lowered: Vec<char> = line.to_lowercase().chars().collect();
        if lowered.len() != characters.len() {
            // A character that changes length when lowercased would put the columns out;
            // rather than report the wrong cells, this row is matched exactly instead.
            find_in_row(&characters, &query.chars().collect::<Vec<_>>(), row, &mut found);
            continue;
        }
        find_in_row(&lowered, &needle.chars().collect::<Vec<_>>(), row, &mut found);
    }
    found
}

fn find_in_row(haystack: &[char], needle: &[char], row: usize, out: &mut Vec<Match>) {
    if needle.is_empty() || haystack.len() < needle.len() {
        return;
    }
    for start in 0..=haystack.len() - needle.len() {
        if haystack[start..start + needle.len()] == *needle {
            out.push(Match {
                row,
                column: start as u16,
                width: needle.len() as u16,
            });
        }
    }
}

/// Bring a match into view and select it, so it reads the same as anything else the pointer
/// had picked out.
pub(crate) fn show(
    terminal: &mut Terminal<'_, '_>,
    found: Match,
    rows: u16,
) -> anyhow::Result<()> {
    // A match in the middle of the pane is easier to read than one against the top edge.
    let above = usize::from(rows / 2);
    terminal.scroll_viewport(ScrollViewport::Row(found.row.saturating_sub(above)));

    let start = screen_ref(terminal, found.column, found.row)?;
    let end = screen_ref(
        terminal,
        found.column + found.width.saturating_sub(1),
        found.row,
    )?;
    terminal
        .set_selection(Some(&Selection::new(start, end, false)))
        .map_err(|error| anyhow::anyhow!("failed to select the match: {error}"))?;
    Ok(())
}

fn screen_ref<'t>(
    terminal: &'t Terminal<'_, '_>,
    x: u16,
    y: usize,
) -> anyhow::Result<libghostty_vt::screen::GridRef<'t>> {
    terminal
        .grid_ref(Point::Screen(PointCoordinate { x, y: y as u32 }))
        .map_err(|error| anyhow::anyhow!("failed to read the matched cell: {error}"))
}

/// Everything the shell is holding, one line per row of the screen and its scrollback.
///
/// Neither unwrapped nor trimmed, unlike a copy: a row has to stay a row, or the line a match
/// was found on would no longer say which row of the grid to scroll to.
fn whole_text(terminal: &Terminal<'_, '_>) -> Option<String> {
    let everything = terminal.select_all().ok()??;
    let options = FormatOptions::new()
        .with_unwrap(false)
        .with_trim(false)
        .with_selection(&everything);
    let bytes = terminal.format_selection_alloc(None, options).ok()??;
    String::from_utf8(bytes.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    fn terminal_showing(lines: &[&str]) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 40,
            rows: 4,
            max_scrollback: 200,
        })
        .expect("expected a terminal");
        for line in lines {
            terminal.vt_write(line.as_bytes());
            terminal.vt_write(b"\r\n");
        }
        terminal
    }

    #[test]
    fn a_match_is_found_with_the_row_and_column_it_sits_at() {
        let terminal = terminal_showing(&["hello world", "nothing here"]);
        let found = find_all(&terminal, "world");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].column, 6);
        assert_eq!(found[0].width, 5);
    }

    #[test]
    fn case_is_not_what_a_search_is_about() {
        let terminal = terminal_showing(&["Cargo.toml"]);

        assert_eq!(find_all(&terminal, "cargo").len(), 1);
        assert_eq!(find_all(&terminal, "CARGO").len(), 1);
    }

    #[test]
    fn every_occurrence_on_a_row_is_its_own_match() {
        let terminal = terminal_showing(&["one two one two one"]);
        let found = find_all(&terminal, "one");

        assert_eq!(found.len(), 3);
        assert_eq!(
            found.iter().map(|found| found.column).collect::<Vec<_>>(),
            vec![0, 8, 16]
        );
    }

    /// The point of searching a shell: what has scrolled off the top is still findable, and
    /// the row it comes back with counts from the top of the scrollback.
    #[test]
    fn a_match_that_scrolled_off_the_screen_is_still_found() {
        let mut lines: Vec<String> = (0..40).map(|index| format!("filler line {index}")).collect();
        lines.insert(3, "the needle is here".to_string());
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let terminal = terminal_showing(&borrowed);

        let found = find_all(&terminal, "needle");
        assert_eq!(found.len(), 1, "the scrollback is searched too");
        assert_eq!(found[0].row, 3, "and the row counts from the top of it");
    }

    #[test]
    fn showing_a_match_scrolls_to_it_and_selects_it() {
        let mut lines: Vec<String> = (0..40).map(|index| format!("filler line {index}")).collect();
        lines.insert(3, "the needle is here".to_string());
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut terminal = terminal_showing(&borrowed);

        let found = find_all(&terminal, "needle");
        show(&mut terminal, found[0], 4).expect("expected the match to be shown");

        let selected = crate::native::terminal::selection::selected_text(&terminal)
            .expect("showing a match selects it");
        assert_eq!(selected, "needle");
    }

    #[test]
    fn an_empty_query_finds_nothing_rather_than_everything() {
        let terminal = terminal_showing(&["hello world"]);

        assert!(find_all(&terminal, "").is_empty());
    }

    #[test]
    fn a_query_that_is_not_there_finds_nothing() {
        let terminal = terminal_showing(&["hello world"]);

        assert!(find_all(&terminal, "absent").is_empty());
    }
}
