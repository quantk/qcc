# Projects and worktrees

## Projects

A project is a name and several repositories or folders. Threads are grouped by project: in the
app's sidebar, in the overview, and in `qcc thread list --json` (the `project` field).

```sh
qcc project create Shop --folder ~/src/shop-api --folder ~/src/shop-web
qcc project add-folder Shop ~/src/shop-infra      # remove-folder, rename, delete
qcc project list
qcc thread spawn --project Shop --cwd ~/src/shop-web --provider codex --prompt '…'
qcc thread project <thread-id> Shop               # or none to remove it from the project
```

- A thread joins a project by itself: the project is given explicitly, or the parent has it
  (agents started by agents stay in their parent's project), or the thread's directory lies in one
  of the project's folders (the most deeply nested folder wins).
- A new project or a new folder takes over threads without a project that are already working in
  its folders.
- Deleting a project keeps its threads (outside any project) and does not touch files.
- A thread works in one folder of the project (in the app it is chosen when the thread is
  created), and the agent sees the other folders by default, as in Codex desktop: Claude Code gets
  them through `--add-dir`, Codex as additional sandbox directories (`writable_roots`) and a
  mention in the instructions, opencode as a list at the start of the session. The rest of the disk
  is not forbidden: the [permission mode](agents.md#permission-mode) decides what is allowed
  without asking. Folders are read when a session starts, so project changes reach the agent with
  the next session.

The app groups threads outside projects by directory. Such a group can be turned into a project
with one button.

## Worktree

`qcc thread spawn --env git-worktree` gives a thread its own working copy: a `git worktree` on a
new branch `qcc/<thread-id>` from the current `HEAD` (or from `--base <ref>`). Threads work in
parallel and touch neither the main checkout nor each other. The agent does not start until the
directory is ready. If preparation fails, the thread fails instead of going off to work in the
original directory.

```sh
qcc thread spawn --provider claude-code --env git-worktree --prompt '…' --follow
qcc thread changes <thread-id>     # which files changed relative to the base
qcc thread diff <thread-id>        # everything as one patch: commits, uncommitted work and new files
qcc thread diff <thread-id> src/a.rs src/b.rs   # only these files (paths as in thread changes)
qcc thread archive <thread-id>     # release the session, delete the worktree, remove the thread from the list
```

- Directory: `<data>/environments/git-worktree/<thread-id>/<repository-name>`. A thread started
  from a subdirectory works in the same subdirectory of its copy.
- `thread changes` and `thread diff` also work for a thread without a worktree that was started
  directly in its own git checkout. Then they show what is uncommitted there relative to `HEAD`
  (edits, deletions, new files), with paths from the repository root. This is done by an
  environment plugin that declared `inspects-directories`. For a directory outside git the command
  answers with an error.
- The whole patch is limited to 2 MB, and a large lockfile can push the other files out of it, so
  `thread diff` accepts paths: the app reads the diff a few files at a time, the ones that are
  visible.
- `.worktreeinclude` in the repository root (gitignore syntax) lists untracked files of the main
  checkout that are copied into a new worktree: `.env`, certificates.
- **Bringing the work back into your checkout**: `qcc thread deliver <id>` applies everything the
  thread did (the branch's commits, uncommitted work, new files) to the original checkout as
  uncommitted changes — all of it, or, if it does not apply cleanly, nothing. `--merge` commits the
  uncommitted work to the `qcc/<id>` branch and merges it into the branch checked out in the
  original checkout; on a conflict the merge is aborted. In the app: the **Apply to …** and
  **Merge** buttons on the Changes tab.
- **Moving into a worktree mid-thread**: `qcc thread isolate <id> [--carry]` (in the app: **Move
  into a worktree…** in the thread menu). The worktree is created from the current `HEAD`. With
  `--carry` the directory's uncommitted changes (all of them, yours too, new files included) move
  into it through `git stash` and leave the directory. The agent session restarts in the new
  directory and receives the conversation. If the worktree could not be made (the directory is not
  a repository), the thread stays where it was and the changes are put back.
- `.qcc-env-setup.sh` runs in a new worktree before the agent starts (`QCC_SOURCE_DIR`,
  `QCC_THREAD_ID`; 15 minute limit). A non-zero exit code cancels the creation: the worktree and the
  branch are deleted.
- `.qcc-env-teardown.sh` runs before deletion; its failure does not prevent the deletion.
- `thread archive` refuses if the worktree has uncommitted changes or they could not be checked
  (`git status` failed); `--force` deletes it anyway. The branch is deleted only if it has no
  commits that exist nowhere else. The timeline of an archived thread stays readable
  (`qcc thread list --archived`).

All of this is done by an ordinary plugin, `plugins/env-git-worktree`, with permissions for `git`,
`bash` and `rmdir`. The core knows nothing about git.

## Comments on changes

You can leave comments on lines of what a thread changed, as in a code review: in the app on the
Changes tab, or from the CLI. A comment remembers the file, the line number (in the new code or,
for a deleted line, in the old code) and the line's text, so it can be found after the code has
moved. Comments are stored in the daemon's journal, separately for each thread, and are deleted
together with the thread.

```sh
qcc comment add --thread <id> src/lib.rs 42 "this needs a timeout"   # --old: a line of the old code
qcc comment list [--thread <id>] [--all]       # open ones; --all also resolved ones
qcc comment resolve <comment-id>... [--note "what was done"]
qcc comment reopen <comment-id>...             # delete <comment-id> removes one
qcc thread send <id> "fix according to the comments" --comment <comment-id> --comment <comment-id>
```

- The agent sees comments in two ways. `thread send --comment` (in the app: the **Send to agent**
  button) passes them with the message as a note: each comment in full, with its location, a quote
  of the line, and how to mark it resolved. And inside its own session the agent reads them
  itself: `qcc comment list` without `--thread` takes its thread from `$QCC_THREAD_ID`.
- When resolving a comment, the agent writes `qcc comment resolve <id> --note "…"`. The app
  immediately shows it resolved, together with the note.
