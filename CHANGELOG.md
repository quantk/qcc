# Changelog

All notable changes to qcc are written down here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and qcc follows
[Semantic Versioning](https://semver.org/); see [Versioning](docs/releasing.md#versioning).

## [0.1.0-rc.2] - 2026-11-23

A second candidate: 0.1.0-rc.1 was never published, because its macOS builds were cancelled.
Protocol version 8, plugin API 0.1.

### Fixed

- The macOS builds of the CLI, the daemon and the desktop app run on `macos-26` runners: GitHub
  retired `macos-14`, which cancelled every macOS job of 0.1.0-rc.1.

## [0.1.0-rc.1] - 2026-11-23

The first release: a candidate to check the release builds. Protocol version 8, plugin API 0.1.

### Added

- `qccd`, a daemon that runs agent sessions as threads, keeps their timelines in a SQLite journal
  and keeps working when no client is connected; `qcc`, its CLI, with `--json` on every command.
- Providers for Claude Code (`claude -p`), Codex (`codex app-server`) and opencode (`opencode acp`),
  turned into one timeline: messages, reasoning, plans, commands with output, edits with diffs, tool
  and MCP calls, permission prompts and questions.
- Per-thread model, effort and permission mode (including read-only), switching them or the agent
  itself between turns; queueing a message or steering it into the running turn; compaction, a new
  conversation in the same thread, and forking a thread from any turn.
- Git worktree environments: a thread on its own branch, its changes by part (uncommitted work,
  commits), comments on lines of the diff for the agent, delivering the work back to the checkout,
  and archiving.
- Orchestration: agents start, wait for and read other agents through the CLI; children inherit the
  parent's project, model and permission mode and never run looser than the parent.
- Projects that group threads, with icons; automations that send an agent a prompt on a schedule.
- MCP servers, skills and per-agent settings kept in the daemon and added to every agent's own,
  switchable per thread.
- Context window usage and its breakdown, token and tool metrics, and the usage limits of the Claude
  and Codex subscriptions.
- A desktop app (Tauri) over the same daemon: an overview of who needs you, a Live wall of running
  agents, the timeline with answers to prompts, Changes with review comments, Metrics, settings,
  attachments and pasted images, and desktop notifications.
- WebAssembly plugins (`wasm32-wasip2`) with explicit permissions, a declarative UI and sandboxed web
  views; the built-in providers, worktrees, orchestration, reviewers and notifications are plugins
  themselves.
- Reviewers: a plugin whose agents review a thread's changes on their own and leave findings on it.
- Release builds for Linux (x86_64, aarch64) and macOS: shell installers for `qcc` and `qccd`, and
  desktop bundles (`.deb`, AppImage, `.dmg`) with both inside.
