//! Where the daemon lives by default. Shared so clients find it without configuration.

use std::env;
use std::path::PathBuf;

/// Overrides the socket path. The daemon also passes it to every agent session.
pub const SOCKET_ENV: &str = "QCC_SOCKET";
/// Overrides the data directory.
pub const DATA_DIR_ENV: &str = "QCC_DATA_DIR";
/// Set by the daemon for every agent session: the thread the agent runs in.
pub const THREAD_ID_ENV: &str = "QCC_THREAD_ID";

fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `$QCC_DATA_DIR`, else `$XDG_DATA_HOME/qcc`, else `~/.local/share/qcc`.
pub fn default_data_dir() -> PathBuf {
    env_path(DATA_DIR_ENV)
        .or_else(|| env_path("XDG_DATA_HOME").map(|dir| dir.join("qcc")))
        .unwrap_or_else(|| home().join(".local/share/qcc"))
}

/// `$QCC_SOCKET`, else `$XDG_RUNTIME_DIR/qcc/qccd.sock`, else the socket inside the
/// default data directory.
pub fn default_socket() -> PathBuf {
    env_path(SOCKET_ENV)
        .or_else(|| env_path("XDG_RUNTIME_DIR").map(|dir| dir.join("qcc/qccd.sock")))
        .unwrap_or_else(|| default_data_dir().join("qccd.sock"))
}
