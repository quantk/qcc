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
