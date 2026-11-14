//! The plugins shipped with the daemon.
//!
//! They are ordinary plugins: compiled from `plugins/` to components at build time
//! and installed into the plugin directory the same way a third-party plugin is.

use std::io;
use std::path::Path;

pub struct BundledPlugin {
    pub id: &'static str,
    /// Contents of the plugin's `plugin.toml`.
    pub manifest: &'static str,
    pub wasm: &'static [u8],
}

/// File name of the manifest in an installed plugin directory.
pub const MANIFEST_FILE: &str = "plugin.toml";
/// File name bundled manifests use for the component.
pub const WASM_FILE: &str = "plugin.wasm";

pub const PLUGINS: &[BundledPlugin] = include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub fn plugin(id: &str) -> Option<&'static BundledPlugin> {
    PLUGINS.iter().find(|plugin| plugin.id == id)
}

/// Install every bundled plugin as `<plugins_dir>/<id>/`, replacing an older copy.
/// Files that are already up to date are left untouched.
pub fn install(plugins_dir: &Path) -> io::Result<()> {
    for plugin in PLUGINS {
        let dir = plugins_dir.join(plugin.id);
        std::fs::create_dir_all(&dir)?;
        write_if_changed(&dir.join(WASM_FILE), plugin.wasm)?;
        write_if_changed(&dir.join(MANIFEST_FILE), plugin.manifest.as_bytes())?;
    }
    Ok(())
}

fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    if std::fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}
