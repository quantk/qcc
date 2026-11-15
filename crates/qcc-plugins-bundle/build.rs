//! Compiles every plugin under `plugins/` to a `wasm32-wasip2` component and embeds it.
//!
//! Built-in plugins get no shortcut into the daemon: they are built from the same
//! sources, with the same SDK, into the same artifact a third-party plugin ships.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "wasm32-wasip2";

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().and_then(Path::parent).unwrap().to_path_buf();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let plugins_dir = root.join("plugins");

    for input in ["plugins", "wit", "crates/qcc-plugin-sdk", "Cargo.lock"] {
        println!("cargo::rerun-if-changed={}", root.join(input).display());
    }

    let mut plugins: Vec<String> = fs::read_dir(&plugins_dir)
        .expect("plugins directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.join("plugin.toml")
                .is_file()
                .then(|| path.file_name().unwrap().to_str().unwrap().to_owned())
        })
        .collect();
    plugins.sort();

    // A separate target directory: the outer cargo holds the lock on its own.
    let target_dir = root.join("target").join("plugins");
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(&root)
        .args(["build", "--release", "--locked", "--target", TARGET, "--target-dir"])
        .arg(&target_dir);
    for plugin in &plugins {
        cargo.args(["-p", plugin]);
    }
    // The nested build is a different target with its own flags; do not let the
    // outer invocation (clippy, coverage, custom rustflags) leak into it.
    for name in [
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTC_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_RUSTFLAGS",
    ] {
        cargo.env_remove(name);
    }
    let status = cargo.status().expect("cannot run cargo");
    assert!(
        status.success(),
        "building the bundled plugins for {TARGET} failed; \
         is the target installed? (`nix develop` provides it)"
    );

    let mut generated = String::from("&[\n");
    for plugin in &plugins {
        let wasm = target_dir
            .join(TARGET)
            .join("release")
            .join(format!("{}.wasm", plugin.replace('-', "_")));
        let embedded = out_dir.join(format!("{plugin}.wasm"));
        fs::copy(&wasm, &embedded).unwrap_or_else(|error| panic!("{}: {error}", wasm.display()));
        let manifest = plugins_dir.join(plugin).join("plugin.toml");
        writeln!(
            generated,
            "    BundledPlugin {{ id: {plugin:?}, manifest: include_str!({manifest:?}), \
             wasm: include_bytes!({embedded:?}) }},"
        )
        .unwrap();
    }
    generated.push_str("]\n");
    fs::write(out_dir.join("bundle.rs"), generated).unwrap();
}
