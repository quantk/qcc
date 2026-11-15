#!/usr/bin/env bash
# The checks of the desktop app. Run inside `nix develop .#desktop`.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo clippy -p qcc-desktop --all-targets -- -D warnings
(
  cd apps/desktop
  pnpm install --frozen-lockfile
  pnpm typecheck
  pnpm test
  pnpm exec vite build
)
