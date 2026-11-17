#!/usr/bin/env bash
# Record the data the bench page reads: a git repository with a large change (hundreds
# of files, a lockfile of tens of thousands of lines, binaries, new files), and what a
# daemon answers about it. Needs `qcc` and `qccd` on PATH; starts its own daemon in a
# temporary directory and stops it after.
set -euo pipefail
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'kill "$daemon" 2>/dev/null || true; rm -rf "$work"' EXIT
unset QCC_THREAD_ID
export QCC_DATA_DIR=$work/qcc QCC_SOCKET=$work/qcc/qccd.sock
mkdir -p "$QCC_DATA_DIR" data
python3 make-repo.py "$work/repo"
qccd >"$work/qccd.log" 2>&1 &
daemon=$!
for _ in $(seq 50); do qcc daemon status >/dev/null 2>&1 && break; sleep 0.1; done
thread=$(qcc thread spawn --provider echo --cwd "$work/repo" --root --json | python3 -c 'import json, sys; print(json.load(sys.stdin)["thread"]["id"])')
qcc thread changes "$thread" --json >data/changes.json
qcc thread diff "$thread" --json >data/diff.json
# Every file's diff, cut nowhere: the page answers `paths` from it.
(
  cd "$work/repo"
  git diff --no-color --no-ext-diff --no-renames HEAD
  git ls-files -z --others --exclude-standard |
    xargs -0 -n1 sh -c 'git diff --no-color --no-index -- /dev/null "$0"; true'
) >data/full.diff
