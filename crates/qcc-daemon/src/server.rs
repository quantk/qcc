//! The unix socket: accepts connections and turns lines into core commands.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, bail};
use qcc_protocol::jsonrpc::{Incoming, Response, RpcError, codes};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::core::{Command, Responder};

/// Bind the socket, replacing a stale one left by a daemon that died.
pub(crate) async fn bind(socket: &Path) -> anyhow::Result<UnixListener> {
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    if socket.exists() {
        if UnixStream::connect(socket).await.is_ok() {
            bail!("another qccd is already listening on {}", socket.display());
        }
        std::fs::remove_file(socket)
            .with_context(|| format!("cannot remove stale socket {}", socket.display()))?;
    }
    let listener =
        UnixListener::bind(socket).with_context(|| format!("cannot bind {}", socket.display()))?;
    // The API is as powerful as the user's shell: nobody else may connect.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub(crate) async fn accept(listener: UnixListener, commands: UnboundedSender<Command>) {
    static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let connection = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve(stream, connection, commands.clone()));
            }
            Err(error) => {
                tracing::error!(%error, "accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn serve(stream: UnixStream, connection: u64, commands: UnboundedSender<Command>) {
    let (reader, mut writer) = stream.into_split();
    // Everything addressed to this connection — responses and notifications alike —
    // goes through one queue, so the core decides their order.
    let (out, mut outgoing) = unbounded_channel::<String>();
    let write = tokio::spawn(async move {
        while let Some(mut line) = outgoing.recv().await {
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match Incoming::parse(&line) {
            Ok(Incoming::Request(request)) => {
                let responder = Responder::new(out.clone(), request.id);
                let command = Command::Rpc {
                    connection,
                    method: request.method,
                    params: request.params,
                    responder,
                };
                if commands.send(command).is_err() {
                    break;
                }
            }
            // The daemon sends no requests, so it expects no responses.
            Ok(Incoming::Response(_)) => {}
            Err(error) => {
                // Never drop a bad request silently: the peer would wait forever.
                let response = Response::err(
                    Value::Null,
                    RpcError::new(codes::PARSE_ERROR, format!("invalid JSON-RPC message: {error}")),
                );
                let _ = out.send(serde_json::to_string(&response).expect("responses serialize"));
            }
        }
    }
    let _ = commands.send(Command::Disconnected { connection });
    // The writer ends by itself once the core drops its last handle to this queue.
    drop(write);
}
