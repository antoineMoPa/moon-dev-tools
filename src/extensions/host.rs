//! The engine a script runs in, and everything it can call on beyond Rhai itself.
//!
//! What reaches the window - a file to open, a shell to start, a toast - is sent to it as an
//! [`Effect`] and done there, on the UI thread, once the script has returned. Everything else
//! here happens on the extension's own thread: running a program or making a request blocks
//! that thread and nothing else.

use std::{
    cell::Cell,
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
    sync::mpsc::Sender,
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::worker::{Effect, Input, Message, Output};

/// How much work one call into a script may do before it is stopped.
///
/// Rhai counts operations rather than time, so this is the same limit on a fast machine and a
/// slow one. A view of every entry of a big folder is well under it; a loop that never ends is
/// stopped in about a second instead of holding the pane's thread forever.
const MAX_OPERATIONS: u64 = 20_000_000;

/// How deeply an expression may nest, at the top of a script and inside a function.
///
/// Rhai's own defaults are lower in a debug build than in a release one, and the debug ones
/// refuse a table row inside a view - `table([..], [#{ cells: [text("a")] }])` is already too
/// deep for them. A view is a tree written as one expression, so these are set for views; the
/// extension's thread has the stack for them - see [`super::worker::STACK_SIZE`].
const MAX_EXPR_DEPTH: usize = 256;
const MAX_FUNCTION_EXPR_DEPTH: usize = 128;

/// How deeply functions may call each other: `tick` calling `load` calling `changes_in`, with
/// the prelude's builders under them.
const MAX_CALL_LEVELS: usize = 64;

/// How long `run` lets a program go before it is stopped, unless the script says otherwise.
/// The pane's thread waits on a program, so one that never ends would otherwise hold the pane
/// still for good - the operation cap only counts what the script itself does.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a running program is looked at for having ended.
const RUN_POLL: Duration = Duration::from_millis(5);

/// How long a request may take, start to end of body, before `fetch` gives up on it. A
/// request is made on the pane's own thread, so a server that never answers would otherwise
/// hold the pane still for good.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// What a script's programs and folders are relative to.
#[derive(Clone)]
pub(crate) struct Host {
    /// The project the window is on: where programs run, and what `init` is told.
    pub(crate) project_root: PathBuf,
    /// The PATH programs are looked for on - see [`crate::shell_path::installed_tools_path`].
    pub(crate) path: String,
}

/// What running a program came to.
#[derive(Serialize)]
struct Ran {
    ok: bool,
    /// The exit code; -1 for a program that could not be started, or was ended by a signal.
    code: i64,
    stdout: String,
    stderr: String,
}

/// What `run` can be told beyond the program and its arguments - all of it optional.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RunOptions {
    /// How long before the program is stopped; [`RUN_TIMEOUT`] when left out.
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Where it runs: a folder of the project, or anywhere by an absolute path. The project
    /// when left out.
    #[serde(default)]
    cwd: Option<String>,
    /// Set in its environment, over what the window's own is.
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// What a request can say beyond its address, as `fetch`'s second argument - all of it
/// optional, the way `fetch()`'s `init` is.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FetchOptions {
    /// `GET` when left out; any case.
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Sent as it is. A script sends JSON by writing it out - `body: data.to_json()` - with
    /// the `Content-Type` header saying so.
    #[serde(default)]
    body: Option<String>,
}

/// What a request came back with, the way a `fetch()` response reads once its body has been
/// taken as text.
#[derive(Serialize)]
struct Fetched {
    /// A status in the 200s.
    ok: bool,
    status: i64,
    /// By header name, in lowercase.
    headers: BTreeMap<String, String>,
    body: String,
}

/// One entry of a folder, as `read_dir` answers.
#[derive(Serialize)]
struct Entry {
    name: String,
    path: String,
    /// Following a link: a link to a folder is a folder.
    is_dir: bool,
    is_link: bool,
    /// Where a link points, as the link says it; empty for anything else.
    link_to: String,
    size: u64,
    /// Seconds since the epoch; 0 when the OS would not say.
    modified: u64,
}

pub(super) fn engine(
    host: &Host,
    outbox: Sender<Output>,
    inbox: Sender<Message>,
    tick: Rc<Cell<Option<Duration>>>,
) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_expr_depths(MAX_EXPR_DEPTH, MAX_FUNCTION_EXPR_DEPTH);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    // A script is a file, so there is no call for building more script out of strings.
    engine.disable_symbol("eval");

    // `print` is how a script says what it is doing, and the window's messages are where that
    // is read back - there is no terminal behind a pane.
    let printing = outbox.clone();
    engine.on_print(move |said| {
        let _ = printing.send(Output::Effect(Effect::Log(said.to_string())));
    });
    let debugging = outbox.clone();
    engine.on_debug(move |said, _source, position| {
        let _ = debugging.send(Output::Effect(Effect::Log(format!("{position}: {said}"))));
    });

    let running = host.clone();
    engine.register_fn(
        "run",
        move |program: &str, args: Array| -> Result<Dynamic, Box<EvalAltResult>> {
            let args = strings_of(args)?;
            to_dynamic(&run(&running, program, &args, RunOptions::default()))
        },
    );
    let running = host.clone();
    engine.register_fn(
        "run",
        move |program: &str, args: Array, options: Map| -> Result<Dynamic, Box<EvalAltResult>> {
            let args = strings_of(args)?;
            to_dynamic(&run(&running, program, &args, options_of(options, "run")?))
        },
    );

    // The same, on a thread of its own: the script goes on answering while the program runs,
    // and hears how it went as an `update` with the event it gave, plus a `result`.
    let (running_later, run_inbox) = (host.clone(), inbox.clone());
    engine.register_fn(
        "run_then",
        move |program: &str, args: Array, event: Map| -> Result<(), Box<EvalAltResult>> {
            run_later(
                &running_later,
                &run_inbox,
                program,
                args,
                RunOptions::default(),
                event,
            )
        },
    );
    let (running_later, run_inbox) = (host.clone(), inbox.clone());
    engine.register_fn(
        "run_then",
        move |program: &str,
              args: Array,
              options: Map,
              event: Map|
              -> Result<(), Box<EvalAltResult>> {
            let options = options_of(options, "run_then")?;
            run_later(&running_later, &run_inbox, program, args, options, event)
        },
    );

    // Node's `fetch()`, kept to what a pane needs: a request out, and its status, headers and
    // body as text back. A status that is not a success is still a response, as it is in Node;
    // a request that got no response at all - no connection, a timeout - is an error the
    // script can `catch`.
    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .expect("an HTTP client builds");
    let fetching = client.clone();
    engine.register_fn(
        "fetch",
        move |url: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            to_dynamic(&fetch(&fetching, url, FetchOptions::default()).map_err(runtime_error)?)
        },
    );
    let fetching = client.clone();
    engine.register_fn(
        "fetch",
        move |url: &str, options: Map| -> Result<Dynamic, Box<EvalAltResult>> {
            let options = options_of(options, "fetch")?;
            to_dynamic(&fetch(&fetching, url, options).map_err(runtime_error)?)
        },
    );
    // The same, on a thread of its own, like `run_then`: `update` hears it with the event it
    // gave, plus the `result` - or the `error`, for a request that got no response.
    let (fetching, fetch_inbox) = (client.clone(), inbox.clone());
    engine.register_fn(
        "fetch_then",
        move |url: &str, event: Map| -> Result<(), Box<EvalAltResult>> {
            fetch_later(&fetching, &fetch_inbox, url, FetchOptions::default(), event)
        },
    );
    engine.register_fn(
        "fetch_then",
        move |url: &str, options: Map, event: Map| -> Result<(), Box<EvalAltResult>> {
            let options = options_of(options, "fetch_then")?;
            fetch_later(&client, &inbox, url, options, event)
        },
    );

    // Rhai has a `parse_json` of its own, which reads an object and nothing else - and the
    // answer of most of the APIs a script would `fetch` from is a list.
    engine.register_fn(
        "parse_json",
        |text: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let parsed: Value = serde_json::from_str(text)
                .map_err(|error| runtime_error(format!("parse_json: {error}")))?;
            to_dynamic(&parsed)
        },
    );

    engine.register_fn(
        "read_dir",
        |path: &str| -> Result<Dynamic, Box<EvalAltResult>> {
            let entries = read_dir(Path::new(path))
                .map_err(|error| runtime_error(format!("could not read {path}: {error}")))?;
            to_dynamic(&entries)
        },
    );

    let opening = outbox.clone();
    engine.register_fn("open_file", move |path: &str| {
        let _ = opening.send(Output::Effect(Effect::OpenFile {
            path: PathBuf::from(path),
            line: None,
        }));
    });
    let opening = outbox.clone();
    engine.register_fn(
        "open_file",
        move |path: &str, line: i64| -> Result<(), Box<EvalAltResult>> {
            // Counted from one, as the number in the fringe is.
            let line = usize::try_from(line)
                .ok()
                .filter(|line| *line >= 1)
                .ok_or_else(|| {
                    runtime_error(format!(
                        "open_file: line {line} is not a line; the first is 1"
                    ))
                })?;
            let _ = opening.send(Output::Effect(Effect::OpenFile {
                path: PathBuf::from(path),
                line: Some(line),
            }));
            Ok(())
        },
    );
    let copying = outbox.clone();
    engine.register_fn("copy", move |text: &str| {
        let _ = copying.send(Output::Effect(Effect::Copy(text.to_string())));
    });
    let starting = outbox.clone();
    engine.register_fn("open_shell", move |command: &str| {
        let _ = starting.send(Output::Effect(Effect::OpenShell(command.to_string())));
    });
    engine.register_fn("notify", move |said: &str| {
        let _ = outbox.send(Output::Effect(Effect::Notify(said.to_string())));
    });

    engine.register_fn("every", move |milliseconds: i64| {
        tick.set((milliseconds > 0).then(|| Duration::from_millis(milliseconds as u64)));
    });
    engine.register_fn("now", || -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_secs() as i64
    });

    engine.register_fn("parent_of", |path: &str| -> String {
        Path::new(path)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            // The root has no parent, and going up from it stays there.
            .unwrap_or_else(|| path.to_string())
    });
    engine.register_fn("name_of", |path: &str| -> String {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    engine.register_fn("join_path", |dir: &str, name: &str| -> String {
        Path::new(dir).join(name).to_string_lossy().to_string()
    });
    engine.register_fn("shell_quote", |text: &str| -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    });

    engine
}

fn run(host: &Host, program: &str, args: &[String], options: RunOptions) -> Ran {
    let timeout = options
        .timeout_ms
        .map_or(RUN_TIMEOUT, Duration::from_millis);
    let mut command = Command::new(program);
    command
        .args(args)
        // `join` keeps an absolute path as it is, and puts a relative one under the project.
        .current_dir(host.project_root.join(options.cwd.unwrap_or_default()))
        .env("PATH", &host.path)
        .envs(options.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A program that is not installed is something a script checks for - colima on a machine
    // that runs docker some other way - so it is an answer rather than an error.
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Ran::failed(format!("could not run {program}: {error}")),
    };
    // Read as it is written: a program with more to say than a pipe holds would otherwise sit
    // waiting for a reader that is waiting for it to end.
    let stdout = read_to_end(child.stdout.take().expect("stdout is piped"));
    let stderr = read_to_end(child.stderr.take().expect("stderr is piped"));

    let until = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ran {
                    ok: status.success(),
                    code: status.code().map_or(-1, i64::from),
                    stdout: stdout.join().expect("a reader thread ends"),
                    stderr: stderr.join().expect("a reader thread ends"),
                };
            }
            Ok(None) if Instant::now() < until => std::thread::sleep(RUN_POLL),
            // What it printed is left behind with the readers: a program it started may still
            // hold the pipes open, and waiting on them is the wait this is here to end.
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ran::failed(format!(
                    "{program} was still running after {} ms, and was stopped",
                    timeout.as_millis()
                ));
            }
            Err(error) => return Ran::failed(format!("could not wait on {program}: {error}")),
        }
    }
}

impl Ran {
    fn failed(said: String) -> Self {
        Self {
            ok: false,
            code: -1,
            stdout: String::new(),
            stderr: said,
        }
    }
}

fn read_to_end(mut from: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = from.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).to_string()
    })
}

/// `run_then`: the program on a thread of its own, and how it went sent back as an event.
fn run_later(
    host: &Host,
    inbox: &Sender<Message>,
    program: &str,
    args: Array,
    options: RunOptions,
    event: Map,
) -> Result<(), Box<EvalAltResult>> {
    let args = strings_of(args)?;
    let mut event = event_to_answer(event, "run_then", &["result"])?;
    let (host, program, inbox) = (host.clone(), program.to_string(), inbox.clone());
    std::thread::Builder::new()
        .name(format!("extension run {program}"))
        .spawn(move || {
            let ran = run(&host, &program, &args, options);
            event.insert(
                "result".to_string(),
                serde_json::to_value(ran).expect("a run is plain data"),
            );
            // The pane may have closed while the program ran; nobody is left to tell.
            let _ = inbox.send(Message::Input(Input::Event(Value::Object(event))));
        })
        .map_err(|error| runtime_error(format!("could not start {error}")))?;
    Ok(())
}

/// `fetch_then`: the request on a thread of its own, and what came of it sent back as an event.
fn fetch_later(
    client: &reqwest::blocking::Client,
    inbox: &Sender<Message>,
    url: &str,
    options: FetchOptions,
    event: Map,
) -> Result<(), Box<EvalAltResult>> {
    let mut event = event_to_answer(event, "fetch_then", &["result", "error"])?;
    let (client, url, inbox) = (client.clone(), url.to_string(), inbox.clone());
    std::thread::Builder::new()
        .name(format!("extension fetch {url}"))
        .spawn(move || {
            match fetch(&client, &url, options) {
                Ok(fetched) => event.insert(
                    "result".to_string(),
                    serde_json::to_value(fetched).expect("a response is plain data"),
                ),
                Err(error) => event.insert("error".to_string(), Value::String(error)),
            };
            // The pane may have closed while the request was out; nobody is left to tell.
            let _ = inbox.send(Message::Input(Input::Event(Value::Object(event))));
        })
        .map_err(|error| runtime_error(format!("could not start {error}")))?;
    Ok(())
}

/// An event a script handed over to be sent back with an answer in it, as the object it will
/// arrive as. One that already has a field the answer goes in is refused: the answer would
/// write over it without a word.
fn event_to_answer(
    event: Map,
    function: &str,
    answered_in: &[&str],
) -> Result<serde_json::Map<String, Value>, Box<EvalAltResult>> {
    let Value::Object(event) = from_dynamic(&Dynamic::from_map(event))? else {
        unreachable!("a map is read back as an object");
    };
    if let Some(taken) = answered_in.iter().find(|field| event.contains_key(**field)) {
        return Err(runtime_error(format!(
            "{function}: the event already has a `{taken}`, which is where the answer goes"
        )));
    }
    Ok(event)
}

/// Make one request, and read its whole body as text. The error is said the way a script
/// would want it in a toast: what was asked for, and why it got nothing.
fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    options: FetchOptions,
) -> Result<Fetched, String> {
    let fetched = || -> anyhow::Result<Fetched> {
        let method = options.method.as_deref().unwrap_or("GET").to_uppercase();
        let mut request = client.request(reqwest::Method::from_bytes(method.as_bytes())?, url);
        for (name, value) in options.headers {
            request = request.header(name, value);
        }
        if let Some(body) = options.body {
            request = request.body(body);
        }
        let response = request.send()?;
        let status = response.status();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).to_string(),
                )
            })
            .collect();
        Ok(Fetched {
            ok: status.is_success(),
            status: i64::from(status.as_u16()),
            headers,
            body: response.text()?,
        })
    };
    fetched().map_err(|error| format!("fetch {url}: {error:#}"))
}

/// The options map a function was given, read into what it takes - a field it does not know
/// is refused, naming the function.
fn options_of<T: serde::de::DeserializeOwned>(
    options: Map,
    function: &str,
) -> Result<T, Box<EvalAltResult>> {
    rhai::serde::from_dynamic(&Dynamic::from_map(options))
        .map_err(|error| runtime_error(format!("{function}'s options: {error}")))
}

/// A folder's entries, in name order.
fn read_dir(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let is_link = entry.file_type()?.is_symlink();
        // Followed, so a link reads as what it points at. A link to nothing has nothing to
        // read, and stands as an empty file.
        let metadata = std::fs::metadata(&path).ok();
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: metadata.as_ref().is_some_and(std::fs::Metadata::is_dir),
            is_link,
            link_to: match is_link {
                true => std::fs::read_link(&path)?.to_string_lossy().to_string(),
                false => String::new(),
            },
            size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
            modified: metadata
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs()),
            path: path.to_string_lossy().to_string(),
        });
    }
    entries.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(entries)
}

fn strings_of(args: Array) -> Result<Vec<String>, Box<EvalAltResult>> {
    args.into_iter()
        .map(|arg| {
            arg.into_string().map_err(|held| {
                runtime_error(format!("a program's arguments are text, not {held}"))
            })
        })
        .collect()
}

pub(super) fn to_dynamic(value: &impl Serialize) -> Result<Dynamic, Box<EvalAltResult>> {
    rhai::serde::to_dynamic(value)
}

fn from_dynamic(value: &Dynamic) -> Result<Value, Box<EvalAltResult>> {
    rhai::serde::from_dynamic(value)
}

fn runtime_error(said: String) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(said),
        Position::NONE,
    ))
}
