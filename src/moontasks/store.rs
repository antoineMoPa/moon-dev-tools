//! The `.moontasks` folder: one directory per task, and the `metadata.json` inside it.
//!
//! The folder is the task. moonreview writes it, but nothing here assumes moonreview is the
//! only writer - a task can be created, renamed or moved between columns with a text editor,
//! and the board picks the change up on its next poll.

// The folder itself is the server's to read and write: the window in a browser only has the
// types a board is told to it in, and the rules both sides keep.
#[cfg(not(target_arch = "wasm32"))]
mod folder;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use folder::*;

use serde::{Deserialize, Serialize};
use web_time::{SystemTime, UNIX_EPOCH};

/// Which column a task sits in, by the name that column goes by in the board's file.
///
/// A name rather than one of a fixed set: the columns belong to the board, and a card says
/// which of them it is in. This is what `metadata.json` has always held in its `status`, so a
/// board written before columns could be changed reads back unchanged.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ColumnId(String);

impl ColumnId {
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ColumnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One column of the board: the name cards are in it under, and what it is called on screen.
///
/// Renaming changes the label and leaves the id alone, so every card already in the column
/// stays in it and a board someone renamed a column on is still readable by hand.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct BoardColumn {
    pub(crate) id: ColumnId,
    pub(crate) label: String,
    /// Which end of this column a card moved in from another one goes to, whatever place it
    /// was dropped at. Absent is where it was dropped, which is how a column of work in hand
    /// wants to behave; a column that is a record rather than a queue - DONE - is set to the
    /// top, so the most recently finished card is the one being looked at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arrivals: Option<ColumnEnd>,
    /// The order this column keeps its cards in by itself, whatever order they are dragged
    /// into - see [`crate::moontasks::column_sort`]. Absent is the order they were dragged
    /// into. A sorted column still remembers that order underneath, so turning its sort off
    /// puts every card back where it was put.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sort: Option<ColumnSort>,
}

/// An order a column keeps its cards in by itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ColumnSort {
    /// By title, with a run of digits read as the number it is: `2` before `10`, `01` before
    /// `02`.
    Alphabetical,
    /// By the numbers in the title, the first of them first - a ticket number, a step of a
    /// plan. A card with no number in its title goes after every card with one.
    Numerical,
    NewestFirst,
    OldestFirst,
}

/// Which end of a column a new card joins.
///
/// A card is made because of what it says, so the top is where one usually wants to be
/// looking - but a column read as a queue is worked from the top down, and a card added to
/// the back of that queue belongs at the bottom. The board asks for the one it means.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ColumnEnd {
    #[default]
    Top,
    Bottom,
}

/// The one column the board itself acts on, by the id it has on a board that started from the
/// defaults. A card only ever changes column because someone moved it; what happens here is
/// that a card let go of in this column lets go of its shells.
///
/// It is pinned by id: renaming the column or dragging it somewhere else keeps its part in
/// this rule, and deleting it turns the rule off rather than picking a column nobody chose.
// This rule and the next are the server's to keep; the window in a browser keeps the last two.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const RELEASES_SHELLS_IN: &str = "done";

/// The column a task stops asking for reviews in, by the same reckoning: a card let go of here
/// is finished, and every repo its `request_for_review.txt` names reads as reviewed whether or
/// not the lines were crossed off one at a time.
///
/// Its own rule rather than a second reading of [`RELEASES_SHELLS_IN`], though both point at the
/// same column on a board that started from the defaults: they are two things the board does,
/// and a board that keeps one of them and not the other is a board that deleted one column.
///
/// Nothing is written to the file for it. The lines are the agent's record of what the work
/// touched, and a card dragged back out of this column is being worked on again - so it comes
/// back asking for exactly what it was asking for before.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const CLOSES_REVIEWS_IN: &str = "done";

/// The column a card's right-click menu offers to move it to, by the same reckoning: the
/// column a finished card goes to on a board that started from the defaults. A board without
/// it has no such entry on the menu - there is no guessing which of its columns means done.
pub(crate) const MENU_FINISHES_IN: &str = "done";

/// The column that draws a dated line between its cards wherever the day they arrived on
/// changes, by the same reckoning: a column that is a record rather than a queue, where "when
/// was this finished" is the question. See [`crate::native::board::day_lines`].
pub(crate) const DATES_ARRIVALS_IN: &str = "done";

/// What a resource on a card is: a plain shell, an agent working on the task, or a file of
/// the repo the task is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskResourceKind {
    Shell,
    Agent,
    File,
    /// A visualization a run of the task announced, kept in the task's folder - see
    /// [`crate::visualizations::on_tasks`].
    Visualization,
}

/// One tag in the single spelling the board keeps it in, or nothing at all for text that has
/// no tag in it.
///
/// Lowercase, with runs of anything but letters, digits, `-` and `_` turned into one dash, so
/// `Needs Tests`, `needs-tests` and ` needs   tests ` are the same tag: a tag is typed by hand
/// on one card and again on the next, and two spellings of one word would be two pills.
pub(crate) fn tag_of(text: &str) -> Option<String> {
    let mut tag = String::new();
    let mut pending_dash = false;
    for character in text.chars() {
        if character.is_alphanumeric() || character == '-' || character == '_' {
            if pending_dash && !tag.is_empty() {
                tag.push('-');
            }
            pending_dash = false;
            tag.extend(character.to_lowercase());
        } else {
            pending_dash = true;
        }
    }
    (!tag.is_empty()).then_some(tag)
}

/// A list of tags as the board keeps it: each one spelled by [`tag_of`], in the order given,
/// with nothing said twice.
pub(crate) fn tags_of<'a>(text: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in text.into_iter().filter_map(tag_of) {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A version 4 UUID, from the same entropy the standard library seeds its hashers with.
pub(crate) fn new_uuid() -> String {
    let (high, low) = (random_u64(), random_u64());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        high >> 32,
        (high >> 16) & 0xffff,
        high & 0x0fff,
        // The variant bits: `10xx`, which puts the first character in `8..=b`.
        0x8000 | (low >> 48) & 0x3fff,
        low & 0xffff_ffff_ffff_u64
    )
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};

    // `RandomState` takes its keys from the operating system, and moves them on every call,
    // which is exactly the per-call randomness an id needs.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default(),
    );
    hasher.finish()
}
