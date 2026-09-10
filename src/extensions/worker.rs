//! The thread an extension runs on.
//!
//! Each open extension pane has one. The script lives there and nowhere else - the engine and
//! everything it holds stay on it - and the window talks to it in messages: an event or a key
//! goes in, and a view to draw and the effects to act on come back. A script that runs a slow
//! program blocks its own thread, never the window's.

use std::{
    cell::Cell,
    rc::Rc,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant, SystemTime},
};

use rhai::{AST, CallFnOptions, Dynamic, Engine, FuncArgs, Map, Scope};
use serde_json::Value;

use super::{
    Extension, Source,
    host::{self, Host},
    view::Element,
};

/// How often a script that is a file is looked at for having changed.
const RELOAD_CHECK: Duration = Duration::from_millis(400);

const PRELUDE: &str = include_str!("prelude.rhai");

/// The stack an extension's thread is given. Rhai parses and evaluates by recursion, and a
/// thread that runs out of stack takes the whole window with it rather than only the pane - so
/// this is sized for the depths [`super::host`] allows, far past the two megabytes a thread
/// gets by default. It is address space set aside, and only what a script reaches is used.
pub(super) const STACK_SIZE: usize = 32 * 1024 * 1024;

/// What the window hands a script.
pub(crate) enum Input {
    /// What a clicked button or row carried, for `update`.
    Event(Value),
    /// A key pressed while the pane had the keyboard, for `on_key`: the character it typed, or
    /// the name of one that types none - `Enter`, `Up`, `Shift+Tab`.
    Key(String),
    /// Whether the pane can be seen. A hidden pane is not ticked, and is ticked the moment it
    /// shows again, so what it shows is caught up rather than a tick behind.
    Visible(bool),
    /// Read the script again and start it over from `init`, dropping the state it held.
    Restart,
}

/// What reaches the thread: an input, or the pane closing.
pub(super) enum Message {
    Input(Input),
    Close,
}

/// What a script hands back.
pub(crate) enum Output {
    /// The view, when there is one to draw, and what is wrong with the script right now.
    ///
    /// A view that could not be made leaves `view` empty, and the pane goes on drawing the last
    /// one under the error: a typo in a script being written is not a reason to lose the pane.
    Drawn {
        view: Option<Element>,
        error: Option<String>,
    },
    /// A version of the script has been read and started. `takes_keys` is whether it has an
    /// `on_key`: a script with none leaves every key to the window.
    Loaded {
        takes_keys: bool,
    },
    Effect(Effect),
}

/// Something only the window can do.
#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    /// A file, and the line to open it at, counted from one.
    OpenFile {
        path: std::path::PathBuf,
        line: Option<usize>,
    },
    /// A shell in the project, with this line typed into it and sent.
    OpenShell(String),
    /// Text for the clipboard.
    Copy(String),
    Notify(String),
    /// A line the script printed, for the window's messages.
    Log(String),
    /// Something that went wrong with the script, for the window's messages. The pane says it
    /// too, but a pane can be hidden, and its error can be put right before anyone looks.
    Failed(String),
}

/// The window's end of a running extension. Dropping it ends the script.
pub(crate) struct Running {
    inbox: Sender<Message>,
    outbox: Receiver<Output>,
}

impl Running {
    /// Start `extension` on a thread of its own. `repaint` is called whenever it has said
    /// something, so the window draws it without waiting for the next input.
    pub(crate) fn start(
        extension: Extension,
        host: Host,
        repaint: impl Fn() + Send + 'static,
    ) -> Self {
        let (inbox, inputs) = mpsc::channel();
        let (outputs, outbox) = mpsc::channel();
        let feeding = inbox.clone();
        thread::Builder::new()
            .name(format!("extension {}", extension.name))
            .stack_size(STACK_SIZE)
            .spawn(move || {
                Worker::start(extension, host, feeding, outputs, Box::new(repaint)).work(inputs);
            })
            .expect("failed to start an extension's thread");
        Self { inbox, outbox }
    }

    pub(crate) fn send(&self, input: Input) {
        // A script that has stopped has already said why, in its last view.
        let _ = self.inbox.send(Message::Input(input));
    }

    /// Everything the script has said since the last call.
    pub(crate) fn take_outputs(&self) -> Vec<Output> {
        self.outbox.try_iter().collect()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.inbox.send(Message::Close);
    }
}

/// One version of a script, read, and with its top-level code run once to see that it runs.
///
/// That code runs again before each call - see [`call_options`] - which is how its constants
/// reach the script's functions as `global::NAME`.
struct Script {
    ast: AST,
}

/// How every function of a script is called: with its top-level code run first, in a scope of
/// its own. Rhai puts a script's constants in `global::` only while that code is running, so a
/// function called without it could not see them; the scope is thrown away after, so what
/// the code does is the same every call.
fn call_options<'a>() -> CallFnOptions<'a> {
    CallFnOptions::new().eval_ast(true).rewind_scope(true)
}

struct Worker {
    extension: Extension,
    host: Host,
    engine: Engine,
    /// The version running, once one has read and started.
    script: Option<Script>,
    state: Dynamic,
    /// When the file was last read, for a script that is one.
    read_at: Option<SystemTime>,
    /// How often `tick` is asked for, as the script last set it with `every`.
    tick: Rc<Cell<Option<Duration>>>,
    next_tick: Option<Instant>,
    visible: bool,
    /// What is wrong, from the last thing that failed. It stays until something puts it right:
    /// an input handled, or a version of the script started. A tick going well is not that -
    /// with a tick every couple of seconds, an error would be up for no longer than that.
    error: Option<String>,
    outputs: Sender<Output>,
    repaint: Box<dyn Fn() + Send>,
    /// The last thing sent, so the same view is not sent again every tick.
    sent: Option<(Option<Element>, Option<String>)>,
    /// Set once the window has stopped listening.
    closed: bool,
}

impl Worker {
    fn start(
        extension: Extension,
        host: Host,
        inbox: Sender<Message>,
        outputs: Sender<Output>,
        repaint: Box<dyn Fn() + Send>,
    ) -> Self {
        let tick = Rc::new(Cell::new(None));
        let mut engine = host::engine(&host, outputs.clone(), inbox, Rc::clone(&tick));
        // An `import` of a script that is a file names files beside it.
        if let Source::File(path) = &extension.source
            && let Some(folder) = path.parent()
        {
            engine.set_module_resolver(rhai::module_resolvers::FileModuleResolver::new_with_path(
                folder,
            ));
        }
        let mut worker = Self {
            extension,
            host,
            engine,
            script: None,
            state: Dynamic::UNIT,
            read_at: None,
            tick,
            next_tick: None,
            visible: true,
            error: None,
            outputs,
            repaint,
            sent: None,
            closed: false,
        };
        worker.begin();
        worker
    }

    fn work(mut self, inputs: Receiver<Message>) {
        while !self.closed {
            let wait = self.next_tick.map_or(RELOAD_CHECK, |at| {
                at.saturating_duration_since(Instant::now())
                    .min(RELOAD_CHECK)
            });
            match inputs.recv_timeout(wait) {
                Ok(Message::Close) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Message::Input(input)) => self.take(input),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.reload_if_changed();
            self.tick_if_due();
        }
    }

    fn take(&mut self, input: Input) {
        match input {
            Input::Restart => self.begin(),
            Input::Visible(visible) => self.show(visible),
            Input::Event(event) => self.act(|worker| {
                let event = host::to_dynamic(&event).map_err(|error| error.to_string())?;
                worker.call("update", (event,))
            }),
            Input::Key(key) => {
                if self.defines("on_key", 1) {
                    self.act(|worker| worker.call("on_key", (key,)));
                }
            }
        }
    }

    /// Something the person did: done, and the view sent. Going well puts right whatever was
    /// wrong - they are looking at the pane, and it did what they asked.
    fn act(&mut self, step: impl FnOnce(&mut Self) -> Result<Dynamic, String>) {
        if self.script.is_none() {
            return;
        }
        match step(self) {
            Ok(_) => self.error = None,
            Err(error) => self.fail(error),
        }
        self.draw();
    }

    /// Read the script and start it from `init`: when the pane opens, and when it is restarted.
    fn begin(&mut self) {
        self.script = None;
        self.state = Dynamic::UNIT;
        match self.read().and_then(|mut script| {
            let state = self.init(&mut script)?;
            Ok((script, state))
        }) {
            Ok((script, state)) => {
                self.state = state;
                self.run_version(script);
            }
            Err(error) => self.fail(error),
        }
        self.draw();
    }

    /// Read a script that is a file again, if it has changed since it was last read.
    ///
    /// The state is kept - the point is to see the change on the pane as it stands - but the
    /// new `init` is run on the side, and a field it answers with that the state has not got is
    /// put into it: a field added while the pane is open is there to read, rather than `()`.
    fn reload_if_changed(&mut self) {
        let Source::File(path) = &self.extension.source else {
            return;
        };
        let modified = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok();
        if modified == self.read_at {
            return;
        }
        if self.script.is_none() {
            return self.begin();
        }
        match self.read().and_then(|mut script| {
            let fresh = self.init(&mut script)?;
            Ok((script, fresh))
        }) {
            Ok((script, fresh)) => {
                adopt_new_fields(&mut self.state, fresh);
                self.run_version(script);
            }
            Err(error) => self.fail(format!(
                "{error}\n(the version that last loaded is the one still running)"
            )),
        }
        self.draw();
    }

    /// Make `script` the version running, and tell the window about it.
    fn run_version(&mut self, script: Script) {
        self.script = Some(script);
        self.error = None;
        self.schedule_tick();
        let takes_keys = self.defines("on_key", 1);
        if self.outputs.send(Output::Loaded { takes_keys }).is_err() {
            self.closed = true;
        }
    }

    fn show(&mut self, visible: bool) {
        let shown = visible && !self.visible;
        self.visible = visible;
        if shown {
            self.tick_now();
        }
    }

    fn tick_if_due(&mut self) {
        if !self.visible || self.script.is_none() {
            return;
        }
        let Some(at) = self.next_tick else {
            self.schedule_tick();
            return;
        };
        if Instant::now() >= at {
            self.tick_now();
        }
    }

    fn tick_now(&mut self) {
        self.schedule_tick();
        if !self.defines("tick", 0) {
            return;
        }
        if let Err(error) = self.call("tick", ()) {
            self.fail(error);
        }
        self.draw();
    }

    fn schedule_tick(&mut self) {
        self.next_tick = self.tick.get().map(|every| Instant::now() + every);
    }

    /// Read the script, merged behind the prelude, and run its top-level code once.
    fn read(&mut self) -> Result<Script, String> {
        let source = match &self.extension.source {
            Source::Shipped(source) => source.to_string(),
            Source::File(path) => {
                self.read_at = std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok();
                std::fs::read_to_string(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()))?
            }
        };
        let prelude = self.engine.compile(PRELUDE).expect("the prelude compiles");
        let script = self
            .engine
            .compile(&source)
            .map_err(|error| format!("{}: {error}", self.extension.name))?;
        let ast = prelude.merge(&script);
        self.engine
            .run_ast_with_scope(&mut Scope::new(), &ast)
            .map_err(|error| format!("{}: {error}", self.extension.name))?;
        Ok(Script { ast })
    }

    /// What `init` answers with, for `script`. It is told the project's folder, if it asks.
    fn init(&self, script: &mut Script) -> Result<Dynamic, String> {
        let Script { ast } = script;
        let scope = &mut Scope::new();
        let answered = match defines(ast, "init", 1) {
            true => {
                let root = self.host.project_root.to_string_lossy().to_string();
                self.engine.call_fn_with_options::<Dynamic>(
                    call_options(),
                    scope,
                    ast,
                    "init",
                    (root,),
                )
            }
            false => {
                self.engine
                    .call_fn_with_options::<Dynamic>(call_options(), scope, ast, "init", ())
            }
        };
        answered.map_err(|error| format!("init: {error}"))
    }

    /// Call one of the script's functions with its state as `this`.
    fn call(&mut self, name: &str, args: impl FuncArgs) -> Result<Dynamic, String> {
        let Script { ast } = self
            .script
            .as_ref()
            .expect("a script is only called once it has started");
        self.engine
            .call_fn_with_options::<Dynamic>(
                call_options().bind_this_ptr(&mut self.state),
                &mut Scope::new(),
                ast,
                name,
                args,
            )
            .map_err(|error| format!("{name}: {error}"))
    }

    fn defines(&self, name: &str, arity: usize) -> bool {
        self.script
            .as_ref()
            .is_some_and(|script| defines(&script.ast, name, arity))
    }

    fn render(&mut self) -> Result<Element, String> {
        let drawn = self.call("view", ())?;
        let element: Element =
            rhai::serde::from_dynamic(&drawn).map_err(|error| format!("view: {error}"))?;
        element.check().map_err(|error| format!("view: {error}"))?;
        Ok(element)
    }

    /// Put up an error, and write it to the window's messages the first time it is seen.
    fn fail(&mut self, error: String) {
        if self.error.as_ref() != Some(&error) {
            let logged = format!("{}: {error}", self.extension.name);
            let _ = self.outputs.send(Output::Effect(Effect::Failed(logged)));
        }
        self.error = Some(error);
    }

    /// Send the view as it now stands, with what is wrong.
    fn draw(&mut self) {
        let view = match self.script.is_some() {
            true => match self.render() {
                Ok(view) => Some(view),
                Err(error) => {
                    self.fail(error);
                    None
                }
            },
            false => None,
        };
        let said = (view, self.error.clone());
        if self.sent.as_ref() == Some(&said) {
            return;
        }
        self.sent = Some(said.clone());
        let (view, error) = said;
        if self.outputs.send(Output::Drawn { view, error }).is_err() {
            self.closed = true;
            return;
        }
        (self.repaint)();
    }
}

fn defines(ast: &AST, name: &str, arity: usize) -> bool {
    ast.iter_functions()
        .any(|function| function.name == name && function.params.len() == arity)
}

/// Put into `state` each field of `fresh` it has not got, with `fresh`'s value. The fields it
/// has keep the values they have. Only maps have fields: a state that is anything else is left
/// as it is.
fn adopt_new_fields(state: &mut Dynamic, fresh: Dynamic) {
    let (Some(mut held), Some(fresh)) = (state.write_lock::<Map>(), fresh.try_cast::<Map>()) else {
        return;
    };
    for (name, value) in fresh {
        held.entry(name).or_insert(value);
    }
}
