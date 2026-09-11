//! Who last touched each line of a file tab, in a column beside the lines.
//!
//! Turned on per tab - `[blame]` in the header, ⌥⇧B, the palette's "toggle blame" or the
//! text's context menu - and read out of `git blame` over the tab's own buffer, so a line
//! typed since the last commit reads as not committed rather than as whatever used to be on
//! that line. The column gathers the lines into stretches, one per commit, with the short
//! sha, the day and the author at the top of each and the commit's summary on the row under;
//! the whole of it is in the note that opens on pointing at a stretch, and clicking one opens
//! the review on that commit - on the local changes, for a stretch nothing has committed.
//!
//! The blame follows the typing at a distance. While the column is up the text is hashed once
//! every [`CHECK_INTERVAL`], and asked about again once it has held still for one interval
//! and is not what the column was worked out from. Not on every keystroke, since each ask is
//! a git run, and not as the typing happens, since a column redrawn under every letter pulls
//! the eye off the code - the same reason the fringe waits before marking a line new.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::{Duration, Instant},
};

use egui::Color32;
use egui_frames::PaneId;
use egui_moon_editor::LineNote;

use crate::{
    api::{BlameChunk, Blamed},
    native::{
        app::App, palette::CommandAction, panes::OpenPaneRequest, panes::Pane, theme::Palette,
    },
};

/// How often a tab with its blame up reads its text again to see whether the blame is still
/// of it. The disk check under the same tab runs about as often.
const CHECK_INTERVAL: Duration = Duration::from_millis(1000);

/// What the column reads at the top of a stretch no commit has.
const NOT_YET_COMMITTED_TITLE: &str = "not committed yet";

/// How many characters of a sha the column shows: what `git log --oneline` shows.
const SHORT_SHA: usize = 7;

/// The blame of one file tab: whether it is up, what it shows, and what it takes to keep
/// that true of the text as it is typed into.
#[derive(Default)]
pub(crate) struct Blaming {
    on: bool,
    /// The stretches on screen, with the hash of the text they were asked about.
    shown: Option<Shown>,
    /// The notes drawn from [`shown`](Self::shown), kept between frames rather than built
    /// again on each, with the ink the uncommitted ones were drawn in - the one thing about
    /// them the theme can change under the tab.
    notes: Option<(Color32, Vec<LineNote>)>,
    /// The text as it was at the last check, so the next can tell whether it held still.
    last_check: Option<Check>,
    /// Whether a blame is out, so a second is not sent before the first is back.
    asking: bool,
}

struct Shown {
    text_hash: u64,
    chunks: Vec<BlameChunk>,
}

struct Check {
    at: Instant,
    text_hash: u64,
}

impl Blaming {
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// Put the column up or take it down. Coming up it starts from nothing: what was shown
    /// before it went down may be of a text long since typed over.
    pub(crate) fn toggle(&mut self) {
        self.on = !self.on;
        self.shown = None;
        self.notes = None;
        self.last_check = None;
    }

    /// The column is taken down when a blame fails: the error says why, and a column that
    /// stayed up empty would be a column asking every second and failing every second.
    fn take_down(&mut self) {
        self.on = false;
        self.shown = None;
        self.notes = None;
    }

    /// Whether the text, as hashed now, is due a blame: the column is up, nothing is out, and
    /// what is shown - if anything - is not of this text, which has held still since the
    /// last check. A column with nothing in it yet is asked at once, because there is no
    /// typing to wait for the end of.
    fn wants_to_ask(&mut self, text_hash: u64, now: Instant) -> bool {
        if !self.on || self.asking {
            return false;
        }
        let shows_this_text = self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.text_hash == text_hash);
        let held_still = self
            .last_check
            .as_ref()
            .is_some_and(|check| check.text_hash == text_hash);
        self.last_check = Some(Check { at: now, text_hash });
        !shows_this_text && (self.shown.is_none() || held_still)
    }

    /// Whether the next check is due - see [`CHECK_INTERVAL`].
    fn check_is_due(&self, now: Instant) -> bool {
        self.last_check
            .as_ref()
            .is_none_or(|check| now.duration_since(check.at) >= CHECK_INTERVAL)
    }

    /// What came back for the text hashed to `text_hash`.
    fn answered(&mut self, text_hash: u64, chunks: Vec<BlameChunk>) {
        self.asking = false;
        self.shown = Some(Shown { text_hash, chunks });
        self.notes = None;
    }

    /// The notes the column draws, one per stretch - see [`note_of`].
    pub(crate) fn notes(&mut self, palette: &Palette) -> &[LineNote] {
        let Some(shown) = &self.shown else {
            return &[];
        };
        let uncommitted_ink = palette.added;
        if self
            .notes
            .as_ref()
            .is_none_or(|(ink, _)| *ink != uncommitted_ink)
        {
            let notes = shown
                .chunks
                .iter()
                .map(|chunk| note_of(chunk, uncommitted_ink))
                .collect();
            self.notes = Some((uncommitted_ink, notes));
        }
        &self.notes.as_ref().expect("just built").1
    }

    /// The stretch a note is about, by the note's index - the two are one to one.
    pub(crate) fn chunk(&self, note: usize) -> Option<&BlameChunk> {
        self.shown.as_ref()?.chunks.get(note)
    }

    /// The stretches on screen, for the test that waits on a real blame.
    #[cfg(test)]
    pub(crate) fn chunks_for_test(&self) -> Option<&[BlameChunk]> {
        self.shown.as_ref().map(|shown| shown.chunks.as_slice())
    }
}

/// What the column reads for one stretch: the short sha, the day and the author at the top,
/// and the commit's summary under, or that nothing has committed the lines yet - in the ink
/// the fringe marks a new line with, since that is what they are.
fn note_of(chunk: &BlameChunk, uncommitted_ink: Color32) -> LineNote {
    match &chunk.blamed {
        Blamed::Committed(commit) => LineNote {
            lines: chunk.lines.clone(),
            title: format!(
                "{} {} {}",
                short_sha(&commit.sha),
                commit.authored_on,
                commit.author
            ),
            detail: commit.summary.clone(),
            ink: None,
        },
        Blamed::NotYetCommitted => LineNote {
            lines: chunk.lines.clone(),
            title: NOT_YET_COMMITTED_TITLE.to_string(),
            detail: String::new(),
            ink: Some(uncommitted_ink),
        },
    }
}

fn short_sha(sha: &str) -> &str {
    &sha[..SHORT_SHA.min(sha.len())]
}

fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Whether the tab in front is a file with a history to ask about - one of the repo's own -
/// which is when the palette offers to toggle its blame.
pub(crate) fn front_tab_blames(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, pane)| {
        matches!(pane, Pane::File { .. })
            && app
                .model
                .file_editors
                .get(&pane_id)
                .is_some_and(|editor| editor.is_loaded() && !editor.is_outside_the_repo())
    })
}

/// Whether the tab in front has its blame up, for the palette to word its command by.
pub(crate) fn front_tab_shows_blame(app: &App) -> bool {
    app.active_pane().is_some_and(|(pane_id, _)| {
        app.model
            .file_editors
            .get(&pane_id)
            .is_some_and(|editor| editor.blaming().is_on())
    })
}

/// Put the blame of the file tab in front up, or take it down - ⌥⇧B, and the palette's
/// "toggle blame".
pub(crate) fn toggle_in_front(app: &mut App) {
    let Some((pane_id, Pane::File { .. })) = app.active_pane() else {
        app.model.error("blame works on a file tab");
        return;
    };
    toggle(app, pane_id);
}

/// Put one file tab's blame up, or take it down.
pub(crate) fn toggle(app: &mut App, pane_id: PaneId) {
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    if editor.is_outside_the_repo() {
        let said = format!(
            "{} is outside the repo, and has no history here to blame",
            editor.file_path
        );
        app.model.error(said);
        return;
    }
    editor.blaming_mut().toggle();
}

/// Keep a tab's blame of the text in it, checking once an interval - see the
/// [module](self). Called as the tab draws, which is where its text is.
pub(crate) fn follow(app: &mut App, ctx: &egui::Context, pane_id: PaneId, session_id: &str) {
    let key = format!("blame:{pane_id}");
    let Some(editor) = app.model.file_editors.get_mut(&pane_id) else {
        return;
    };
    if !editor.blaming().is_on() || !editor.is_loaded() {
        return;
    }
    // The check has to come round even with nobody typing: the ask waits for the text to
    // hold still, and a frame has to happen for it to see that it has.
    ctx.request_repaint_after(CHECK_INTERVAL);
    let now = Instant::now();
    if !editor.blaming().check_is_due(now) {
        return;
    }
    let text_hash = hash_of(editor.text());
    if !editor.blaming_mut().wants_to_ask(text_hash, now) {
        return;
    }
    editor.blaming_mut().asking = true;
    let text = editor.text().to_string();
    let file_path = editor.file_path.clone();

    let for_call = session_id.to_string();
    let for_ask = file_path.clone();
    app.tasks.spawn_keyed(
        Some(key),
        move |backend| backend.blame_file(&for_call, &for_ask, &text),
        move |model, result| {
            let Some(editor) = model.file_editors.get_mut(&pane_id) else {
                return;
            };
            match result {
                Ok(payload) => editor.blaming_mut().answered(text_hash, payload.chunks),
                Err(error) => {
                    editor.blaming_mut().asking = false;
                    editor.blaming_mut().take_down();
                    model.error(format!("could not blame {file_path}: {error}"));
                }
            }
        },
    );
}

/// The whole of a note, on pointing at its stretch: what the column had room for and the
/// rest - the full sha, and what a click does.
pub(crate) fn draw_tooltip(ui: &egui::Ui, palette: &Palette, chunk: &BlameChunk) {
    let id = ui.id().with(("blame-tooltip", chunk.lines.start));
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        id,
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.set_max_width(360.0);
        match &chunk.blamed {
            Blamed::Committed(commit) => {
                ui.label(egui::RichText::new(&commit.summary).strong());
                ui.label(format!("{} · {}", commit.author, commit.authored_on));
                ui.label(
                    egui::RichText::new(&commit.sha)
                        .monospace()
                        .color(palette.muted),
                );
                ui.label(
                    egui::RichText::new("Click to open the review on this commit")
                        .color(palette.muted),
                );
            }
            Blamed::NotYetCommitted => {
                ui.label(egui::RichText::new(NOT_YET_COMMITTED_TITLE).strong());
                ui.label(
                    "These lines are not in any commit: typed since the last one, saved or not.",
                );
                ui.label(
                    egui::RichText::new("Click to open the review on the local changes")
                        .color(palette.muted),
                );
            }
        }
    });
}

/// Open the review of the tab's session on the commit a stretch was last touched in - the
/// local changes, for a stretch nothing has committed - and bring it forward.
///
/// Deferred like every other pane change made from inside a draw - see
/// [`App::open_file_pane`]: the tree holding the file pane must not be rebuilt under it.
pub(crate) fn show_commit(app: &mut App, session_id: &str, chunk: &BlameChunk) {
    let commit = match &chunk.blamed {
        Blamed::Committed(commit) => Some(commit.sha.clone()),
        Blamed::NotYetCommitted => None,
    };
    crate::native::review::sidebar::select_commit(app, session_id, commit);
    if app.pending_action.is_some() {
        return;
    }
    // A review already open keeps its title; one opened here for the first time is named
    // the way the root review is, which is what a file tab's session nearly always is.
    let title = app
        .model
        .layout
        .find_pane(|pane| pane.reviews(session_id))
        .map(|(_, pane)| match pane {
            Pane::Review { title, .. } => title.clone(),
            _ => unreachable!("reviews() only answers for a review pane"),
        })
        .unwrap_or_else(|| "review".to_string());
    app.pending_action = Some(CommandAction::OpenPane(OpenPaneRequest::Review {
        session_id: session_id.to_string(),
        title,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::BlamedCommit;

    fn committed(lines: std::ops::Range<usize>) -> BlameChunk {
        BlameChunk {
            lines,
            blamed: Blamed::Committed(BlamedCommit {
                sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
                author: "Ada Lovelace".to_string(),
                authored_on: "2024-01-02".to_string(),
                summary: "Add the library".to_string(),
            }),
        }
    }

    fn not_committed(lines: std::ops::Range<usize>) -> BlameChunk {
        BlameChunk {
            lines,
            blamed: Blamed::NotYetCommitted,
        }
    }

    #[test]
    fn a_note_reads_the_short_sha_the_day_and_the_author_over_the_summary() {
        let note = note_of(&committed(3..7), Color32::GREEN);
        assert_eq!(note.lines, 3..7);
        assert_eq!(note.title, "0123456 2024-01-02 Ada Lovelace");
        assert_eq!(note.detail, "Add the library");
        assert_eq!(note.ink, None);

        let note = note_of(&not_committed(0..1), Color32::GREEN);
        assert_eq!(note.title, NOT_YET_COMMITTED_TITLE);
        assert_eq!(note.detail, "");
        assert_eq!(note.ink, Some(Color32::GREEN));
    }

    /// A column just put up asks at once; after that it asks only once the text has held
    /// still for a check and is not what is shown, and never while an ask is out.
    #[test]
    fn a_blame_is_asked_for_at_once_and_then_only_once_the_typing_has_stopped() {
        let now = Instant::now();
        let mut blaming = Blaming::default();
        assert!(
            !blaming.wants_to_ask(1, now),
            "a column that is down asks nothing"
        );

        blaming.toggle();
        assert!(
            blaming.wants_to_ask(1, now),
            "an empty column is asked at once"
        );
        blaming.asking = true;
        assert!(!blaming.wants_to_ask(1, now), "not while an ask is out");
        blaming.answered(1, vec![committed(0..1)]);
        assert!(
            !blaming.wants_to_ask(1, now),
            "what is shown is of this text"
        );

        // Typing: a check sees new text, the next sees it again.
        assert!(
            !blaming.wants_to_ask(2, now),
            "the text has only just changed"
        );
        assert!(!blaming.wants_to_ask(3, now), "and changed again");
        assert!(blaming.wants_to_ask(3, now), "held still for a check: ask");

        blaming.toggle();
        assert!(!blaming.is_on());
        assert!(
            blaming.chunks_for_test().is_none(),
            "taken down, nothing is kept"
        );
    }

    #[test]
    fn a_check_comes_round_once_an_interval() {
        let now = Instant::now();
        let mut blaming = Blaming::default();
        blaming.toggle();
        assert!(blaming.check_is_due(now));
        blaming.wants_to_ask(1, now);
        assert!(!blaming.check_is_due(now + Duration::from_millis(500)));
        assert!(blaming.check_is_due(now + CHECK_INTERVAL));
    }

    /// The notes are built once per answer, and again when the theme changes the ink the
    /// uncommitted ones are drawn in.
    #[test]
    fn notes_are_kept_between_frames_and_rebuilt_for_a_new_ink() {
        let mut blaming = Blaming::default();
        blaming.toggle();
        blaming.wants_to_ask(1, Instant::now());
        blaming.answered(1, vec![committed(0..2), not_committed(2..3)]);

        let mut palette = Palette::of(crate::native::theme::ThemeMode::Dark);
        palette.added = Color32::GREEN;
        let notes = blaming.notes(&palette).to_vec();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[1].ink, Some(Color32::GREEN));

        palette.added = Color32::RED;
        assert_eq!(blaming.notes(&palette)[1].ink, Some(Color32::RED));
        assert_eq!(blaming.chunk(1), Some(&not_committed(2..3)));
        assert_eq!(blaming.chunk(2), None);
    }
}
