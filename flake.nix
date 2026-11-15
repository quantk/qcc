{
  description = "qcc — control center for coding agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/02c087f3fe673c9e317f206a627cb6b910336dbe";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          # Toolchain version and targets live in rust-toolchain.toml so that
          # rustup users get the same setup.
          rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          core = [
            rust
            pkgs.pkg-config
            pkgs.sqlite
            pkgs.wasm-tools
            pkgs.git
            pkgs.gnumake
          ];

          # Everything the Tauri desktop app needs on top of the core shell.
          desktop = [
            pkgs.cargo-tauri
            pkgs.nodejs
            pkgs.pnpm
            pkgs.webkitgtk_4_1
            pkgs.gtk3
            pkgs.libsoup_3
            pkgs.glib
            pkgs.cairo
            pkgs.pango
            pkgs.gdk-pixbuf
            pkgs.atk
            pkgs.librsvg
            pkgs.openssl
          ];
        in
        {
          # Stages 0–5: daemon, CLI and plugins. No GUI dependencies.
          default = pkgs.mkShell { packages = core; };

          # Stage 6: `nix develop .#desktop`.
          desktop = pkgs.mkShell {
            packages = core ++ desktop;
            shellHook = ''
              export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS"
              # The DMA-BUF renderer leaves the window blank with some drivers (NVIDIA
              # in particular), so it is off unless asked for; it is also what makes
              # scrolling smooth. `WEBKIT_DISABLE_DMABUF_RENDERER=0 ./dev desktop`
              # tries it.
              export WEBKIT_DISABLE_DMABUF_RENDERER=''${WEBKIT_DISABLE_DMABUF_RENDERER-1}
            '';
          };
        }
      );
    };
}
