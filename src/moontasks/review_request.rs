//! `request_for_review.txt`: the repos an agent says are ready to be looked at, in the order
//! they have to be deployed.
//!
//! An agent working in a task often finishes with work spread over several repos - a submodule,
//! its parent, another submodule - and the order they must be committed in is the part that
//! matters most and the part prose in `notes.md` cannot be acted on. So the agent writes the
//! list here instead, one repo per line, top to bottom, and the board draws a row for each.
//!
//! The file is the agent's. Nothing here writes it back: an entry goes when the agent takes it
//! out, and the board only reads.
//!
//! The window reads the files itself, off the board's folder, rather than asking the server for
//! them: this is a handful of short files in a folder the window already knows, and a route, a
//! trait method and two implementations of it would be four places to change every time a line
//! of the format does.

// The files are read and written where the board's folder is, which a window in a browser is
// never on: it has none of the rows, so nothing to amend either.
#[cfg(not(target_arch = "wasm32"))]
mod on_disk;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use on_disk::*;

/// What is being done to one entry of a task's file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Amend {
    /// Take the line out - a review that turned out not to be wanted.
    Dismiss,
    /// Cross it off, or put it back. The line stays, because it is still true that the repo was
    /// part of this work; it just no longer wants looking at.
    Done(bool),
}
