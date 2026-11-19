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
qcc thread diff <thread-id> --uncommitted        # only what is not committed yet
qcc thread diff <thread-id> --from <commit> --to <commit>   # a run of the thread's commits
qcc thread diff <thread-id> --full src/a.rs     # the whole file as context, not 3 lines around each change
qcc thread archive <thread-id>     # release the session, delete the worktree, remove the thread from the list
                                   # (and the threads started under it; --no-descendants: only this one)
```

- Directory: `<data>/environments/git-worktree/<thread-id>/<repository-name>`. A thread started
  from a subdirectory works in the same subdirectory of its copy. `--env-dir` and `--branch` put
  the worktree where you say, on a branch you name: see
  [below](#a-worktree-where-you-say-on-a-branch-you-name).
- `thread changes` also lists the commits the thread made since the base (along the branch's
  first parents, newest first). With them both commands can show a part of the work:
  `--uncommitted` is what is not committed yet (against `HEAD`, new files included);
  `--from <commit>` is what came after that commit, and `--to <commit>` what came up to and
  including it. `--from` alone runs up to the working tree, uncommitted work too; `--to` alone
  starts at the base. A commit is named by its hash as `thread changes` lists it. Without these
  flags it is all the work since the base.
- `thread changes` and `thread diff` also work for a thread without a worktree that was started
  directly in its own git checkout. Then they show what is uncommitted there relative to `HEAD`
  (edits, deletions, new files), with paths from the repository root. This is done by an
  environment plugin that declared `inspects-directories`. For a directory outside git the command
  answers with an error.
- The whole patch is limited to 2 MB, and a large lockfile can push the other files out of it, so
  `thread diff` accepts paths: the app reads the diff a few files at a time, the ones that are
  visible. `--full` makes each file one hunk from its first line to its last; the 2 MB limit
  still holds, so it is meant for a file or two at a time.
- `.worktreeinclude` in the repository root (gitignore syntax) lists untracked files of the main
  checkout that are copied into a new worktree: `.env`, certificates.
- **Bringing the work back into your checkout**: `qcc thread deliver <id>` applies everything the
  thread did (the branch's commits, uncommitted work, new files) to the original checkout as
  uncommitted changes — all of it, or, if it does not apply cleanly, nothing. `--merge` commits the
  uncommitted work to the `qcc/<id>` branch and merges it into the branch checked out in the
  original checkout; on a conflict the merge is aborted. In the app: the **Apply to …** and
  **Merge** buttons on the Changes tab.
- **Moving into a worktree mid-thread**: `qcc thread isolate <id> [--carry]` (in the app: **Move
  into a worktree…** in the thread menu). The worktree is created from the current `HEAD`; from
  the CLI it takes `--env-dir`, `--branch` and `--reuse-branch` like `thread spawn`. With
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
  (`qcc thread list --archived`). Archiving a thread archives its descendants too; see
  [Archiving a tree of threads](orchestration.md#archiving-a-tree-of-threads).

### A worktree where you say, on a branch you name

By default qcc chooses both the directory and the branch. Three flags of `thread spawn --env
git-worktree` (and of `thread isolate`) let you choose them, for instance to gather the worktrees of
one task that spans several repositories in one folder, on the branch name your team uses:

```sh
cd ~/Developer
qcc thread spawn --env git-worktree --cwd BGT/bgt-auth \
    --env-dir tasks/BGT-123/bgt-auth --branch feature/BGT-123 --prompt '…'
qcc thread spawn --env git-worktree --cwd BGT/bgt-frontend \
    --env-dir tasks/BGT-123/bgt-frontend --branch feature/BGT-123 --prompt '…'
```

Everything else works as for any worktree: `thread changes`, `thread diff`, `thread deliver`,
`thread archive`, `.worktreeinclude` and the setup and teardown scripts. Without these flags nothing
changes.

- **`--env-dir <dir>`** is the worktree itself, not a folder to put it in. A relative path is
  counted from the directory the command runs in, like `--cwd`, and `..` is resolved by name. The
  thread then works under that path, so an agent picks up the `AGENTS.md` files above it.
  - The directory must not exist yet, or be an empty directory. A directory with anything in it, a
    file, a symbolic link and an existing git checkout are refused; nothing is ever overwritten or
    adopted.
  - Inside the working tree of another git repository (a root checkout that holds your clones) the
    directory must be ignored there (`.gitignore` or `.git/info/exclude`). Otherwise it is refused:
    the worktree would show up in that repository as untracked files and could be added to it by
    accident.
  - Two threads cannot have one directory: the second request is refused while the first thread is
    not archived.
- **`--branch <name>`** names the branch instead of `qcc/<thread-id>`. Names under `qcc/` are
  reserved for the branches qcc names itself.
  - A branch of that name that exists already, in the repository or on one of its remotes as last
    fetched (qcc does not fetch), is refused: a branch somebody else works on is never taken over
    silently, and a new local branch would part ways with the remote one.
  - **`--reuse-branch`** says you mean that branch, to go on with a task. The worktree checks out
    the local branch, or else makes one that follows the remote branch (a branch found on several
    remotes is refused; make the local branch yourself). `thread changes`, `thread diff` and
    `thread deliver` then count from where the branch stood when the worktree was made, that is,
    the work of this thread; with `--base <ref>`, from where the branch left that ref, that is, the
    whole task.
  - One branch cannot be checked out in two worktrees of a repository, so a second thread asking
    for it is refused. The same name in different repositories is fine.
- **`thread archive`** removes the worktree, and refuses while it has uncommitted changes, as always.
  What it does with the rest is more careful than for a worktree of qcc's own:
  - A branch you named is kept as soon as it has a commit, pushed or not, and a branch that was
    reused is always kept. Only a branch that was made for the worktree and still stands where it
    started is deleted: nothing was done on it. (A `qcc/<thread-id>` branch is deleted as before,
    if it has no commits that exist nowhere else.)
  - The folder that holds the worktree (`tasks/BGT-123`) is removed once it is empty, and only if
    qcc made it for a worktree. A folder that was there before stays, even empty, and so does
    everything above it.
- If preparation fails after the worktree was made (the setup script), the worktree goes again,
  and so do the branch and the folder if they were made for it. A reused branch stays.

All of this is done by an ordinary plugin, `plugins/env-git-worktree`, with permissions for `git`,
`bash` and `rmdir`. The core knows nothing about git: an environment declares `custom-placement`,
and only then is it told a directory and a branch.

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
