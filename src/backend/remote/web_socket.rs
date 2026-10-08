//! Browser sockets - detach inactive pages and restore streams when they return.
use crate::{
    backend::{Say, Socket},
    web::activity,
};
use anyhow::{Result, anyhow};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, mpsc},
};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{BinaryType, MessageEvent, WebSocket};

struct Live {
    socket: WebSocket,
    _handlers: (
        Closure<dyn FnMut(MessageEvent)>,
        Closure<dyn FnMut()>,
        Closure<dyn FnMut()>,
    ),
}
impl Drop for Live {
    fn drop(&mut self) {
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onopen(None);
        let _ = self.socket.close();
    }
}
struct State {
    url: String,
    terminal: bool,
    cursor: Option<u64>,
    error: Option<String>,
    live: Option<Live>,
    sender: Option<mpsc::Sender<Vec<u8>>>,
    unsent: Vec<String>,
}

pub(super) fn open(url: &str, terminal: bool) -> Result<Socket> {
    let (sender, heard) = mpsc::channel();
    let state = Rc::new(RefCell::new(State {
        url: url.into(),
        terminal,
        cursor: None,
        error: None,
        live: None,
        sender: Some(sender),
        unsent: Vec::new(),
    }));
    let weak = Rc::downgrade(&state);
    let subscription = activity::subscribe(move |active| {
        let Some(state) = weak.upgrade() else { return };
        if active {
            if let Err(error) = resume(&state) {
                web_sys::console::error_1(&format!("{error:#}").into());
                state.borrow_mut().sender.take();
            }
        } else {
            state.borrow_mut().live.take();
        }
    });
    if activity::active() {
        resume(&state)?;
    }
    Ok(Socket {
        heard,
        said: Arc::new(BrowserSocket {
            state,
            _subscription: subscription,
        }),
    })
}

fn resume(state: &Rc<RefCell<State>>) -> Result<()> {
    let current = state.borrow();
    if current.live.is_some() || current.sender.is_none() || current.error.is_some() {
        return Ok(());
    }
    let url = if current.terminal {
        format!(
            "{}{}moon_terminal_stream=true{}",
            current.url,
            if current.url.contains('?') { '&' } else { '?' },
            current.cursor.map_or(String::new(), |cursor| format!(
                "&moon_terminal_cursor={cursor}"
            ))
        )
    } else {
        current.url.clone()
    };
    let socket = WebSocket::new(&url).map_err(|error| anyhow!("{error:?}"))?;
    socket.set_binary_type(BinaryType::Arraybuffer);
    drop(current);
    let weak = Rc::downgrade(state);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(state) = weak.upgrade() else { return };
        if !activity::active() {
            return;
        }
        let data = event.data();
        let mut state = state.borrow_mut();
        if state.terminal {
            if let Some(text) = data.as_string() {
                state.error = Some(
                    serde_json::from_str::<serde_json::Value>(&text)
                        .ok()
                        .and_then(|value| value["terminal_error"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| {
                            "Invalid terminal stream message. Reconnect to this shell.".into()
                        }),
                );
                return;
            }
            let frame = js_sys::Uint8Array::new(&data).to_vec();
            match crate::api::terminal_stream::decode(&frame) {
                Ok((cursor, bytes)) => {
                    if let Some(sender) = &state.sender {
                        if sender.send(bytes.to_vec()).is_ok() {
                            state.cursor = Some(cursor);
                        }
                    }
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        } else if let Some(sender) = &state.sender {
            let chunk = data
                .as_string()
                .map(String::into_bytes)
                .unwrap_or_else(|| js_sys::Uint8Array::new(&data).to_vec());
            let _ = sender.send(chunk);
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    let weak = Rc::downgrade(state);
    let on_close = Closure::<dyn FnMut()>::new(move || {
        // An unexpected closure still means the shell/display ended. A suspension removes
        // this handler first and keeps the receiver alive.
        if let Some(state) = weak.upgrade() {
            let mut state = state.borrow_mut();
            if state.error.is_none() {
                state.sender.take();
            }
        }
    });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    let weak = Rc::downgrade(state);
    let opened = socket.clone();
    let on_open = Closure::<dyn FnMut()>::new(move || {
        let Some(state) = weak.upgrade() else { return };
        let mut state = state.borrow_mut();
        for message in state.unsent.drain(..) {
            if let Err(error) = opened.send_with_str(&message) {
                web_sys::console::error_1(&error);
            }
        }
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    state.borrow_mut().live = Some(Live {
        socket,
        _handlers: (on_message, on_close, on_open),
    });
    Ok(())
}

struct BrowserSocket {
    state: Rc<RefCell<State>>,
    _subscription: Rc<dyn Fn(bool)>,
}
// SAFETY: this WASM target has one thread; native socket handles use the same Send + Sync trait.
unsafe impl Send for BrowserSocket {}
unsafe impl Sync for BrowserSocket {}
impl Say for BrowserSocket {
    fn connection_error(&self) -> Option<String> {
        self.state.borrow().error.clone()
    }

    fn say(&self, text: String) -> Result<()> {
        let mut state = self.state.borrow_mut();
        if let Some(error) = &state.error {
            return Err(anyhow!("{error}"));
        }
        if !activity::active()
            || state
                .live
                .as_ref()
                .is_some_and(|live| live.socket.ready_state() == WebSocket::CONNECTING)
        {
            state.unsent.push(text);
            return Ok(());
        }
        let live = state
            .live
            .as_ref()
            .ok_or_else(|| anyhow!("socket is closed"))?;
        live.socket
            .send_with_str(&text)
            .map_err(|error| anyhow!("{error:?}"))
    }
}
