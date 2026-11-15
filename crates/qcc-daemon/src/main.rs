use std::path::PathBuf;

use clap::Parser;
use qcc_daemon::Config;
use qcc_protocol::paths;
use tokio::signal::unix::{SignalKind, signal};

/// The qcc daemon: owns threads, the event journal and the plugins.
#[derive(Parser)]
#[command(name = "qccd", version)]
struct Args {
    /// Journal, installed plugins and plugin data [default: ~/.local/share/qcc].
    #[arg(long, env = paths::DATA_DIR_ENV)]
    data_dir: Option<PathBuf>,

    /// Unix socket to listen on [default: $XDG_RUNTIME_DIR/qcc/qccd.sock].
    #[arg(long, env = paths::SOCKET_ENV)]
    socket: Option<PathBuf>,

    /// Log filter in `tracing` syntax, for example `debug` or `qcc_daemon=trace`.
    #[arg(long, env = "QCC_LOG", default_value = "info")]
    log: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&args.log))
        .with_writer(std::io::stderr)
        .init();

    // An explicit data directory keeps its socket to itself, so two daemons with
    // different data never fight over the default socket.
    let socket = match (&args.socket, &args.data_dir) {
        (Some(socket), _) => socket.clone(),
        (None, Some(data_dir)) => data_dir.join("qccd.sock"),
        (None, None) => paths::default_socket(),
    };
    let config = Config {
        data_dir: args.data_dir.unwrap_or_else(paths::default_data_dir),
        socket,
        cache_dir: paths::default_cache_dir(),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut terminate = signal(SignalKind::terminate())?;
        let shutdown = async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        };
        qcc_daemon::serve(config, || {}, shutdown).await
    })
}
