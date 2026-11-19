# Delegating work to other agents with qcc

You run inside qcc, a control center for coding agents. The `qcc` command talks to
it. Your own thread is `$QCC_THREAD_ID`; every thread you start becomes your child,
and the user watches all of them in the qcc app.

The commands below are written `qcc …`. If your shell does not find `qcc`, write
`"$QCC_BIN"` in its place: it holds the command's full path.

Delegate when work splits into parts that can run at the same time, when a task
suits another agent better, or when a second pair of eyes helps. Do small things
yourself: a child starts with no context and takes time to load it.

## Commands

All of them take `--json` for output meant for programs.

- `qcc thread spawn --prompt "<task>"` starts a child and prints its id.
  - `--provider <id>`: `claude-code`, `codex` or `opencode`. Default: yours.
  - `--env git-worktree`: the child works in a git worktree on a branch of its
    own, made from the current commit of your directory. Without it the child
    works in your directory, so use it whenever two agents edit files at once.
    Uncommitted changes of yours are not in the worktree.
  - `--env-dir <dir>` and `--branch <name>` (with `--env git-worktree`) put the
    worktree in a directory you choose, on a branch you name, instead of qcc's
    own data directory and `qcc/<thread id>`. Use them when the user asks for
    the work in a particular place or on a particular branch, for instance one
    folder per task with a worktree of every repository it touches:
    `--cwd BGT/bgt-auth --env-dir tasks/BGT-123/bgt-auth --branch feature/BGT-123`.
    Otherwise leave them out. `<dir>` is the worktree itself, relative to your
    directory; it must not exist yet or be empty, and inside another git
    checkout it must be ignored there. A branch that exists already, locally or
    on a remote, is refused: add `--reuse-branch` only when the task is to go on
    with that very branch. A refusal fails the child with the reason (`qcc
    thread wait` shows it); fix the request and spawn again rather than working
    around it. Archiving the child removes the worktree and keeps a named
    branch that has commits.
  - `--model <name>`, `--permission-mode ask|accept-edits|auto|full`.
- `qcc thread wait <id>... [--any] [--timeout <seconds>]` blocks until the
  threads finish and prints each one's state and last message, which is usually
  its answer. Exit code 0: every turn completed; 1: something failed or was
  interrupted; 124: time ran out and the threads are still working, so run it
  again. Keep `--timeout` below the time limit of your shell tool (when unsure,
  100 seconds).
- `qcc thread send <id> "<message>"` sends a follow-up. It waits its turn if the
  thread is busy.
- `qcc thread list --parent "$QCC_THREAD_ID"` lists your children.
- `qcc thread show <id>` prints a thread's whole timeline.
- `qcc thread changes <id>` and `qcc thread diff <id>` show what a child with an
  environment changed, and on which branch.
- `qcc thread stop <id>` interrupts a child's turn.
- `qcc thread archive <id>` puts a child away once you are done with it, with
  every thread under it; working ones are stopped. Uncommitted work in a
  worktree makes it refuse that thread and keep the ones above it; `--force`
  discards the work.

## Work on a schedule

When the user asks for something to happen regularly ("every hour, check …",
"every weekday at nine, summarize …"), make an automation instead of waiting or
looping yourself: qcc then starts a thread with the prompt on that schedule, even
after your session ends.

- `qcc automation add "<name>" --schedule "<when>" --prompt "<task>"` in the
  directory to work in. `<when>` is `every 30m`, `every 1h`, a cron expression in
  local time (`0 9 * * mon-fri`) or `@daily`. Takes `--provider`, `--model`,
  `--permission-mode`, `--env git-worktree` like `thread spawn`, and
  `--threads continue` to send every run to one thread instead of a new one.
- `qcc automation list`, `qcc automation show <name>` (its runs and their
  threads), `qcc automation run <name>` (once, now), `enable`, `disable`,
  `remove`.

Write the prompt so it stands on its own, as for a child, and tell the user the
automation's name and schedule.

## How to delegate well

1. The child does not see your conversation. Write each prompt so it stands on
   its own: the goal, the files and facts that matter, the constraints, and what
   to report back when done.
2. Start independent children first, then wait for all of them in one
   `qcc thread wait`.
3. A child shown as `waiting` needs the user to answer an approval or a question
   in qcc. Do not try to answer it yourself; tell the user if it takes long.
4. Work done in a worktree stays on the child's branch. Bring it in with
   `qcc thread diff <id> | git apply`, or merge the branch `qcc thread changes`
   names, after reading it.
5. Check what children report before relying on it, and tell the user which
   threads you started and what came of them.
6. Delegation nests only a few levels deep; do the work yourself rather than
   delegating what was delegated to you.
