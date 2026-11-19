# Development

Instructions for coding agents working on the repository are in [AGENTS.md](../AGENTS.md).

## Dev commands

One command, from any shell (it enters `nix develop` by itself):

```sh
./dev demo      # start the daemon, run a few echo threads, stop it
./dev           # qccd in the foreground, data in ./.dev
./dev qcc thread spawn --provider echo --prompt 'hello' --follow   # the CLI against this daemon
./dev qcc thread spawn --provider claude-code --permission-mode ask --prompt 'what is in this directory?' --follow
./dev qcc thread send <thread-id> 'and one more thing' --steer
./dev desktop   # the desktop app (nix develop .#desktop)
./dev check     # fmt, clippy -D warnings, all tests
./dev clean     # delete ./.dev
```

Every `./dev` command that builds the project also restarts the dev daemon if it was left over
from a previous build: otherwise the app and the CLI would be talking to old code. If agents are
working in it, the script lists them and asks (without a terminal it only warns and leaves the
daemon alone).

Inside `nix develop` the same is available as `make run|demo|check|test|clean`.

Every `qcc` command has `--json`. Outside `./dev` the data lives in `~/.local/share/qcc`
(`QCC_DATA_DIR`), and the socket is `$XDG_RUNTIME_DIR/qcc/qccd.sock` (`QCC_SOCKET`).

How `./dev desktop` differs from `./dev app` is described in
[Desktop app](desktop.md#running-it).

## Architecture

```
wit/plugin.wit            plugin contract (qcc:plugin@0.1.0)
crates/qcc-domain         types without IO: the delta grammar and the canonical timeline
crates/qcc-runtime        delta assembler, thread state machine, recovery after restart
crates/qcc-store          SQLite: event journal and projections
crates/qcc-plugin-host    wasmtime, permissions, plugin actor
crates/qcc-plugin-sdk     guest side of the contract
crates/qcc-plugin-testkit conformance suite for bridges
crates/qcc-plugins-bundle builds plugins/ into wasm components when the daemon is built
crates/qcc-protocol       daemon API (JSON-RPC over a unix socket)
crates/qcc-client         Rust client of the API
crates/qcc-daemon         qccd
crates/qcc-cli            qcc
apps/desktop              control panel: src-tauri (proxy to qccd) + React
plugins/provider-echo     example provider
plugins/provider-claude-code, provider-acp, provider-codex   bridges to real agents
plugins/env-git-worktree  environment: a git worktree per thread
plugins/orchestration     skill for agents, handoff and review commands
plugins/notify            desktop notifications when an agent finishes, fails or asks
```

The main rule: **the bridge knows the dialect, the core knows the timeline.** A provider plugin
translates the agent's output into a narrow set of deltas; the ids, order and lifecycle of turns
and items belong to the assembler in `qcc-runtime`. The core never branches on a provider id, and
built-in plugins take the same path as third-party ones.

No host import blocks, so work that waits for a process answers later: `handle` returns
`deferred(token)`, and the result arrives through `completions.complete`. This is how
`env-git-worktree` is built: each operation is a state machine over finished git commands
(`src/jobs.rs`, tested without wasm).

## Bridges and recorded traffic

Agent protocol formats are not written from memory: `plugins/provider-*/fixtures/*.ndjson` holds
traffic recorded from the installed CLIs, and next to it is `*.timeline.jsonl`, the timeline it must
assemble into. The tests `plugins/provider-*/tests/conformance.rs` replay the recording in place of
the agent process, through the real wasm host and assembler.

When an agent has been updated and its behavior changed:

```sh
# record the scenario again (the scripts run the real CLI)
python3 scripts/fixtures/record-claude.py RAW.ndjson allow '[{"send": …}, {"wait": "result"}]'
python3 scripts/fixtures/record-jsonrpc.py RAW.ndjson '[…]' -- opencode acp
# remove paths, credentials and anything extra; the script checks that nothing personal is left
python3 scripts/fixtures/redact.py claude RAW.ndjson plugins/provider-claude-code/fixtures/NAME.ndjson --work "$PWD"
# rewrite the golden timelines and look at the diff
QCC_UPDATE_GOLDEN=1 cargo test -p provider-claude-code --test conformance
```
