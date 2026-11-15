//! Finding and loading plugins.
//!
//! Bundled plugins take the same road as everyone else: they are installed into the
//! plugin directory and loaded from there. The one difference is trust — the user
//! chose the daemon, so its own plugins get what they request. Any other plugin gets
//! only what a grant file gives it.

use std::path::Path;
use std::sync::Arc;

use qcc_domain::{Delta, ThreadId};
use qcc_plugin_host::services::DirKv;
use qcc_plugin_host::{Grant, MANIFEST_FILE, Permissions, PluginHost, Services};
use tokio::sync::mpsc::UnboundedSender;

use crate::Config;
use crate::core::Command;

pub(crate) fn is_bundled(id: &str) -> bool {
    qcc_plugins_bundle::plugin(id).is_some()
}

/// What the user granted a third-party plugin: `<data>/grants/<id>.toml`, in the
/// format of a manifest's `[permissions]` table. No file means no permissions.
fn grant_for(config: &Config, id: &str) -> Grant {
    if is_bundled(id) {
        return Grant::AllRequested;
    }
    let path = config.grants_dir().join(format!("{id}.toml"));
    let granted = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str::<Permissions>(&text).unwrap_or_else(|error| {
            tracing::error!(plugin = id, %error, "ignoring an invalid grant file");
            Permissions::default()
        }),
        Err(_) => {
            tracing::warn!(plugin = id, grant = %path.display(), "plugin has no granted permissions");
            Permissions::default()
        }
    };
    Grant::Only(granted)
}

pub(crate) fn load_all(
    config: &Config,
    commands: &UnboundedSender<Command>,
) -> anyhow::Result<PluginHost> {
    let plugins_dir = config.plugins_dir();
    qcc_plugins_bundle::install(&plugins_dir)?;

    let mut host = PluginHost::with_cache(Some(&config.cache_dir.join("wasm")))?;
    let mut dirs: Vec<_> = std::fs::read_dir(&plugins_dir)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();

    for dir in dirs {
        // The directory name is only a location; the manifest names the plugin.
        let id = match plugin_id(&dir) {
            Some(id) => id,
            None => continue,
        };
        let grant = grant_for(config, &id);
        let loaded = host.load_dir(&dir, &grant, |manifest| {
            let deltas = commands.clone();
            let crashes = commands.clone();
            Services {
                deltas: Arc::new(move |plugin: &str, thread: ThreadId, deltas_: Vec<Delta>| {
                    let _ = deltas.send(Command::Deltas {
                        plugin: plugin.to_owned(),
                        thread,
                        deltas: deltas_,
                    });
                }),
                kv: Box::new(DirKv::new(config.plugin_data_dir(&manifest.id).join("kv"))),
                processes: Arc::new(qcc_plugin_host::services::OsProcesses),
                on_crash: Arc::new(move |plugin: &str, reason: &str| {
                    let _ = crashes.send(Command::PluginCrashed {
                        plugin: plugin.to_owned(),
                        reason: reason.to_owned(),
                    });
                }),
            }
        });
        match loaded {
            Ok(plugin) => tracing::info!(
                plugin = plugin.id(),
                version = %plugin.manifest().version,
                "plugin loaded"
            ),
            // One broken plugin must not take the daemon down.
            Err(error) => tracing::error!(dir = %dir.display(), %error, "plugin failed to load"),
        }
    }
    Ok(host)
}

fn plugin_id(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    match qcc_plugin_host::Manifest::parse(&text) {
        Ok(manifest) => Some(manifest.id),
        Err(error) => {
            tracing::error!(dir = %dir.display(), %error, "invalid plugin manifest");
            None
        }
    }
}
