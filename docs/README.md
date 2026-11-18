# qcc documentation

- [agents.md](agents.md) — supported agents, models, effort, switching agents, settings, MCP
  servers, permission modes, queue and steer, reset and compaction, context breakdown, message
  notes, attachments, skills.
- [projects-and-worktrees.md](projects-and-worktrees.md) — projects, isolating threads in git
  worktrees, delivering their work back, comments on changes.
- [orchestration.md](orchestration.md) — agents starting agents through the CLI, mode
  inheritance, `thread wait`, and the `orchestration` plugin (`handoff`, `review`).
- [automations.md](automations.md) — prompts the daemon sends an agent on a schedule (cron or
  intervals): new thread or one thread per automation, overlapping and missed runs, history.
- [desktop.md](desktop.md) — running the desktop app (`./dev desktop`, `./dev app`) and what each
  screen, panel and shortcut does.
- [development.md](development.md) — dev commands, repository architecture, and recorded agent
  traffic for bridge tests.
- [plugins.md](plugins.md) — writing a plugin: host limits, plugin UI, commands, skills, threads,
  settings and permissions.
