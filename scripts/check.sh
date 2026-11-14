#!/usr/bin/env bash
# The checks every stage must pass. Run inside `nix develop`.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
