//! How a remote backend reaches its server: the address it is given turned into URLs, the
//! requests and streamed searches sent over HTTP, and the websocket a shell is attached through.

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{api::SearchLine, search::SearchListener};

use super::{REQUEST_TIMEOUT, RemoteBackend, SEARCH_TIMEOUT};

impl RemoteBackend {
    /// `target` is a URL, or a `host` / `host:port` shorthand that means plain HTTP.
    pub(crate) fn connect(target: &str) -> Result<Self> {
        let base_url = base_url_for(target)?;
        let label = label_for(&base_url);
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;

        let health = client
            .get(format!("{base_url}/healthz"))
            .send()
            .with_context(|| format!("failed to reach a moonreview server at {base_url}"))?;
        if !health.status().is_success() {
            bail!("{base_url} answered {} for /healthz", health.status());
        }

        Ok(Self {
            base_url,
            label,
            client,
        })
    }

    pub(super) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("GET {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?
            .json()
            .with_context(|| format!("could not decode the response to GET {path}"))
    }

    pub(super) fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T> {
        self.client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .with_context(|| format!("POST {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?
            .json()
            .with_context(|| format!("could not decode the response to POST {path}"))
    }

    pub(super) fn post(&self, path: &str, body: &impl Serialize) -> Result<()> {
        self.client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .with_context(|| format!("POST {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    pub(super) fn get_ok(&self, path: &str) -> Result<()> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("GET {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    /// A search the server streams, a line of JSON per report - see [`SearchLine`] - and an
    /// empty line on every tick nothing changed. Each line is handed to the listener as it
    /// comes; when the listener stops wanting the search, the response is dropped, which
    /// closes the connection, which is what stops the search on the far side.
    pub(super) fn stream_search<T: DeserializeOwned>(
        &self,
        path: &str,
        listener: &mut dyn SearchListener<T>,
    ) -> Result<()> {
        let response = self
            .client
            .get(format!("{}{path}", self.base_url))
            .timeout(SEARCH_TIMEOUT)
            .send()
            .with_context(|| format!("GET {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        for line in BufReader::new(response).lines() {
            let line = line.with_context(|| format!("GET {path} broke off"))?;
            if !listener.wanted() {
                return Ok(());
            }
            if line.is_empty() {
                continue;
            }
            let heard: SearchLine<T> = serde_json::from_str(&line)
                .with_context(|| format!("could not read a line of GET {path}"))?;
            match heard {
                SearchLine::Found(progress) => listener.found(progress),
                SearchLine::Failed(reason) => bail!("{reason}"),
            }
        }
        Ok(())
    }

    pub(super) fn delete(&self, path: &str) -> Result<()> {
        self.client
            .delete(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("DELETE {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }
}

/// The server puts the real reason in the body, so a bare status line is not enough.
fn remote_refusal(error: reqwest::Error) -> anyhow::Error {
    anyhow!("{error}")
}

fn base_url_for(target: &str) -> Result<String> {
    let target = target.trim().trim_end_matches('/');
    if target.is_empty() {
        bail!("remote server address must not be empty");
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok(target.to_string());
    }
    if target.contains("://") {
        bail!("remote server address must be http:// or https://, got {target}");
    }
    // A bare `host` or `host:port` is the common case over an SSH tunnel.
    if target.contains(':') {
        Ok(format!("http://{target}"))
    } else {
        Ok(format!("http://{target}:{}", crate::api::DEFAULT_PORT))
    }
}

fn label_for(base_url: &str) -> String {
    base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string()
}

pub(super) fn websocket_url(base_url: &str, path: &str) -> String {
    let socket_base = if let Some(rest) = base_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base_url.to_string()
    };
    format!("{socket_base}{path}")
}

/// Both halves of an attached remote shell run on the socket, so writes go through the
/// same lock the reader thread holds between frames.
pub(super) struct RemoteShell {
    pub(super) socket: Arc<Mutex<SharedSocket>>,
}

type SharedSocket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

impl egui_tty::Tty for RemoteShell {
    fn write(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "input", "data": text }))
            .map_err(egui_tty::Error::msg)
    }

    /// Sent as its own kind of message, so the far end knows this is the terminal answering
    /// the program rather than a person at the keyboard.
    fn reply(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "reply", "data": text }))
            .map_err(egui_tty::Error::msg)
    }

    /// Likewise its own kind: the pointer over the shell is not a person answering it.
    fn report(&self, data: &[u8]) -> egui_tty::Result<()> {
        let text = String::from_utf8_lossy(data).to_string();
        self.send(&json!({ "type": "report", "data": text }))
            .map_err(egui_tty::Error::msg)
    }

    fn resize(&self, cols: u16, rows: u16) -> egui_tty::Result<()> {
        self.send(&json!({ "type": "resize", "cols": cols, "rows": rows }))
            .map_err(egui_tty::Error::msg)
    }
}

impl RemoteShell {
    fn send(&self, message: &serde_json::Value) -> Result<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| anyhow!("terminal socket lock poisoned"))?;
        socket
            .send(tungstenite::Message::Text(message.to_string().into()))
            .context("failed to send to the remote shell")?;
        Ok(())
    }
}

/// Reads have to give the lock up so keystrokes can go out, so the socket polls instead of
/// blocking forever.
pub(super) fn set_read_timeout(socket: &SharedSocket) -> Result<()> {
    let timeout = Some(Duration::from_millis(20));
    match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream.set_read_timeout(timeout)?,
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => {
            stream.get_ref().set_read_timeout(timeout)?
        }
        _ => bail!("unsupported terminal socket transport"),
    }
    Ok(())
}

pub(super) fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_host_becomes_an_http_url_on_the_default_port() {
        assert_eq!(
            base_url_for("dev-box").expect("expected a URL"),
            format!("http://dev-box:{}", crate::api::DEFAULT_PORT)
        );
    }

    #[test]
    fn host_and_port_shorthand_keeps_the_port() {
        assert_eq!(
            base_url_for("127.0.0.1:9000").expect("expected a URL"),
            "http://127.0.0.1:9000"
        );
    }

    #[test]
    fn explicit_urls_are_kept_and_trailing_slashes_dropped() {
        assert_eq!(
            base_url_for("https://review.example.com/").expect("expected a URL"),
            "https://review.example.com"
        );
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        let error = base_url_for("ssh://dev-box").expect_err("expected ssh:// to be rejected");
        assert!(error.to_string().contains("must be http:// or https://"));
    }

    #[test]
    fn websocket_urls_follow_the_http_scheme() {
        assert_eq!(
            websocket_url("http://dev-box:42000", "/socket"),
            "ws://dev-box:42000/socket"
        );
        assert_eq!(
            websocket_url("https://dev-box", "/socket"),
            "wss://dev-box/socket"
        );
    }

    #[test]
    fn file_paths_with_spaces_and_unicode_are_encoded() {
        assert_eq!(urlencode("src/a b/é.rs"), "src%2Fa%20b%2F%C3%A9.rs");
    }
}
