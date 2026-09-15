//! The work log: the project's journal, opened at a new dated entry by `Tools › Work Log`
//! and the palette's `work log`.
//!
//! It is `work-log.org` in the board's folder, `.moontasks`, made the first time it is
//! opened. One file the person keeps appending to, entry after entry, each headed by the
//! moment it was started:
//!
//! ```text
//! * Tue  1 Sep 2026 18:05:04 EDT
//! ==================================================
//!
//! what was going on
//!
//! #now#
//! ```
//!
//! The file ends on the line `#now#`, and a new entry goes in above it: two blank lines, the
//! heading, a rule of fifty `=`, and two blank lines the caret is left on the first of. A
//! file with a few hundred entries in one shape is not the place to start another, so the
//! shape is kept to the byte.
//!
//! The tab is an ordinary file tab of the repo's own session - saved with ⌘S, kept up with
//! what another editor writes to it, and read-only nowhere. The entry is typed into the tab
//! rather than written to the file: saving is the person's.

use anyhow::{Context, Result, bail};
use egui_frames::PaneId;

use crate::native::{
    app::App,
    panes::{Pane, PaneKind},
};

/// The line the file ends on, which every new entry goes above. Looked for from the top: the
/// first line holding it is the one.
pub(crate) const NOW_MARKER: &str = "#now#";

/// The rule under an entry's heading: fifty of them.
const RULE: &str = "==================================================";

/// How the heading's moment is written, which is `date` in an English locale:
/// `Tue  1 Sep 2026 18:05:04 EDT`, the day padded to two with a space.
const STAMP_FORMAT: &str = "+%a %e %b %Y %H:%M:%S %Z";

/// The entry being added: the moment it was asked for, which is what heads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewEntry {
    pub(crate) stamp: String,
}

impl NewEntry {
    /// An entry headed by now.
    ///
    /// `date` rather than a date crate: the heading carries the zone as a name - `EDT` -
    /// which a date crate has no table for. In the C locale, so the day and month read the
    /// same whatever language the window was launched under.
    pub(crate) fn now() -> Result<Self> {
        let output = std::process::Command::new("date")
            .env("LC_ALL", "C")
            .arg(STAMP_FORMAT)
            .output()
            .context("could not run `date`")?;
        if !output.status.success() {
            bail!("`date` failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        Ok(Self {
            stamp: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        })
    }

    /// The lines the entry puts into the file, as one string to insert before the marker's
    /// line. The two blank lines at the front keep it clear of the entry above; the two at
    /// the end are where it is written, with the caret on the first of them.
    pub(crate) fn header(&self) -> String {
        format!("\n\n* {}\n{RULE}\n\n\n", self.stamp)
    }
}

/// Where a new entry goes into the text, worked out from the text as it stands.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Insertion {
    /// The byte the header goes in at: the start of the marker's line.
    pub(crate) byte: usize,
    pub(crate) header: String,
    /// The byte the caret is left at once the header is in: the first of the two blank
    /// lines at its end.
    pub(crate) caret: usize,
    /// The line the caret is on then, counted from one as the fringe counts.
    pub(crate) line: usize,
}

/// The entry's place in the text, or `None` for a text with no [`NOW_MARKER`] in it - which
/// is the person's to fix, not something to guess an end of the file around.
pub(crate) fn insertion(text: &str, entry: &NewEntry) -> Option<Insertion> {
    let marker = text.find(NOW_MARKER)?;
    let byte = text[..marker].rfind('\n').map_or(0, |at| at + 1);
    let header = entry.header();
    // The header ends in the two blank lines; the caret is at the start of the first, which
    // is before the last two line breaks.
    let caret = byte + header.len() - 2;
    let line =
        text[..byte].matches('\n').count() + header[..header.len() - 2].matches('\n').count() + 1;
    Some(Insertion {
        byte,
        header,
        caret,
        line,
    })
}

impl App {
    /// Open the project's work log at a new dated entry, in the tab already on it or a new
    /// one. The file is made on the way when the project has none yet, which is a call to
    /// the repo's side, so the tab opens once that comes back; the entry waits on the tab
    /// until its text is there to put it in - see [`Self::add_waiting_work_log_entry`].
    pub(crate) fn open_work_log(&mut self) {
        let entry = match NewEntry::now() {
            Ok(entry) => entry,
            Err(error) => {
                self.model
                    .error(format!("could not date the entry: {error:#}"));
                return;
            }
        };
        let session_id = self.model.root_session_id.clone();
        let for_call = session_id.clone();
        self.tasks.spawn(
            move |backend| backend.open_work_log(&for_call),
            move |model, result| match result {
                Ok(file_path) => {
                    let found = model.layout.find_pane(|pane| {
                        matches!(pane, Pane::File { session_id: of, file_path: open, revision: None, .. }
                            if *of == session_id && *open == file_path)
                    });
                    let pane_id = match found {
                        Some((pane_id, _)) => {
                            model.layout.focus_pane(pane_id);
                            pane_id
                        }
                        None => {
                            let frame = model
                                .layout
                                .frame_holding(model.layout.active_frame(), |pane| {
                                    pane.kind() == PaneKind::File
                                })
                                .unwrap_or_else(|| model.layout.primary_frame());
                            model.layout.add_pane(
                                frame,
                                Pane::File {
                                    session_id,
                                    file_path,
                                    task_id: None,
                                    revision: None,
                                },
                                None,
                            )
                        }
                    };
                    model.work_log_entries_waiting.insert(pane_id, entry);
                }
                Err(error) => model.error(format!("could not open the work log: {error}")),
            },
        );
    }

    /// Put the entry waiting for this tab into it, once the tab has the text to put it in.
    /// Run as the tab is drawn, which is the first moment there is an editor to ask.
    pub(crate) fn add_waiting_work_log_entry(&mut self, pane_id: PaneId) {
        if !self.model.work_log_entries_waiting.contains_key(&pane_id) {
            return;
        }
        let loaded = self
            .model
            .file_editors
            .get(&pane_id)
            .is_some_and(|editor| editor.is_loaded());
        if !loaded {
            return;
        }
        let entry = self
            .model
            .work_log_entries_waiting
            .remove(&pane_id)
            .expect("checked just above");
        let editor = self
            .model
            .file_editors
            .get_mut(&pane_id)
            .expect("checked just above");
        if let Err(said) = editor.add_work_log_entry(&entry) {
            self.model.error(said);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> NewEntry {
        NewEntry {
            stamp: "Tue  1 Sep 2026 18:05:04 EDT".to_string(),
        }
    }

    /// Byte for byte the shape the file already has: `\n\n* `, the date, fifty `=`, `\n\n\n`.
    #[test]
    fn the_header_is_shaped_like_every_entry_already_in_the_file() {
        assert_eq!(
            entry().header(),
            "\n\n* Tue  1 Sep 2026 18:05:04 EDT\n==================================================\n\n\n"
        );
        assert_eq!(RULE.len(), 50);
    }

    #[test]
    fn the_entry_goes_in_above_the_marker_line_and_the_caret_on_its_first_blank_line() {
        let text = "* old\n=====\n\nthings\n\n#now#\n";
        let found = insertion(text, &entry()).expect("expected the marker to be found");

        assert_eq!(found.byte, text.find("#now#").unwrap());
        let mut with = text.to_string();
        with.insert_str(found.byte, &found.header);
        assert_eq!(
            with,
            "* old\n=====\n\nthings\n\n\n\n* Tue  1 Sep 2026 18:05:04 EDT\n==================================================\n\n\n#now#\n"
        );
        // The caret sits at the start of the first blank line under the rule: what is before
        // it ends with the rule's line break, what is after it is that line's break, the
        // other blank line, and the marker.
        assert!(with[..found.caret].ends_with("=\n"));
        assert_eq!(&with[found.caret..], "\n\n#now#\n");
        // Lines: 1 `* old`, 2 rule, 3 blank, 4 things, 5 blank, 6 blank, 7 blank, 8 heading,
        // 9 rule, 10 the caret's blank line.
        assert_eq!(found.line, 10);
    }

    /// What a project's first entry looks like: the file the board made holds only the
    /// marker, so the entry opens the file.
    #[test]
    fn a_marker_on_the_first_line_puts_the_entry_at_the_top() {
        let found = insertion("#now#\n", &entry()).expect("expected the marker to be found");

        assert_eq!(found.byte, 0);
        assert_eq!(found.line, 5);
    }

    #[test]
    fn a_file_with_no_marker_has_nowhere_for_an_entry() {
        assert_eq!(insertion("* old\n\nthings\n", &entry()), None);
    }

    /// `date` in the C locale, in the shape the file already has: the day padded to two with
    /// a space, an English month, the zone as a name.
    #[test]
    fn the_stamp_reads_like_the_headings_already_in_the_file() {
        let entry = NewEntry::now().expect("expected `date` to run");

        // `Tue  1 Sep 2026 18:05:04 EDT`: six words, a one-digit day padded with a space.
        let words: Vec<&str> = entry.stamp.split_whitespace().collect();
        assert_eq!(words.len(), 6, "{:?}", entry.stamp);
        assert!(
            words[0].len() == 3 && words[0].chars().all(char::is_alphabetic),
            "{:?}",
            entry.stamp
        );
        assert!(
            words[1].chars().all(|c| c.is_ascii_digit()),
            "{:?}",
            entry.stamp
        );
        assert!(
            words[2].len() == 3 && words[2].chars().all(char::is_alphabetic),
            "{:?}",
            entry.stamp
        );
        assert_eq!(words[3].len(), 4, "{:?}", entry.stamp);
        assert_eq!(words[4].len(), 8, "{:?}", entry.stamp);
        if words[1].len() == 1 {
            assert!(
                entry.stamp.contains("  "),
                "a one-digit day is padded with a space: {:?}",
                entry.stamp
            );
        }
    }
}
