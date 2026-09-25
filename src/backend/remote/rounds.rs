//! How a task asks the server for things in a browser, where nothing may wait.
//!
//! Every [`crate::backend::Backend`] call returns its answer, and natively a task's work runs on
//! a thread of its own that can wait for one. A page has one thread, and a page waiting on the
//! network is a page nobody can click. So a task's work runs in rounds instead:
//!
//! - The first time the work asks for something, the request goes out and the call fails with
//!   [`StillFetching`]. The work carries on or gives up as it would on any failure.
//! - When every answer asked for so far is in, the work runs again from the top. This time the
//!   calls it already made are answered from the round, in the order they were made, and a
//!   call it gets to for the first time goes out as before.
//! - The first run that asks for nothing new is the one whose result counts.
//!
//! So the work has to ask the same things in the same order each time it runs, which work that
//! only decides what to ask from what it was told does. One that asks differently is a bug the
//! round refuses to paper over.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use anyhow::{Result, anyhow, bail};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::XmlHttpRequest;

/// What a call that went out in this run answers with. The work sees it as any other error.
#[derive(Debug)]
pub(crate) struct StillFetching;

impl std::fmt::Display for StillFetching {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("still waiting on the server")
    }
}

impl std::error::Error for StillFetching {}

thread_local! {
    /// The round of the task whose work is running, while it runs.
    static RUNNING: RefCell<Option<Round>> = const { RefCell::new(None) };
}

/// The requests one task made, and their answers as they come in.
#[derive(Clone)]
pub(crate) struct Round {
    state: Rc<RoundState>,
}

struct RoundState {
    asked: RefCell<Vec<Asked>>,
    /// How far into `asked` this run of the work has got.
    cursor: Cell<usize>,
    /// Requests out and not yet answered.
    waiting: Cell<usize>,
    /// What to do once `waiting` is back to none: run the work again.
    when_answered: RefCell<Option<Box<dyn FnOnce()>>>,
}

struct Asked {
    /// Method, URL and body: what a later run's call has to match to be handed the answer.
    request: String,
    /// The body of a success, or why it failed. `None` while the request is out.
    answer: Option<Result<String, String>>,
}

impl Round {
    pub(crate) fn new() -> Self {
        Self {
            state: Rc::new(RoundState {
                asked: RefCell::new(Vec::new()),
                cursor: Cell::new(0),
                waiting: Cell::new(0),
                when_answered: RefCell::new(None),
            }),
        }
    }

    /// Run `work` once, with its requests going through this round.
    pub(crate) fn run<R>(&self, work: impl FnOnce() -> R) -> R {
        self.state.cursor.set(0);
        RUNNING.with(|running| {
            let previous = running.borrow_mut().replace(self.clone());
            assert!(previous.is_none(), "a task's work ran inside another task's work");
        });
        let result = work();
        RUNNING.with(|running| running.borrow_mut().take());
        result
    }

    /// Whether the last run made a request that is still out, which makes its result not the
    /// work's answer, whatever the work did with the failure.
    pub(crate) fn is_waiting(&self) -> bool {
        self.state.waiting.get() > 0
    }

    /// Call `then` once every request out is answered.
    pub(crate) fn when_answered(&self, then: impl FnOnce() + 'static) {
        let previous = self.state.when_answered.borrow_mut().replace(Box::new(then));
        assert!(previous.is_none(), "a round was told twice what to do once answered");
    }

    fn answer(&self, method: &str, url: &str, body: Option<&str>) -> Result<String> {
        let request = format!("{method} {url} {}", body.unwrap_or(""));
        let at = self.state.cursor.get();
        self.state.cursor.set(at + 1);

        if let Some(asked) = self.state.asked.borrow().get(at) {
            if asked.request != request {
                panic!(
                    "a task asked for `{request}` where its earlier run asked for `{}` - work \
                     run in a browser has to ask the same things in the same order each time",
                    asked.request
                );
            }
            return match &asked.answer {
                Some(Ok(text)) => Ok(text.clone()),
                Some(Err(reason)) => bail!("{reason}"),
                None => Err(StillFetching.into()),
            };
        }

        self.state.asked.borrow_mut().push(Asked {
            request,
            answer: None,
        });
        self.state.waiting.set(self.state.waiting.get() + 1);
        send(method, url, body, self.clone(), at)
            .map_err(|error| anyhow!("{method} {url} could not be sent: {error:?}"))?;
        Err(StillFetching.into())
    }

    fn answered(&self, at: usize, answer: Result<String, String>) {
        self.state.asked.borrow_mut()[at].answer = Some(answer);
        let waiting = self.state.waiting.get() - 1;
        self.state.waiting.set(waiting);
        if waiting > 0 {
            return;
        }
        // Taken before it is called: running the work again may set the next one.
        let then = self.state.when_answered.borrow_mut().take();
        if let Some(then) = then {
            then();
        }
    }
}

/// What the server answers a browser whose login is over: its session ran out, or was kicked
/// out - see `crate::server::auth`. Every request the window makes from then on is answered
/// the same, so the window is not left standing with nothing it can ask for.
const LOGIN_OVER: u16 = 401;

/// Reload the page: the server shows a browser whose login is over the login page in place of
/// the window, at the same address - see `crate::server::web_page` - so the person is put where
/// a new pass key goes, and comes back to the same repo and frame once it is in.
fn log_in_again() {
    let reloaded = web_sys::window()
        .expect("a round runs in a page")
        .location()
        .reload();
    if let Err(error) = reloaded {
        web_sys::console::error_1(&error);
    }
}

/// Ask the server, through the round of the task whose work is running. Every backend call in
/// a browser is made from a task's work - see `crate::native::tasks`.
pub(super) fn request(method: &str, url: &str, body: Option<&str>) -> Result<String> {
    let round = RUNNING.with(|running| running.borrow().clone()).unwrap_or_else(|| {
        panic!("{method} {url} was asked outside a task, where a browser has nothing to wait with")
    });
    round.answer(method, url, body)
}

fn send(
    method: &str,
    url: &str,
    body: Option<&str>,
    round: Round,
    at: usize,
) -> Result<(), wasm_bindgen::JsValue> {
    let request = XmlHttpRequest::new()?;
    request.open_with_async(method, url, true)?;

    let heard = request.clone();
    let described = format!("{method} {url}");
    let on_end = Closure::once_into_js(move || {
        let status = heard.status().unwrap_or(0);
        let text = heard.response_text().ok().flatten().unwrap_or_default();
        // The server puts the reason for a refusal in the body, so that is what one reads as.
        let answer = match status {
            0 => Err(format!("{described} failed: the server could not be reached")),
            200..=299 => Ok(text),
            _ => Err(format!("{described} answered {status}: {text}")),
        };
        if status == LOGIN_OVER {
            log_in_again();
        }
        round.answered(at, answer);
    });
    request.set_onloadend(Some(on_end.unchecked_ref()));

    match body {
        Some(body) => {
            request.set_request_header("content-type", "application/json")?;
            request.send_with_opt_str(Some(body))
        }
        None => request.send(),
    }
}
