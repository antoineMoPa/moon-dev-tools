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
//! The window asks the server for the rows, like everything else on the board: the folder is on
//! the machine the repo is on, which a `--remote` window or one in a browser is not - and
//! whether a row is still pending is git run in the repos it names, which only that machine can
//! do. See `crate::moontasks::service::list_review_requests`.

// The files are read and written where the board's folder is, which is the server's side.
#[cfg(not(target_arch = "wasm32"))]
mod on_disk;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use on_disk::*;

use serde::{Deserialize, Serialize};

use crate::moontasks::ReviewRequestView;

/// What is being done to one entry of a task's file.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Amend {
    /// Take the line out - a review that turned out not to be wanted.
    Dismiss,
    /// Cross it off, or put it back. The line stays, because it is still true that the repo was
    /// part of this work; it just no longer wants looking at.
    Done(bool),
}

/// The same change, made to the rows a window is showing: what the server's `amend` will do to
/// the file, done to the list ahead of it so the board answers the click at once. A dismissed
/// row goes, and the rows under it in the same file move up one - their index is their
/// place in the file, and the file is one line shorter.
pub(crate) fn amend_views(
    requests: &mut Vec<ReviewRequestView>,
    task_id: &str,
    index: usize,
    amend: Amend,
) {
    match amend {
        Amend::Dismiss => {
            requests.retain(|request| !(request.task_id == task_id && request.index == index));
            for request in requests
                .iter_mut()
                .filter(|request| request.task_id == task_id && request.index > index)
            {
                request.index -= 1;
            }
        }
        Amend::Done(done) => {
            if let Some(request) = requests
                .iter_mut()
                .find(|request| request.task_id == task_id && request.index == index)
            {
                request.done = done;
            }
        }
    }
}
