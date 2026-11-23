# Releases

Releases are built by GitHub Actions for Linux (x86_64, aarch64) and macOS (Apple silicon, Intel).
Windows is not supported: `qccd` listens on a unix socket.

## Installing a release

The CLI and the daemon come with a shell installer each. Install both: they go to the same
directory (`$XDG_BIN_HOME`, or `~/.local/bin`), and `qccd` hands agents the `qcc` next to it.

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/quantk/qcc/releases/latest/download/qcc-cli-installer.sh | sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/quantk/qcc/releases/latest/download/qcc-daemon-installer.sh | sh
```

The same binaries are in the `qcc-cli-<target>.tar.xz` and `qcc-daemon-<target>.tar.xz` archives
of the release, with a `.sha256` next to each.

The desktop app carries its own `qccd` and `qcc`, so it works without the installers above:

- **Linux:** `qcc_<version>_amd64.deb` (`sudo apt install ./qcc_<version>_amd64.deb`; it also puts
  `qcc` and `qccd` in `/usr/bin`) or `qcc_<version>_amd64.AppImage` (`chmod +x` and run it). The
  bundles need WebKitGTK 4.1 (Ubuntu 22.04 or newer, or an equivalent).
- **macOS:** `qcc_<version>_aarch64.dmg` for Apple silicon, `qcc_<version>_x64.dmg` for Intel.
  The app is not signed or notarized, so Gatekeeper refuses to open it ("is damaged" or "cannot be
  opened"). After copying it to Applications, clear the quarantine flag:

  ```sh
  xattr -d com.apple.quarantine /Applications/qcc.app
  ```

The app starts the `qccd` bundled with it when no daemon is running; the daemon keeps running after
the window is closed. If you have installed the CLI too, keep both from the same release: a daemon
and an app of different protocol versions refuse to talk to each other.

## Making a release

The version lives in one place, `version` under `[workspace.package]` in `Cargo.toml`; every crate
and the desktop app (`tauri.conf.json` has no version of its own) take it from there.

1. Bump `[workspace.package] version` in `Cargo.toml` and run `cargo check` so that `Cargo.lock`
   follows. Commit it.
2. Tag the commit with the same version and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

A tag with a suffix (`v0.2.0-rc.1`) becomes a pre-release.

The tag starts `.github/workflows/release.yml`:

- **dist** ([axodotdev/cargo-dist](https://github.com/axodotdev/cargo-dist), configured in
  `dist-workspace.toml`) builds `qcc` and `qccd` for the four targets on Ubuntu 22.04 (for an older
  glibc) and macOS runners, and writes the archives, checksums and shell installers;
- `.github/workflows/desktop.yml`, which the release workflow calls, builds the desktop bundles
  with `tauri-apps/tauri-action`, putting the `qccd` and `qcc` built by dist inside them as
  sidecars (`apps/desktop/src-tauri/tauri.release.conf.json`);
- then one GitHub Release with all of it is created.

On pull requests the workflow only runs `dist plan`.

`release.yml` is generated: do not edit it by hand. Change `dist-workspace.toml` (or
`.github/build-setup.yml`, the extra steps of every build job) and run `dist generate`; `dist plan`
fails while the file is out of date. A local dry run of one target:

```sh
nix shell nixpkgs#cargo-dist -c nix develop -c dist build --artifacts=local --target=x86_64-unknown-linux-gnu
```

The desktop bundle can be built locally the same way the release does it, after putting the
binaries in `apps/desktop/src-tauri/binaries/qccd-<target>` and `qcc-<target>` (that directory is
ignored by git):

```sh
cd apps/desktop
nix develop ../..#desktop -c cargo tauri build --target x86_64-unknown-linux-gnu --config src-tauri/tauri.release.conf.json
```
