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

## Versioning

qcc has three versions, and each says something different.

| Version | Where | What it covers |
| --- | --- | --- |
| Product | `version` under `[workspace.package]` in `Cargo.toml` | the release: every crate and the desktop app (`tauri.conf.json` has no version of its own) take it from there |
| Protocol | `PROTOCOL_VERSION` in `crates/qcc-protocol/src/lib.rs`, an integer | the JSON-RPC between `qccd` and its clients (the CLI, the app); they refuse each other when it differs |
| Plugin API | `package qcc:plugin@X.Y.Z` in `wit/plugin.wit` and `API_VERSION` (`"X.Y"`) in `crates/qcc-plugin-host/src/manifest.rs` | the contract plugins are built against; the two change together, and so does `api` in the `plugin.toml` of the plugins in `plugins/` |

### The product version

The product follows [SemVer](https://semver.org/) and stays at `0.x` for now:

- `0.MINOR.0` for breaking or notable changes: a new protocol version, a breaking change of the
  plugin API, a change of the data format that existing data is not migrated to, incompatible
  changes of CLI commands or flags, large features.
- `0.x.PATCH` for fixes and small changes that leave the protocol alone.
- `1.0.0` once the plugin API and the CLI are stable and come with a promise of compatibility.

A release that raises the protocol or the plugin API is at least a minor release, and its changelog
says so.

The product version is changed only by the release commit on `main`. Task branches leave it alone,
so branches worked on side by side do not conflict over it.

### The protocol and the plugin API

`PROTOCOL_VERSION` goes up with every change of the RPC that a client or a daemon of another build
would trip over: a method added, removed or renamed, a parameter or a result changed incompatibly.
It is raised in the same branch that changes the protocol, not at release time.

The plugin API goes up when the interface of plugins changes: `wit/plugin.wit`, the imports the host
provides, the permissions. Before 1.0 a breaking change raises the minor version (`0.1.0` →
`0.2.0`); a host loads only plugins built for exactly its `X.Y`. Like the protocol, it is raised in
the branch that makes the change.

### Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/): a section per
version, headed `## [X.Y.Z] - YYYY-MM-DD`, with `Added`, `Changed`, `Fixed` and `Removed` groups.
It is written at release time from `git log <previous tag>..HEAD`, grouped by what users notice
rather than commit by commit. Task branches do not keep an `Unreleased` section.

dist takes the section whose heading matches the version of the tag and uses it as the body of the
GitHub Release (its title becomes the release's name), so the heading has to carry the exact
version, pre-release suffix included: `## [0.1.0-rc.1] - 2026-11-23`.

### Tags and release candidates

Tags are made on `main` only and look like `vX.Y.Z`. dist requires the version in `Cargo.toml` to
be the version of the tag, so a build is checked first with a release candidate:

1. Release `X.Y.Z-rc.N`: `Cargo.toml` says `X.Y.Z-rc.N`, the tag is `vX.Y.Z-rc.N`, and the GitHub
   Release is marked as a pre-release. Fix what it shows and make `rc.N+1` if needed.
2. Once a candidate is good, release `X.Y.Z` from a new release commit and tag `vX.Y.Z`. Its
   changelog section covers everything since the previous final release; the sections of its
   candidates fold into it.

The desktop bundles take the pre-release version too: the `.deb`, the AppImage and the `.dmg` of
`0.1.0-rc.1` are named `qcc_0.1.0-rc.1_*`. Two things to know about it:

- dpkg reads `0.1.0-rc.1` as the upstream version `0.1.0` with the Debian revision `rc.1`, which
  sorts after `0.1.0`. Installing the final `.deb` over a candidate is therefore a downgrade to apt:
  remove the candidate first (`sudo apt remove qcc`) or install with
  `sudo apt install --allow-downgrades ./qcc_<version>_amd64.deb`.
- On macOS the app's `CFBundleShortVersionString` gets `0.1.0-rc.1`, which Apple expects to be
  numbers only. Unsigned builds open regardless (after clearing the quarantine flag, see above); it
  would matter for notarization or the App Store, neither of which qcc uses.

Releases are made when there is something to release, not after every merge.

## Making a release

On a clean `main`, with `vPREV` the previous tag:

1. Write the section of the new version in `CHANGELOG.md` from `git log vPREV..HEAD`. Commit it.
2. Set `[workspace.package] version` in `Cargo.toml` and run `cargo check` so that `Cargo.lock`
   follows. Check the release notes with `dist plan` (see below). Commit both as
   `Release X.Y.Z`.
3. Tag that commit and push `main` and the tag:

   ```sh
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

The tag starts `.github/workflows/release.yml`:

- **dist** ([axodotdev/cargo-dist](https://github.com/axodotdev/cargo-dist), configured in
  `dist-workspace.toml`) builds `qcc` and `qccd` for the four targets on Ubuntu 22.04 (for an older
  glibc) and Apple silicon `macos-26` runners, and writes the archives, checksums and shell
  installers;
- `.github/workflows/desktop.yml`, which the release workflow calls, builds the desktop bundles
  with `tauri-apps/tauri-action`, putting the `qccd` and `qcc` built by dist inside them as
  sidecars (`apps/desktop/src-tauri/tauri.release.conf.json`);
- then one GitHub Release with all of it is created.

On pull requests the workflow only runs `dist plan`.

`release.yml` is generated: do not edit it by hand. Change `dist-workspace.toml` (or
`.github/build-setup.yml`, the extra steps of every build job) and run `dist generate`; `dist plan`
fails while the file is out of date. The plan of a tag, with the release notes it will publish:

```sh
nix shell nixpkgs#cargo-dist -c dist plan --tag vX.Y.Z --output-format=json | jq -r '.announcement_title, .announcement_github_body'
```

GitHub retires old runner images (macOS 14 went in November 2026). The macOS label is pinned in two
places that must stay the same: `[dist.github-custom-runners]` in `dist-workspace.toml` and the
matrix of `.github/workflows/desktop.yml`. To move to a newer image, change both after checking the
labels against [actions/runner-images](https://github.com/actions/runner-images). `release.yml`
holds no labels: its build matrix comes from `dist plan` at run time, which `dist plan` shows
locally.

A local dry run of one target:

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
