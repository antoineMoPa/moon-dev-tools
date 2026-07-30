use std::{env, io::Read, io::Write, path::PathBuf};

use axum::{
    extract::{
        Path as AxumPath, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;

use crate::api::{AppError, AppState};

const OUTPUT_CHUNK_SIZE: usize = 8 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

fn login_shell() -> String {
    env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

pub(crate) async fn terminal_socket(
    AxumPath(session_id): AxumPath<String>,
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let repo_path =
        crate::api::with_session(&state, &session_id, |session| Ok(session.repo_path.clone()))?;

    Ok(upgrade.on_upgrade(move |socket| async move {
        if let Err(error) = run_terminal(socket, repo_path).await {
            eprintln!("[moonreview] terminal session ended: {error}");
        }
    }))
}

async fn run_terminal(socket: WebSocket, repo_path: PathBuf) -> anyhow::Result<()> {
    let pty = native_pty_system().openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut command = CommandBuilder::new(login_shell());
    command.arg("-l");
    command.cwd(&repo_path);
    command.env("TERM", "xterm-256color");
    let mut child = pty.slave.spawn_command(command)?;
    // The slave handle must be dropped so the reader sees EOF once the shell exits.
    drop(pty.slave);

    let mut reader = pty.master.try_clone_reader()?;
    let mut writer = pty.master.take_writer()?;
    let master = pty.master;

    let (output_sender, mut output_receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut buffer = vec![0u8; OUTPUT_CHUNK_SIZE];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(count) => {
                    if output_sender.blocking_send(buffer[..count].to_vec()).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let (mut socket_sender, mut socket_receiver) = socket.split();

    let mut pump_output = tokio::spawn(async move {
        while let Some(chunk) = output_receiver.recv().await {
            if socket_sender.send(Message::Binary(chunk.into())).await.is_err() {
                return;
            }
        }
        let _ = socket_sender.close().await;
    });

    loop {
        tokio::select! {
            _ = &mut pump_output => break,
            incoming = socket_receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                let Message::Text(text) = message else { continue };
                match serde_json::from_str::<ClientMessage>(&text)? {
                    ClientMessage::Input { data } => writer.write_all(data.as_bytes())?,
                    ClientMessage::Resize { cols, rows } => master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })?,
                }
            }
        }
    }

    pump_output.abort();
    child.kill()?;
    child.wait()?;
    Ok(())
}
