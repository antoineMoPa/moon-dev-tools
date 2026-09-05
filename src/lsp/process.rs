//! One language server process, and the JSON-RPC that goes over its stdio.
//!
//! Plain [`std::process::Command`] with a thread reading its stdout, the way
//! [`crate::terminal`] runs its shells. The calls this serves come in through
//! [`crate::backend::Backend`], which is synchronous and not necessarily inside a tokio
//! runtime, so an async LSP framework would have to be bridged at every call anyway - and
//! the whole of what it would buy is the framing in [`super::framing`].

use std::{
    collections::HashMap,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::{framing::Frames, languages::ServerSpec, protocol::ProgressNote};

/// How long a request waits for its answer. A server that has stopped answering must not
/// take the thread that asked with it: the window's worker threads are few, and a stuck one
/// is a pane that never draws again.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Starting up is allowed longer: a server reads the project's manifest before it replies,
/// and on a cold cache that is not instant.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `shutdown` is given before the process is killed anyway. Being polite is worth
/// a moment; waiting on it is not.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(300);
/// How long a server has to stay quiet before it is taken to be ready - see [`Readiness`].
const SETTLING: Duration = Duration::from_secs(2);
const READ_CHUNK: usize = 16 * 1024;

/// Which units the server counts a column in - see [`super::protocol::lsp_character`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PositionEncoding {
    /// Bytes, which is what the editor already has: nothing to convert.
    Utf8,
    /// UTF-16 code units, the protocol's default and the only one a server must support.
    Utf16,
}

/// Whether the server has finished starting, told from its progress notifications.
///
/// This is the honest answer and the initialize reply is not: rust-analyzer answers
/// `initialize` in milliseconds and then indexes for tens of seconds, and a request in
/// between comes back empty or refused - which reads in a pane as "no definition found"
/// rather than as "not yet".
///
/// So readiness is: nothing the server announced is still outstanding, **and** it has been
/// quiet for [`SETTLING`] since the last thing it said. The quiet is what makes this right
/// for both kinds of server. One that announces nothing at all is ready once the window has
/// passed. One that announces several pieces of work in a row - rust-analyzer fetches
/// metadata, then loads, then indexes - passes through `outstanding == 0` in the gap between
/// each pair of them, and calling that ready is how a request lands mid-indexing and comes
/// back `content modified`.
struct Readiness {
    /// When the server last said anything about its own progress, starting from
    /// `initialized`. The clock the quiet is measured on.
    last_spoke: Instant,
    /// Work the server began and has not ended.
    outstanding: usize,
}

impl Readiness {
    fn is_ready(&self) -> bool {
        self.outstanding == 0 && self.last_spoke.elapsed() >= SETTLING
    }
}

/// What one server is doing right now, as it last said in a `$/progress` notification.
///
/// One per server rather than a list: a server that begins a second piece of work before it
/// has ended the first has moved on to it, and a bar that named both would be a bar nobody
/// reads. The title is carried forward from the `begin`, because a `report` does not repeat
/// it - see [`super::protocol::ProgressNote`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Working {
    /// What the server called this piece of work: "Indexing", "Fetching metadata".
    pub(crate) title: String,
    /// The line it is writing under that title, where it writes one.
    pub(crate) detail: Option<String>,
    /// How far through, where the server says. Most work does not.
    pub(crate) percentage: Option<u8>,
}

/// Fold one `$/progress` notification into what the server is doing.
///
/// A `begin` is a new piece of work, a `report` moves the one already there along, and an
/// `end` leaves the server doing nothing. A `report` with no work behind it is a server
/// whose `begin` this client never saw, and it is left alone rather than made into work
/// with no name.
pub(super) fn follow_progress(working: &mut Option<Working>, kind: Option<&str>, note: ProgressNote) {
    match kind {
        Some("begin") => {
            *working = Some(Working {
                // The protocol makes the title of a `begin` mandatory; a server that leaves
                // it out is still doing something, and "working" is what the bar says then.
                title: note.title.unwrap_or_else(|| "working".to_string()),
                detail: note.message,
                percentage: note.percentage,
            });
        }
        Some("report") => {
            if let Some(working) = working.as_mut() {
                // Only what the report actually carried is taken: a report that says nothing
                // but a percentage leaves the line under the title as it was.
                if note.message.is_some() {
                    working.detail = note.message;
                }
                if note.percentage.is_some() {
                    working.percentage = note.percentage;
                }
            }
        }
        Some("end") => *working = None,
        _ => {}
    }
}

/// The requests waiting for their answers, by the id they went out with.
type Pending = Arc<Mutex<HashMap<i64, mpsc::Sender<Result<Value, String>>>>>;

pub(crate) struct LanguageServer {
    pub(crate) name: &'static str,
    /// What the server agreed to count columns in, out of its `initialize` reply. Written
    /// once, by [`LanguageServer::start`], before anybody else has the server at all.
    encoding: OnceLock<PositionEncoding>,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Mutex<Child>,
    next_id: AtomicI64,
    pending: Pending,
    readiness: Arc<Mutex<Readiness>>,
    /// What the server last said it is doing. Kept beside readiness rather than folded into
    /// it: readiness is a rule about quiet that the window must not second-guess, and this
    /// is the server's own account of the wait, which is what the status bar reads out.
    working: Arc<Mutex<Option<Working>>>,
    /// The text of every document open in this server, by its path relative to the repo.
    ///
    /// Kept because a position has to be turned into the server's units against the line it
    /// falls on, and because full-text sync means the server is told the whole text anyway.
    documents: Mutex<HashMap<String, String>>,
}

impl LanguageServer {
    /// Start a server on a repo and walk it through `initialize` / `initialized`. Blocks
    /// until it has replied, which is the only point at which what it agreed to is known.
    pub(crate) fn start(spec: &'static ServerSpec, repo_root: &std::path::Path) -> Result<Self> {
        let command_path = super::languages::installed_at(spec.command)
            .ok_or_else(|| anyhow!("{} is not installed", spec.command))?;
        let mut child = Command::new(command_path)
            .args(spec.args)
            .current_dir(repo_root)
            .env("PATH", crate::shell_path::installed_tools_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start {}", spec.command))?;

        let stdin = Arc::new(Mutex::new(
            child.stdin.take().context("the server has no stdin")?,
        ));
        let stdout = child.stdout.take().context("the server has no stdout")?;
        let stderr = child.stderr.take().context("the server has no stderr")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let readiness = Arc::new(Mutex::new(Readiness {
            last_spoke: Instant::now(),
            outstanding: 0,
        }));
        let working: Arc<Mutex<Option<Working>>> = Arc::new(Mutex::new(None));

        read_messages(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&readiness),
            Arc::clone(&working),
            Arc::clone(&stdin),
        );
        // Nothing reads a server's diagnostics prose, but a full stderr pipe would stop the
        // server dead, so it is drained and dropped.
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let mut stderr = stderr;
            let _ = stderr.read_to_end(&mut sink);
        });

        let server = Self {
            name: spec.name,
            encoding: OnceLock::new(),
            stdin,
            child: Mutex::new(child),
            next_id: AtomicI64::new(1),
            pending,
            readiness,
            working,
            documents: Mutex::new(HashMap::new()),
        };

        let reply = server.request_with_timeout(
            "initialize",
            super::protocol::initialize_params(repo_root),
            INITIALIZE_TIMEOUT,
        )?;
        let encoding = super::protocol::agreed_encoding(&reply)?;
        let _ = server.encoding.set(encoding);
        server.notify("initialized", json!({}))?;
        // Readiness is counted from here rather than from the spawn: what came before is
        // the server reading the project, and what comes after is what it announces.
        *server.readiness.lock().unwrap() = Readiness {
            last_spoke: Instant::now(),
            outstanding: 0,
        };
        *server.working.lock().unwrap() = None;

        Ok(server)
    }

    pub(crate) fn encoding(&self) -> PositionEncoding {
        *self
            .encoding
            .get()
            .expect("a started server has agreed an encoding")
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.readiness.lock().unwrap().is_ready()
    }

    /// What the server is doing right now, if it has said. `None` for one that is doing
    /// nothing it has announced, which is every server that has finished starting.
    pub(crate) fn working(&self) -> Option<Working> {
        self.working.lock().unwrap().clone()
    }

    /// The text of one open document, as this side last told the server it stands. `None`
    /// for a document that is not open here, which is what makes a request on a file nobody
    /// opened an error rather than a question the server cannot answer.
    pub(crate) fn document_text(&self, file_path: &str) -> Option<String> {
        self.documents.lock().unwrap().get(file_path).cloned()
    }

    pub(crate) fn remember_document(&self, file_path: &str, text: &str) {
        self.documents
            .lock()
            .unwrap()
            .insert(file_path.to_string(), text.to_string());
    }

    pub(crate) fn forget_document(&self, file_path: &str) {
        self.documents.lock().unwrap().remove(file_path);
    }

    /// Whether this file has already been announced to the server, which is the difference
    /// between `didOpen` and `didChange`.
    pub(crate) fn has_document(&self, file_path: &str) -> bool {
        self.documents.lock().unwrap().contains_key(file_path)
    }

    pub(crate) fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, sender);

        let sent = write_message(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        );
        if let Err(error) = sent {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }

        match receiver.recv_timeout(timeout) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(refusal)) => bail!("{} refused {method}: {refusal}", self.name),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!(
                    "{} did not answer {method} within {} seconds",
                    self.name,
                    timeout.as_secs()
                )
            }
        }
    }

    pub(crate) fn notify(&self, method: &str, params: Value) -> Result<()> {
        write_message(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
    }
}

impl Drop for LanguageServer {
    /// Ask the server to stop, and take it down if it will not. The grace is short on
    /// purpose: this runs on whoever dropped the registry, and a window closing must not
    /// wait on a server that has stopped listening.
    fn drop(&mut self) {
        let _ = self.request_with_timeout("shutdown", json!(null), SHUTDOWN_GRACE);
        let _ = self.notify("exit", json!(null));
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn write_message(stdin: &Mutex<ChildStdin>, message: &Value) -> Result<()> {
    let mut stdin = stdin
        .lock()
        .map_err(|_| anyhow!("the server's stdin lock is poisoned"))?;
    stdin
        .write_all(&super::framing::frame(&message.to_string()))
        .context("failed to write to the language server")?;
    stdin
        .flush()
        .context("failed to flush to the language server")?;
    Ok(())
}

/// Read the server's stdout for as long as it has one: answers go to whoever is waiting for
/// them, progress moves readiness along, and the server's own requests are answered so it
/// does not sit waiting on us.
fn read_messages(
    stdout: std::process::ChildStdout,
    pending: Pending,
    readiness: Arc<Mutex<Readiness>>,
    working: Arc<Mutex<Option<Working>>>,
    stdin: Arc<Mutex<ChildStdin>>,
) {
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut frames = Frames::default();
        let mut buffer = vec![0u8; READ_CHUNK];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => frames.push(&buffer[..count]),
            }
            while let Some(message) = frames.next_message() {
                let Ok(message) = serde_json::from_str::<Value>(&message) else {
                    continue;
                };
                handle_message(&message, &pending, &readiness, &working, &stdin);
            }
        }

        // The server is gone. Everyone waiting on it is told now rather than sitting out
        // their whole timeout.
        for (_, waiting) in pending.lock().unwrap().drain() {
            let _ = waiting.send(Err("the language server exited".to_string()));
        }
    });
}

fn handle_message(
    message: &Value,
    pending: &Pending,
    readiness: &Mutex<Readiness>,
    working: &Mutex<Option<Working>>,
    stdin: &Mutex<ChildStdin>,
) {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id");

    match (method, id) {
        // The server asking us something. Every one of these is answered, because a server
        // waiting on a reply that never comes stops making progress.
        (Some(method), Some(id)) => {
            let result = super::protocol::reply_to_server_request(method, message);
            let _ = write_message(
                stdin,
                &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            );
        }
        (Some("$/progress"), None) => {
            let kind = super::protocol::progress_kind(message);
            let mut readiness = readiness.lock().unwrap();
            // Anything it says about its own work restarts the quiet, `report` included: a
            // server part way through indexing is a server that has not finished.
            readiness.last_spoke = Instant::now();
            match kind {
                Some("begin") => readiness.outstanding += 1,
                Some("end") => readiness.outstanding = readiness.outstanding.saturating_sub(1),
                _ => {}
            }
            drop(readiness);
            follow_progress(
                &mut working.lock().unwrap(),
                kind,
                super::protocol::progress_note(message),
            );
        }
        // Any other notification - diagnostics, log messages - is not this client's business.
        (Some(_), None) => {}
        // An answer to something we asked.
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else { return };
            let Some(waiting) = pending.lock().unwrap().remove(&id) else {
                return;
            };
            let answer = match message.get("error") {
                Some(error) => Err(error.to_string()),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = waiting.send(answer);
        }
        (None, None) => {}
    }
}
