//! How a remote backend reaches its server from a browser: requests through the round of the
//! task making them - see [`super::rounds`] - and a shell's socket as the browser's
//! `WebSocket`.

#[path = "web_socket.rs"]
mod web_socket;

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{
    api::SearchLine,
    backend::{Say, SearchListener, Socket},
};

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

    /// Attach to a shell on the server through its socket.
    pub(super) fn attach_shell(&self, url: &str) -> Result<egui_tty::TtyStream> {
        let Socket { heard, said } = self.open_socket(url)?;
        Ok(egui_tty::TtyStream {
            output: heard,
            tty: Arc::new(RemoteShell { said }),
        })
    }

    /// Open a socket to the server. What the socket says goes into the channel of what was
    /// heard as it arrives; the channel closes with the socket.
    pub(super) fn open_socket(&self, url: &str) -> Result<Socket> {
        let url = match super::rounds::expected_profile() {
            Some(profile) => format!(
                "{url}{}moon_profile={}",
                if url.contains('?') { '&' } else { '?' },
                super::urlencode(&profile),
            ),
            None => url.to_owned(),
        };
        web_socket::open(&url, url.contains("/terminals/"))
    }
}

/// A shell attached through a socket: what is typed goes up it as the messages
/// `crate::terminal` reads.
struct RemoteShell {
    said: Arc<dyn Say>,
}

impl RemoteShell {
    fn send(&self, message: &serde_json::Value) -> egui_tty::Result<()> {
        self.said
            .say(message.to_string())
            .map_err(|error| egui_tty::Error::msg(format!("{error:#}")))
    }
}

impl egui_tty::Tty for RemoteShell {
    fn connection_error(&self) -> Option<String> {
        self.said.connection_error()
    }

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
