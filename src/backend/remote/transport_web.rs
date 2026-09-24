//! How a remote backend reaches its server from a browser: requests through the round of the
//! task making them - see [`super::rounds`] - and a shell's socket as the browser's
//! `WebSocket`.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, mpsc},
};

use anyhow::{Result, anyhow, bail};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{BinaryType, MessageEvent, WebSocket};

use crate::{api::SearchLine, backend::SearchListener};

use super::{
    RemoteBackend,
    addresses::{base_url_for, label_for},
};

/// Nothing is kept between requests: each belongs to the round of the task that made it.
pub(super) struct Connection;

impl RemoteBackend {
    /// `target` is a URL, or a `host` / `host:port` shorthand that means plain HTTP. The page
    /// passes its own origin: the server that served it, so there is no asking whether it is
    /// there.
    pub(crate) fn connect(target: &str) -> Result<Self> {
        let base_url = base_url_for(target)?;
        let label = label_for(&base_url);
        Ok(Self {
            base_url,
            label,
            connection: Connection,
        })
    }

    pub(super) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        decode("GET", path, &self.request("GET", path, None)?)
    }

    pub(super) fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T> {
        let body = serde_json::to_string(body)?;
        decode("POST", path, &self.request("POST", path, Some(&body))?)
    }

    pub(super) fn post(&self, path: &str, body: &impl Serialize) -> Result<()> {
        let body = serde_json::to_string(body)?;
        self.request("POST", path, Some(&body))?;
        Ok(())
    }

    pub(super) fn get_ok(&self, path: &str) -> Result<()> {
        self.request("GET", path, None)?;
        Ok(())
    }

    pub(super) fn delete(&self, path: &str) -> Result<()> {
        self.request("DELETE", path, None)?;
        Ok(())
    }

    /// A search the server streams, a line of JSON per report - see [`SearchLine`]. A round
    /// only hands the body over once it is complete, so the listener hears every report at the
    /// end rather than as each is found.
    pub(super) fn stream_search<T: DeserializeOwned>(
        &self,
        path: &str,
        listener: &mut dyn SearchListener<T>,
    ) -> Result<()> {
        let body = self.request("GET", path, None)?;
        for line in body.lines() {
            if !listener.wanted() {
                return Ok(());
            }
            if line.is_empty() {
                continue;
            }
            let heard: SearchLine<T> = serde_json::from_str(line)
                .map_err(|error| anyhow!("could not read a line of GET {path}: {error}"))?;
            match heard {
                SearchLine::Found(progress) => listener.found(progress),
                SearchLine::Failed(reason) => bail!("{reason}"),
            }
        }
        Ok(())
    }

    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<String> {
        let Self {
            base_url,
            connection: Connection,
            ..
        } = self;
        super::rounds::request(method, &format!("{base_url}{path}"), body)
    }

    /// Attach to a shell on the server through its socket. What the socket says goes into the
    /// output channel as it arrives; the channel closes with the socket.
    pub(super) fn attach_shell(&self, url: &str) -> Result<egui_tty::TtyStream> {
        let socket = WebSocket::new(url).map_err(js_error)?;
        socket.set_binary_type(BinaryType::Arraybuffer);

        let (sender, output) = mpsc::channel::<Vec<u8>>();
        let sender = Rc::new(RefCell::new(Some(sender)));

        let heard = Rc::clone(&sender);
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let data = event.data();
            let chunk = if let Some(text) = data.as_string() {
                text.into_bytes()
            } else {
                js_sys::Uint8Array::new(&data).to_vec()
            };
            if let Some(sender) = heard.borrow().as_ref() {
                // The terminal is gone when this fails; the socket is closed along with it.
                let _ = sender.send(chunk);
            }
        });
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        // Dropping the sender is how the terminal hears the shell is gone.
        let closed = Rc::clone(&sender);
        let on_close = Closure::<dyn FnMut()>::new(move || {
            closed.borrow_mut().take();
        });
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        let unsent = Rc::new(RefCell::new(Vec::<String>::new()));
        let opened_socket = socket.clone();
        let flushed = Rc::clone(&unsent);
        let on_open = Closure::<dyn FnMut()>::new(move || {
            for message in flushed.borrow_mut().drain(..) {
                if let Err(error) = opened_socket.send_with_str(&message) {
                    web_sys::console::error_1(&error);
                }
            }
        });
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        Ok(egui_tty::TtyStream {
            output,
            tty: Arc::new(RemoteShell {
                socket,
                unsent,
                _handlers: (on_message, on_close, on_open),
            }),
        })
    }
}

/// The browser end of a shell's socket. Messages written before the socket opens - the first
/// resize, above all - wait in `unsent` until it does.
struct RemoteShell {
    socket: WebSocket,
    unsent: Rc<RefCell<Vec<String>>>,
    _handlers: (
        Closure<dyn FnMut(MessageEvent)>,
        Closure<dyn FnMut()>,
        Closure<dyn FnMut()>,
    ),
}

// SAFETY: `Tty` asks for Send + Sync because natively a shell's handle is shared with the thread
// reading it. This build is for wasm32-unknown-unknown without the atomics feature, which has
// one thread, so the handle is never on another thread to be shared with.
unsafe impl Send for RemoteShell {}
// SAFETY: as above.
unsafe impl Sync for RemoteShell {}

impl RemoteShell {
    fn send(&self, message: &serde_json::Value) -> egui_tty::Result<()> {
        let message = message.to_string();
        if self.socket.ready_state() == WebSocket::CONNECTING {
            self.unsent.borrow_mut().push(message);
            return Ok(());
        }
        self.socket
            .send_with_str(&message)
            .map_err(|error| egui_tty::Error::msg(format!("{error:?}")))
    }
}

impl Drop for RemoteShell {
    fn drop(&mut self) {
        // Closing tells the server this window let the shell go; the shell itself goes on.
        let _ = self.socket.close();
    }
}

impl egui_tty::Tty for RemoteShell {
    fn write(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "input", "data": text }))
    }

    /// Sent as its own kind of message, so the far end knows this is the terminal answering
    /// the program rather than a person at the keyboard.
    fn reply(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "reply", "data": text }))
    }

    /// Likewise its own kind: the pointer over the shell is not a person answering it.
    fn report(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "report", "data": text }))
    }

    fn resize(&self, cols: u16, rows: u16) -> egui_tty::Result<()> {
        self.send(&json!({ "type": "resize", "cols": cols, "rows": rows }))
    }
}

fn decode<T: DeserializeOwned>(method: &str, path: &str, body: &str) -> Result<T> {
    serde_json::from_str(body)
        .map_err(|error| anyhow!("could not decode the response to {method} {path}: {error}"))
}

fn js_error(error: JsValue) -> anyhow::Error {
    anyhow!("{error:?}")
}
