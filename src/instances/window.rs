//! The window's end of `moon open`: the socket a shell reaches it on, and the files that
//! have come in over it waiting for the next frame to open them.
//!
//! Only a real window listens. Every other caller of the app is a ui test, and a test must
//! not put itself in the way of a `moon open` typed in the window the developer running it
//! has open - see [`crate::native::app::App::listen_for_shell_asks`].

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use super::{Answer, Ask, Instance, remove_records, socket_path, write_record};

/// A file a shell asked this window to open.
pub(crate) struct OpenFileAsked {
    pub(crate) path: PathBuf,
    /// The line to put on screen, for `moon open <file>:<line>`.
    pub(crate) line: Option<usize>,
}

/// What a window keeps so shells can reach it: the project it is written down as being on,
/// and the asks that have arrived since the last frame.
pub(crate) struct ShellAsks {
    /// Read by the listening thread to decide whether this window has anything to open a
    /// file into yet, and written by the window whenever it opens another project.
    project: Arc<Mutex<Option<String>>>,
    arrived: Arc<Mutex<Vec<OpenFileAsked>>>,
    /// What this window is called on the command line, which is what `moon list` prints
    /// beside the project.
    program: String,
    /// When this window was last brought to the front, which is written into its record so a
    /// shell can tell the window being looked at from the ones behind it.
    focused_at_unix: Arc<Mutex<u64>>,
}

impl ShellAsks {
    /// Open the window's socket and start answering on it. The window is not written down
    /// yet: it has nothing to be found by until it is open on a project, which is what
    /// [`ShellAsks::on_project`] says.
    pub(crate) fn listen(
        program: String,
        reads_this_machine: bool,
        ctx: egui::Context,
    ) -> Result<Self> {
        let path = socket_path(std::process::id()).context("no home directory to listen in")?;
        let dir = path.parent().expect("the socket sits in the instances dir");
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        // A pid this process was handed again may have left its socket behind; binding to a
        // path that already exists fails, and that file cannot belong to anyone else.
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("failed to listen on {}", path.display()))?;

        let asks = Self {
            project: Arc::new(Mutex::new(None)),
            arrived: Arc::new(Mutex::new(Vec::new())),
            program,
            focused_at_unix: Arc::new(Mutex::new(0)),
        };
        let project = asks.project.clone();
        let arrived = asks.arrived.clone();
        thread::Builder::new()
            .name("moon-shell-asks".to_string())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    // One ask per connection, and each is answered before the next is read:
                    // a shell waits for its answer, so nothing is gained by doing several at
                    // once, and the window is only ever asked as fast as somebody types.
                    if let Err(error) = answer(stream, reads_this_machine, &project, &arrived) {
                        eprintln!("[moonreview] could not answer a `moon` ask: {error}");
                        continue;
                    }
                    // The file lands in a tab on the next frame, and a window nobody is
                    // looking at draws no frames until something asks it to.
                    ctx.request_repaint();
                }
            })
            .context("failed to start the thread answering shells")?;

        Ok(asks)
    }

    /// Say which project this window is on, so a shell asking about a file of it finds this
    /// window. Writing it again for the project it already says is harmless and is what a
    /// window that reopened the same project does.
    pub(crate) fn on_project(&self, project_path: &str) -> Result<()> {
        *self.project.lock().expect("the project lock") = Some(project_path.to_string());
        self.write_down(project_path.to_string())
    }

    /// Say this window has just been brought to the front. A file whose project no window is
    /// open on goes to the window that was in front most recently, so the moment it comes
    /// forward is the moment worth writing down.
    ///
    /// Nothing to write before the window is on a project: it has no record until then, and
    /// [`ShellAsks::on_project`] carries the time in with it when it writes the first one.
    pub(crate) fn came_to_the_front(&self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past 1970")
            .as_secs();
        *self.focused_at_unix.lock().expect("the focus lock") = now;

        let Some(project_path) = self.project.lock().expect("the project lock").clone() else {
            return Ok(());
        };
        self.write_down(project_path)
    }

    fn write_down(&self, project_path: String) -> Result<()> {
        write_record(&Instance {
            pid: std::process::id(),
            program: self.program.clone(),
            project_path,
            focused_at_unix: *self.focused_at_unix.lock().expect("the focus lock"),
        })
    }

    /// The files asked for since the last time this was called.
    pub(crate) fn drain(&self) -> Vec<OpenFileAsked> {
        std::mem::take(&mut *self.arrived.lock().expect("the arrived lock"))
    }
}

impl Drop for ShellAsks {
    fn drop(&mut self) {
        remove_records(std::process::id());
    }
}

/// Read one ask off a connection and answer it.
///
/// A file of another project is taken as readily as one of this window's own: the window
/// opens a session on the project holding it and puts the file in a tab of that - see
/// [`crate::native::open_from_shell`]. Which window is asked first is the shell's business,
/// and it asks the ones open on the file's project before any other.
///
/// What is refused is a window with nothing to open a file into: one still on its launch
/// screen, and one whose repo is on another machine, where a path typed in a shell here
/// names nothing at all.
fn answer(
    stream: UnixStream,
    reads_this_machine: bool,
    project: &Arc<Mutex<Option<String>>>,
    arrived: &Arc<Mutex<Vec<OpenFileAsked>>>,
) -> Result<()> {
    let mut asked = String::new();
    BufReader::new(&stream)
        .read_line(&mut asked)
        .context("failed to read the ask")?;
    let Ask::OpenFile { path, line } = serde_json::from_str(asked.trim())
        .with_context(|| format!("failed to read {asked:?} as an ask"))?;

    let answer = match project.lock().expect("the project lock").clone() {
        Some(project) if !reads_this_machine => Answer::Refused {
            reason: format!("this window is open on {project} on another machine"),
        },
        Some(_) => {
            arrived
                .lock()
                .expect("the arrived lock")
                .push(OpenFileAsked {
                    path: PathBuf::from(path),
                    line,
                });
            Answer::Opened
        }
        None => Answer::Refused {
            reason: "this window has no project open yet".to_string(),
        },
    };

    let mut writing = &stream;
    writeln!(writing, "{}", serde_json::to_string(&answer)?)?;
    writing.flush().context("failed to answer the shell")
}
