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
qcc thread archive <id> [--force] [--no-descendants]   # put the thread and everything under it away
qcc thread fork <id> [<turn>] --prompt '…'     # try another way from an earlier turn, leaving the thread as it is
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

## Archiving a tree of threads

`qcc thread archive <id>` archives the thread and every thread started under it, at any depth
(an archived thread in between is looked through: what is under it still goes). It works one thread
at a time, deepest first, and prints a line per archived thread, the one asked for last.

- **Working descendants are stopped.** Archiving releases a descendant's session, which ends its
  turn the way `thread stop --release` does; the report marks it `(stopped)`. The thread you archive
  must not be working itself: stop it first, as when archiving a single thread. A tree with a
  session that is still starting or stopping is refused whole, before anything is touched.
- **While it runs, the tree is held**: its threads take no messages, run nothing from their queues,
  get no new children and cannot be moved under another parent.
- **A refusal keeps what is above it.** A descendant that refuses (uncommitted work in its worktree
  without `--force`, an environment error) stays as it was, and so do its ancestors, the thread
  you archive included; the other branches are archived. No archived thread is left with live
  threads under it. The command prints why each thread stayed and exits with 1; run it again with
  `--force` to discard the uncommitted work, or commit it first.
- `--no-descendants` archives only the thread and leaves its children as they are.

Over the protocol this is `thread.archive` with `descendants: true`; without it the method archives
one thread, as before, and answers just `{ thread }`. With it the answer also lists `archived`,
`stopped`, `refused` (with the error code and message of each) and `kept`. A thread is never
spawned under an archived thread or one being archived.

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
  For reviews that start on their own and leave line comments, see [Reviewers](reviewers.md).

From inside an agent `--thread` can be omitted: by default it is the agent's own thread.
`qcc command list` shows the commands of all plugins.

How skills are offered, turned on and off is described in [Skills](agents.md#skills).
