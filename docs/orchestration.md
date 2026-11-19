# Orchestration

An agent can hand work to other agents through the same CLI. Every session gets `QCC_THREAD_ID`,
`QCC_SOCKET` and `QCC_BIN` (the full path to `qcc`), so a thread started from inside an agent
automatically becomes its child.

```sh
qcc thread spawn --prompt '…' [--provider codex] [--env git-worktree]   # the default provider is the parent's
qcc thread spawn --prompt '…' --env git-worktree --env-dir tasks/T-1/app --branch feature/T-1   # a worktree where you say
qcc thread wait <id>… [--any] [--timeout 100]   # wait until the threads finish; prints their last answers
qcc thread list --parent <id>                  # children; without --parent, everything as a tree
qcc thread parent <id> <parent-id>             # move a thread under another one; none makes it standalone
```

A child inherits from its parent whatever is not set explicitly at start: the provider, the
project, the permission mode, the model and the effort (the model and effort only if the provider
is the same). If the child's provider does not support the parent's mode, the nearest stricter
mode is used. The agent's own settings (`qcc settings provider … --permission-mode`) apply only if
the parent has no mode.

A child never works with more freedom than its parent (the order from strict to free: `ask`,
`accept-edits`, `auto`, `full`) — neither at start nor later through `thread mode`. Otherwise an
agent could bypass its own mode through a child. A parent without a mode works by its agent's
configuration, which qcc cannot see, so its children can only have `ask` or the same "agent's
default" mode. A child's mode can be loosened only by loosening the parent's mode first. For the
same reason `thread parent` does not put a thread under a parent stricter than the thread, nor
under itself or its own descendant.

**A thread on the user's behalf.** An agent that helps manage qcc starts the user's threads, not
children: `thread spawn --root` from inside a session makes the thread standalone (no parent, no
ceiling from the parent's mode) and records the session that started it in `started_by`
(`qcc thread show` prints "started by thread …"). This is not a new loophole: a standalone thread
can be started anyway by unsetting `QCC_THREAD_ID`; `started_by` just says honestly where it came
from.

For an agent that reads state rather than timelines:

```sh
qcc thread list --state waiting --json   # who is waiting for the user (working, waiting, idle, failed, archived)
qcc thread show <id> --last 20           # the last lines of the timeline, not all of it
```

**A service thread.** `thread spawn --hidden` (or `hidden` in a plugin's `threads.spawn`) is a
thread for a plugin's own needs. The app does not show it in the sidebar or the overview, and
`thread list` shows it only with `--hidden`; it is addressed by id. A plugin that did not give a
`cwd` in `threads.spawn` gets its own directory for the thread, `<data>/plugin-data/<id>/threads`.

`thread wait` returns when nothing is running in the thread and nothing will run without a new
message (idle, failure, archive). A thread waiting for the user's answer does not count as
finished: on timeout it is shown as `waiting for the user`. Exit code: 0 — all turns completed,
1 — some failed or was interrupted, 124 — the timeout expired. Delegation depth is limited: no
more than four levels below the thread a human started.

## The orchestration plugin

The `orchestration` plugin is an ordinary plugin on the public API (permissions `threads`,
`skills`, `kv`):

- **Skill** `qcc-orchestration`: how to delegate through `qcc`. The daemon writes it to
  `<data>/skills/orchestration/qcc-orchestration/SKILL.md`, and each bridge tells its agent about
  it in the agent's usual way: Claude Code through `--append-system-prompt`, Codex through
  `developerInstructions`, ACP/opencode as the first block of the first request of a new session.
  The user's home configs are not changed. Tested live: Claude Code and Codex read the file on
  their own and start a child.
- **`qcc orchestration handoff --thread <id> [--to codex] [--note '…']`** — a new thread in the
  same directory continues the task with the original thread's conversation.
- **`qcc orchestration review --thread <id> [--by codex] [--focus '…']`** — a reviewer looks at
  what the thread did; when it finishes, its remarks go to the original thread as a message.

From inside an agent `--thread` can be omitted: by default it is the agent's own thread.
`qcc command list` shows the commands of all plugins.

How skills are offered, turned on and off is described in [Skills](agents.md#skills).
