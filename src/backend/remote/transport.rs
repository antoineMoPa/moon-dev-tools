//! How a remote backend reaches its server: the address it is given turned into URLs, the
//! requests and streamed searches sent over HTTP, and the websocket a shell is attached through.

use std::{
    io::{BufRead, BufReader},
    net::TcpStream,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{
    StatusCode,
    blocking::Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use tungstenite::client::IntoClientRequest;

use crate::{api::SearchLine, backend::SearchListener};

use super::{
    RemoteBackend,
    addresses::{base_url_for, label_for},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a streamed search may take: as long as `ag` needs on a large tree, which is not
/// the half minute a plain request gets. A search nobody wants is stopped long before.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What requests go out through: one HTTP client, kept for its connection pool, which shows
/// the pass key on every request it sends - see [`crate::pass_keys`].
pub(super) struct Connection {
    client: Client,
    /// Kept as well for the one thing the client does not send: a shell's websocket, and a
    /// window this one starts on the same server.
    pub(super) pass_key: String,
}

/// Where a key comes from when `--pass-key` gives none - the same variable a window this one
/// starts is handed its key in, which keeps the key out of the command line other users see.
pub(crate) const PASS_KEY_ENV_VAR: &str = "MOON_PASS_KEY";

impl RemoteBackend {
    /// `target` is a URL, or a `host` / `host:port` shorthand that means plain HTTP.
    ///
    /// The key is tried once here, so a wrong one is said to be wrong while connecting rather
    /// than as the first thing the window asks for failing.
    pub(crate) fn connect(target: &str, pass_key: String) -> Result<Self> {
        let base_url = base_url_for(target)?;
        let label = label_for(&base_url);
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .default_headers(HeaderMap::from_iter([(AUTHORIZATION, bearer(&pass_key)?)]))
            .build()
            .context("failed to build HTTP client")?;

        let health = client
            .get(format!("{base_url}/healthz"))
            .send()
            .with_context(|| format!("failed to reach a moonreview server at {base_url}"))?;
        if !health.status().is_success() {
            bail!("{base_url} answered {} for /healthz", health.status());
        }
        let admitted = client
            .get(format!("{base_url}/api/pass-key"))
            .send()
            .with_context(|| format!("failed to reach a moonreview server at {base_url}"))?;
        if admitted.status() == StatusCode::UNAUTHORIZED {
            bail!(
                "{base_url} did not accept the pass key ({}); run `moon generate-pass-key` on \
                 that machine and pass what it prints with --pass-key or {PASS_KEY_ENV_VAR}",
                admitted.text().unwrap_or_default().trim()
            );
        }
        admitted.error_for_status().map_err(remote_refusal)?;

        Ok(Self {
            base_url,
            label,
            connection: Connection { client, pass_key },
        })
    }

    pub(super) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.connection
            .client
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
        self.connection
            .client
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
        self.connection
            .client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .with_context(|| format!("POST {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    pub(super) fn get_ok(&self, path: &str) -> Result<()> {
        self.connection
            .client
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
            .connection
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
        self.connection
            .client
            .delete(format!("{}{path}", self.base_url))
            .send()
            .with_context(|| format!("DELETE {path} failed"))?
            .error_for_status()
            .map_err(remote_refusal)?;
        Ok(())
    }

    /// Attach to a shell on the server through its socket. A thread reads the socket into the
    /// output channel; writes share the socket with it through a lock.
    pub(super) fn attach_shell(&self, url: &str) -> Result<egui_tty::TtyStream> {
        let mut request = url
            .into_client_request()
            .with_context(|| format!("{url} is not a websocket address"))?;
        request
            .headers_mut()
            .insert(AUTHORIZATION, bearer(&self.connection.pass_key)?);
        let (socket, _) = tungstenite::connect(request)
            .with_context(|| format!("failed to attach to the remote shell at {url}"))?;
        set_read_timeout(&socket)?;

        let socket = Arc::new(Mutex::new(socket));
        let (sender, output) = mpsc::channel();
        let reader_socket = Arc::clone(&socket);

        thread::spawn(move || {
            loop {
                let message = {
                    let Ok(mut socket) = reader_socket.lock() else {
                        return;
                    };
                    match socket.read() {
                        Ok(message) => Some(message),
                        // A read timeout is how the writer gets the lock between frames.
                        Err(tungstenite::Error::Io(error))
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            None
                        }
                        Err(_) => return,
                    }
                };

                let Some(message) = message else {
                    thread::sleep(Duration::from_millis(4));
                    continue;
                };

                let chunk = match message {
                    tungstenite::Message::Binary(bytes) => bytes.to_vec(),
                    tungstenite::Message::Text(text) => text.as_bytes().to_vec(),
                    tungstenite::Message::Close(_) => return,
                    _ => continue,
                };
                if sender.send(chunk).is_err() {
                    return;
                }
            }
        });

        Ok(egui_tty::TtyStream {
            output,
            tty: Arc::new(RemoteShell { socket }),
        })
    }
}

/// `Authorization` for `pass_key`, marked sensitive so that nothing printing a request prints
/// the key along with it.
fn bearer(pass_key: &str) -> Result<HeaderValue> {
    let mut authorization = HeaderValue::from_str(&format!("Bearer {pass_key}"))
        .context("a pass key is letters, digits, `-`, `_` and a `.`, which this is not")?;
    authorization.set_sensitive(true);
    Ok(authorization)
}

/// The server puts the real reason in the body, so a bare status line is not enough.
fn remote_refusal(error: reqwest::Error) -> anyhow::Error {
    anyhow!("{error}")
}

/// Both halves of an attached remote shell run on the socket, so writes go through the
/// same lock the reader thread holds between frames.
struct RemoteShell {
    socket: Arc<Mutex<SharedSocket>>,
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
fn set_read_timeout(socket: &SharedSocket) -> Result<()> {
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
