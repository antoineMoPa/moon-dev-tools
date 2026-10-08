//! Server asks - the socket a `moon serve` answers `moon launch` on.
//!
//! A `moon serve` is no window, and is written down as none: no shell is ever sent to it with
//! a file to open, a folder or a line of the wire - see [`super::running`]. Its shells are
//! started with its process in [`super::WINDOW_ENV`] all the same, and that is how a
//! `moon launch` typed in one of them finds this socket, which is named after the process the
//! way a window's is.

use std::{os::unix::net::UnixStream, sync::Arc, thread};

use anyhow::{Context, Result};

use super::{
    Answer, Ask, StartsApplications, launched, listen_on_own_socket, read_ask, remove_records,
    write_answer,
};

/// What a server keeps so its shells can reach it. Dropping it takes the socket away.
pub(crate) struct ServerAsks;

impl ServerAsks {
    /// Open the server's socket and start answering on it, with `applications` starting what
    /// is asked for.
    pub(crate) fn listen(applications: Arc<dyn StartsApplications>) -> Result<Self> {
        let listener = listen_on_own_socket()?;
        thread::Builder::new()
            .name("moon-server-asks".to_string())
            .spawn(move || {
                // One ask per connection, each answered before the next is read, as a
                // window's are: a start is waited on for a second or two, and a shell asks
                // only as fast as somebody types.
                for stream in listener.incoming().flatten() {
                    if let Err(error) = answer(stream, applications.as_ref()) {
                        eprintln!("[moonreview] could not answer a `moon` ask: {error}");
                    }
                }
            })
            .context("failed to start the thread answering shells")?;
        Ok(Self)
    }
}

impl Drop for ServerAsks {
    fn drop(&mut self) {
        remove_records(std::process::id());
    }
}

/// Read one ask off a connection and answer it. Everything but a program to start is a
/// window's to do, and is refused.
fn answer(stream: UnixStream, applications: &dyn StartsApplications) -> Result<()> {
    let answer = match read_ask(&stream)? {
        Ask::Launch { command, folder } => launched(applications, &command, &folder),
        _ => Answer::Refused {
            reason: "this moon is a `moon serve`, which has no window to do that in".to_string(),
        },
    };
    write_answer(&stream, &answer)
}
