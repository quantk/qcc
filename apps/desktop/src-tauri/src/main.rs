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

use base64::Engine as _;
use qcc_client::{Client, ClientError};
use qcc_protocol::jsonrpc::RpcError;
use qcc_protocol::methods::{DaemonStatus, Empty};
use qcc_protocol::{PROTOCOL_VERSION, paths};
use serde::Serialize;
use serde_json::Value;
use tauri::http::{Response, StatusCode, header};
use tauri::{AppHandle, Emitter, Manager, State};

/// Code of the errors the app itself reports: the daemon cannot be reached.
const NOT_CONNECTED: i32 = -1;
/// Overrides which `qccd` the app starts when none is running.
const DAEMON_BIN_ENV: &str = "QCC_DAEMON_BIN";
const STATE_EVENT: &str = "daemon-state";
const NOTIFICATION_EVENT: &str = "daemon-notification";
const RETRY_EVERY: Duration = Duration::from_millis(500);
/// A daemon that keeps dying is not restarted in a tight loop.
const LAUNCH_EVERY: Duration = Duration::from_secs(15);
/// How often a daemon of another build is asked again, in case it was replaced.
const MISMATCH_RETRY_EVERY: Duration = Duration::from_secs(3);

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

/// The largest picture the composer shows a thumbnail of before sending.
const PREVIEW_LIMIT: u64 = 20 * 1024 * 1024;

/// A picture the user picked to attach, base64-encoded, for its thumbnail in the
/// composer: the webview cannot read files. Only files the user chose reach here, and
/// only images: anything else, or anything too large, has no preview.
#[tauri::command]
async fn attachment_preview(path: String) -> Option<String> {
    let path = PathBuf::from(path);
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return None;
    }
    let read = tauri::async_runtime::spawn_blocking(move || {
        let metadata = std::fs::metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > PREVIEW_LIMIT {
            return None;
        }
        std::fs::read(&path).ok()
    });
    let data = read.await.ok()??;
    Some(base64::engine::general_purpose::STANDARD.encode(data))
}

#[tauri::command]
fn daemon_state(daemon: State<'_, Arc<Daemon>>) -> DaemonState {
    daemon.state.lock().unwrap().clone()
}

/// The scheme plugin web bundles are served under: `qcc-plugin://<plugin>/<path>`.
/// The plugin id is the host, so every plugin is an origin of its own: apart from
/// the app and apart from every other plugin.
const PLUGIN_SCHEME: &str = "qcc-plugin";
/// What a page of a web bundle may load: its own files and nothing from the network.
/// The page runs in a sandboxed frame; its only way out is the message bridge.
const BUNDLE_CSP: &str =
    "default-src 'self' 'unsafe-inline' data: blob:; frame-src 'none'; form-action 'none'";

fn plain(status: StatusCode, message: String) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.into_bytes())
        .expect("a static response is valid")
}

/// Fetch one file of a plugin's web bundle from the daemon.
async fn serve_asset(daemon: &Daemon, plugin: &str, path: &str) -> Response<Vec<u8>> {
    let file = path.trim_start_matches('/');
    let client = daemon.client.lock().unwrap().clone();
    let Some(client) = client else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "not connected to qccd".into());
    };
    let params = serde_json::json!({ "plugin": plugin, "path": file });
    let asset = match client.call_raw("ui.asset", params).await {
        Ok(asset) => asset,
        Err(error) => return plain(StatusCode::NOT_FOUND, error.to_string()),
    };
    let data = asset["data"].as_str().unwrap_or_default();
    let Ok(body) = base64::engine::general_purpose::STANDARD.decode(data) else {
        return plain(StatusCode::BAD_GATEWAY, "qccd sent an unreadable file".into());
    };
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            asset["content_type"].as_str().unwrap_or("application/octet-stream"),
        )
        .header(header::CONTENT_SECURITY_POLICY, BUNDLE_CSP)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap_or_else(|error| plain(StatusCode::BAD_GATEWAY, error.to_string()))
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

/// Why the daemon on the socket cannot serve this app, if it cannot.
async fn protocol_mismatch(client: &Client) -> Option<String> {
    let status = match client.call::<DaemonStatus>(Empty {}).await {
        Ok(status) => status,
        Err(error) => return Some(format!("qccd did not say which version it is: {error}")),
    };
    (status.protocol != PROTOCOL_VERSION).then(|| {
        format!(
            "the running qccd (pid {pid}, version {version}) speaks protocol {theirs}, this app \
             needs {PROTOCOL_VERSION}. Stop it (`kill {pid}`; agents it runs are interrupted) \
             and the app starts the new one",
            pid = status.pid,
            version = status.version,
            theirs = status.protocol,
        )
    })
}

/// Hold a connection for as long as the app runs, starting the daemon when there
/// is none to connect to.
async fn keep_connected(app: AppHandle, daemon: Arc<Daemon>) {
    let mut launched: Option<Instant> = None;
    loop {
        match Client::connect_raw(&daemon.socket).await {
            Ok((client, mut notifications)) => {
                // A daemon from another build answers, but not with the methods this
                // app calls. It is not restarted from here: it may be running agents.
                if let Some(error) = protocol_mismatch(&client).await {
                    daemon.set_state(&app, DaemonState::Disconnected { error });
                    tokio::time::sleep(MISMATCH_RETRY_EVERY).await;
                    continue;
                }
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
        .register_asynchronous_uri_scheme_protocol(PLUGIN_SCHEME, |context, request, responder| {
            let daemon = context.app_handle().state::<Arc<Daemon>>().inner().clone();
            let plugin = request.uri().host().unwrap_or_default().to_owned();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn(async move {
                responder.respond(serve_asset(&daemon, &plugin, &path).await);
            });
        })
        .setup(move |app| {
            tauri::async_runtime::spawn(keep_connected(app.handle().clone(), daemon));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc, daemon_state, attachment_preview])
        .run(tauri::generate_context!())
        .expect("the qcc desktop app runs");
}
