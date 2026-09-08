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
    /// Read by the listening thread to decide whether a file is one this window can open,
    /// and written by the window whenever it opens another project.
    project: Arc<Mutex<Option<String>>>,
    arrived: Arc<Mutex<Vec<OpenFileAsked>>>,
    /// What this window is called on the command line, which is what `moon list` prints
    /// beside the project.
    program: String,
}

impl ShellAsks {
    /// Open the window's socket and start answering on it. The window is not written down
    /// yet: it has nothing to be found by until it is open on a project, which is what
    /// [`ShellAsks::on_project`] says.
    pub(crate) fn listen(program: String, ctx: egui::Context) -> Result<Self> {
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
                    if let Err(error) = answer(stream, &project, &arrived) {
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
        write_record(&Instance {
            pid: std::process::id(),
            program: self.program.clone(),
            project_path: project_path.to_string(),
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
/// A file outside this window's project is refused rather than opened: a tab is opened on a
/// file of the project the window is on, named by its path inside it, so there is no tab to
/// be opened on anything else. The shell that asked tries the next window.
fn answer(
    stream: UnixStream,
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
        Some(project) if std::path::Path::new(&path).starts_with(&project) => {
            arrived
                .lock()
                .expect("the arrived lock")
                .push(OpenFileAsked {
                    path: PathBuf::from(path),
                    line,
                });
            Answer::Opened
        }
        Some(project) => Answer::Refused {
            reason: format!("this window is open on {project}"),
        },
        None => Answer::Refused {
            reason: "this window has no project open yet".to_string(),
        },
    };

    let mut writing = &stream;
    writeln!(writing, "{}", serde_json::to_string(&answer)?)?;
    writing.flush().context("failed to answer the shell")
}
