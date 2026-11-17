#!/usr/bin/env bash
# Build the bench page for production and run it on every data set; prints one JSON line each.
set -euo pipefail
cd "$(dirname "$0")/.."
pnpm exec vite build --config bench/vite.config.ts --logLevel error >/dev/null
cp -r bench/data /tmp/qcc-bench/dist/bench/
pnpm exec vite preview --config bench/vite.config.ts --outDir /tmp/qcc-bench/dist --port 1489 --strictPort >/dev/null 2>&1 &
server=$!
trap 'kill $server' EXIT
sleep 2
for view in tab panel; do
  for data in ${DATASETS:-data}; do
    echo "$view $data $(timeout 300 node bench/run.mjs "http://localhost:1489/bench/changes.html?view=$view&data=$data")"
  done
done
