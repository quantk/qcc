//! The desktop app's backend: a proxy between the webview and qccd.
//!
//! It holds one connection to the daemon, passes requests and notifications through
//! as JSON and reports whether the daemon is reachable. Everything the app knows
//! about threads lives in the daemon; nothing here interprets it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qcc_client::{Client, ClientError};
use qcc_protocol::jsonrpc::RpcError;
use qcc_protocol::paths;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

/// Code of the errors the app itself reports: the daemon cannot be reached.
const NOT_CONNECTED: i32 = -1;
/// Overrides which `qccd` the app starts when none is running.
const DAEMON_BIN_ENV: &str = "QCC_DAEMON_BIN";
const STATE_EVENT: &str = "daemon-state";
const NOTIFICATION_EVENT: &str = "daemon-notification";
const RETRY_EVERY: Duration = Duration::from_millis(500);
/// A daemon that keeps dying is not restarted in a tight loop.
const LAUNCH_EVERY: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
enum DaemonState {
    Connecting,
    Connected { socket: String },
    Disconnected { error: String },
}

struct Daemon {
    socket: PathBuf,
    client: Mutex<Option<Arc<Client>>>,
    state: Mutex<DaemonState>,
}

impl Daemon {
    fn set_state(&self, app: &AppHandle, state: DaemonState) {
        let mut current = self.state.lock().unwrap();
        if *current != state {
            *current = state.clone();
            let _ = app.emit(STATE_EVENT, state);
        }
    }
}

/// Call a daemon method. Parameters, result and error pass through untouched.
#[tauri::command]
async fn rpc(
    daemon: State<'_, Arc<Daemon>>,
    method: String,
    params: Value,
) -> Result<Value, RpcError> {
    let client = daemon.client.lock().unwrap().clone();
    let Some(client) = client else {
        return Err(RpcError::new(NOT_CONNECTED, "not connected to qccd"));
    };
    client.call_raw(&method, params).await.map_err(|error| match error {
        ClientError::Rpc(error) => error,
        other => RpcError::new(NOT_CONNECTED, other.to_string()),
    })
}

#[tauri::command]
fn daemon_state(daemon: State<'_, Arc<Daemon>>) -> DaemonState {
    daemon.state.lock().unwrap().clone()
}

/// Start `qccd` detached: it keeps the threads running after the app is closed.
fn launch_daemon() -> std::io::Result<()> {
    let binary = std::env::var_os(DAEMON_BIN_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            let sibling = std::env::current_exe().ok()?.with_file_name("qccd");
            sibling.exists().then_some(sibling)
        })
        .unwrap_or_else(|| PathBuf::from("qccd"));
    let data_dir = paths::default_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let log =
        std::fs::OpenOptions::new().create(true).append(true).open(data_dir.join("qccd.log"))?;
    let mut child = Command::new(binary)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        // Its own process group: a Ctrl-C aimed at the app does not reach it.
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Hold a connection for as long as the app runs, starting the daemon when there
/// is none to connect to.
async fn keep_connected(app: AppHandle, daemon: Arc<Daemon>) {
    let mut launched: Option<Instant> = None;
    loop {
        match Client::connect_raw(&daemon.socket).await {
            Ok((client, mut notifications)) => {
                *daemon.client.lock().unwrap() = Some(client);
                let socket = daemon.socket.display().to_string();
                daemon.set_state(&app, DaemonState::Connected { socket });
                while let Some(notification) = notifications.recv().await {
                    let payload = serde_json::json!({
                        "method": notification.method,
                        "params": notification.params,
                    });
                    let _ = app.emit(NOTIFICATION_EVENT, payload);
                }
                *daemon.client.lock().unwrap() = None;
                let error = "the connection to qccd was lost".to_owned();
                daemon.set_state(&app, DaemonState::Disconnected { error });
            }
            Err(error) => {
                let mut message = error.to_string();
                if launched.is_none_or(|at| at.elapsed() >= LAUNCH_EVERY) {
                    launched = Some(Instant::now());
                    match launch_daemon() {
                        Ok(()) => message = "starting qccd".to_owned(),
                        Err(error) => message = format!("{message}; cannot start qccd: {error}"),
                    }
                }
                if launched.is_some_and(|at| at.elapsed() < Duration::from_secs(3)) {
                    daemon.set_state(&app, DaemonState::Connecting);
                } else {
                    daemon.set_state(&app, DaemonState::Disconnected { error: message });
                }
            }
        }
        tokio::time::sleep(RETRY_EVERY).await;
    }
}

fn main() {
    let daemon = Arc::new(Daemon {
        socket: paths::default_socket(),
        client: Mutex::new(None),
        state: Mutex::new(DaemonState::Connecting),
    });
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(daemon.clone())
        .setup(move |app| {
            tauri::async_runtime::spawn(keep_connected(app.handle().clone(), daemon));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc, daemon_state])
        .run(tauri::generate_context!())
        .expect("the qcc desktop app runs");
}
