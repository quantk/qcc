# Desktop app

## Running it

```sh
./dev desktop   # build and open the app; the daemon starts by itself
./dev app       # the same for daily use: a release build, updated when the command is run again
```

`./dev desktop` is for development: Vite with hot reload, a debug build, data in `./.dev`.
`./dev app` is for using the app: it builds a release with the frontend embedded (no dev server,
editing the sources reloads nothing) and installs `qcc-desktop`, `qccd` and `qcc` into
`~/.local/lib/qcc` (another location: `QCC_APP_DIR`). The data lives where an installed qcc keeps
it: `~/.local/share/qcc`, and the socket is `$XDG_RUNTIME_DIR/qcc/qccd.sock`, separate from
`.dev`. So builds in the repository (and the agents that run them) do not restart the working
daemon. Running `./dev app` again rebuilds, replaces only the files that changed, restarts the
daemon of the old build (if agents are working in it, it asks first) and reopens the window. If
everything is up to date and the window is open, it touches nothing. To open the app without
building, run `~/.local/lib/qcc/qcc-app` (a wrapper with the GTK environment from the desktop
shell, suitable for a launcher). The CLI of the working daemon is `~/.local/lib/qcc/qcc`.

Released builds (`.deb`, AppImage, `.dmg` with `qccd` and `qcc` inside) are described in
[Releases](releasing.md).

By default WebKitGTK starts without the DMA-BUF renderer: with some drivers (NVIDIA) the window
stays blank with it. If scrolling is slow, try turning it on:
`WEBKIT_DISABLE_DMABUF_RENDERER=0 ./dev desktop`.

## What it shows

The app is a control panel: you see all agents at once and can answer each of them.

### Screens

Screens open with icon buttons in the sidebar (the name and the shortcut are in the tooltip).
Above the threads are the places where you watch agents (overview, Live, Metrics, plugin screens);
at the bottom is configuration (Automations, MCP servers, Skills, Settings) and the archived threads switch.
`Alt+1` … `Alt+9` open screens in the order of the buttons, `Ctrl+,` opens the settings. The Live
button shows how many agents are working or waiting for you; it is blue if someone is waiting.

### Overview

Three columns: who is waiting for you (a question, a permission request, a failure, or a new answer
you have not opened yet), who is working (what it is doing and how long the turn has been running),
and who has finished (the last reply). A permission request can be answered right from the card. A
thread with an unread answer is highlighted in the sidebar too; it becomes read when you open it.
Under its title a sidebar row shows the agent as a short mark, its logo or else the initials of
its name (the full name on hover), and the project the thread works on: the last folder of its
directory, or of the checkout a worktree is made from, with a branch icon for a worktree; hover it
for the full path. While the thread waits for you, has a new reply, failed or is working, the
status follows the project after a dot (`· Needs your answer`, `· Run cargo test`); hover it for
the whole text. Where the row is narrow, what the agent is doing is cut first and the project keeps
its part of the row, while a question or a failure, shown in color, takes most of the row before it
is cut.
Threads started by other agents never count as unread: their answers are collected by the parent
agent.

### Live

`Ctrl+Shift+L`. A wall of tiles for all agents that are working, waiting for you, or finished a
turn within the last 10 minutes (up to 24 tiles). Order: first those waiting for you, then the
working ones (the one working longest first), then the recently finished. A tile shows the turn
time, project, agent, model, tokens spent, context fill, and the agent's latest steps as they
appear: commands with the tail of their output, edits, reads, tools, the answer being written. The
steps sit in a box of limited height that always shows the latest one; a click on the tile opens
the whole timeline. A question or permission request can be answered right from the tile.

The switch in the header sets how big the tiles are, and the choice is remembered. Compact (the
default) fits a dozen agents or more on a screen: as many columns as the window width allows, the
last 5 steps with a line of output each. Detailed shows wider tiles with the last 10 steps and
three lines of output.

### Metrics

Spending and turns of all agents over time (the daemon method `usage.history`, by hour). For the
selected range (7/30/90 days or all time, days in local time) it shows tokens (input + output),
turns, the share of failed turns and working time. Below: a chart of tokens by day, split by
project, agent or model; a table by group (input, cached, output, turns, failure share, average
turn, working time); and the threads that spent the most. Cached is shown separately: for some
agents it is already included in input. Cost in money is not computed yet. Below them is "What
agents used": which tools, MCP tools (with totals per server), skills and programs (by the first
word of the command) the agents called in the range — how many times, in how many threads, the
failure share, the user's refusals, average and longest time (the `usage.tools` method, by hour
from the journal). Everything is for all threads or for one group (a project, agent, model). MCP
calls and skill use are marked by the provider's bridge (the `mcp_server` and `skill` fields of a
tool item): Claude Code by the `mcp__<server>__<tool>` name and the `Skill` tool, Codex by
`mcpToolCall`; opencode does not distinguish them yet.

### Usage limits

A section at the bottom of the sidebar, "Usage limits", keeps the subscription limits of the agents
in view: a line per agent that reports them (Claude Code, Codex; opencode does not say) with its
main windows as small bars, `Claude 5h 42% · week 18%`. A window turns amber past 80% and red at
95% or once the agent says the limit is reached. A click opens the details: the plan, every window
(per-model weekly ones too) with when it resets ("resets in 2h 10m"), how old the numbers are
("as of 25 min ago", with a clock mark once they are older than 15 minutes) and a Refresh button.
A window whose reset time has passed shows 0%: what was spent in it no longer counts.

The numbers are the daemon's, kept per agent across restarts (see
[Agents: subscription usage limits](agents.md#subscription-usage-limits)). Running sessions keep
them current by themselves. The app asks the agents again (each starts for a moment, without a
prompt) when it connects or the details are opened and the numbers are older than 5 minutes, and
on Refresh or the palette command "Refresh usage limits". When an agent cannot be asked, its last
numbers stay, marked old, with the reason in the details. "Usage limits in full" is a block with
all of it, offered but not placed: open it from the palette ("Usage limits") or put it where you
like (Block layout below), for example among the screens.

### Thread

A timeline with streaming output, commands, diffs of edits and permission requests; a composer
with the queue, steer and attachments (files and images: the paperclip, `Ctrl+V`, drag and drop);
switching agent, model, effort and permission mode; archiving with a confirmation if the worktree
has uncommitted work. Archiving a thread with threads under it asks first: the dialog lists every
descendant that will be archived with it, marks the working ones (they are stopped) and the ones
whose worktree has uncommitted work (it is discarded). An archive that takes longer than 300 ms
shows its progress in the dialog: a bar, every thread of the tree with its step (stopping the
agent, removing its worktree, archived, not archived, kept) and how many are done; hiding the
dialog lets the archive go on. A quick archive shows nothing extra. If a thread still refuses, the
app says which ones stayed and offers to archive them anyway.

Typing `/` at the start of a message or after a space opens a list of skills over the composer:
the qcc skills this thread's sessions get, marked `qcc`, and the ones its agent finds by itself,
asked in the thread's working directory so the project's own (`.claude/skills`, `.agents/skills`)
are there too, marked by where the agent has them from. Typing `@` the same way lists the files
and folders of the thread's working directory (its worktree, for a thread in one): what git
ignores, `.git`, `node_modules`, `target` and the like are left out. Both lists narrow as you type,
fuzzily (`cmpsr` finds `src/Composer.tsx`; a letter in a file's name, at the start of a word or
right after the previous one counts more), at most 50 entries. ↑ and ↓ move, Enter or a click
puts the entry in (`/name ` or `@path `) instead of sending, Tab does the same except that on a
folder it puts in `@folder/` and goes on listing what is inside, and Escape closes the list until
the next word. A path with spaces is written in quotes. opencode does not say which skills it has,
so for its threads the list holds only qcc's.

A command's or tool's output unfolds to its first 12 lines with "Show all N lines" under them;
it has no scrolling of its own, so the wheel always scrolls the timeline. What you open or close
stays where it is on the screen: the timeline stops following the end until you scroll.

Actions say how long they took after their title, faintly: "Ran cargo test · 47s", "Thought for
12s". One that took under 2 seconds says nothing, which also leaves out the zero durations of
history imported or forked from elsewhere and of items an agent reports only once they are done.
An action still running counts up once it has gone on for 5 seconds. Under the last reply of a
finished turn, beside its copy button, is how long the whole turn took. Hovering a duration shows
when the action started and ended.

Under each of the agent's replies, a copy button (shown while the pointer is over the reply)
puts the reply on the clipboard as the markdown the agent wrote. Selecting text anywhere in the
timeline brings up a **Reply** button over the selection: it adds the selection to the composer
as a markdown quote (`> ` before every line) after whatever is typed there, a blank line apart,
and leaves the caret under the quote. The button goes away with the selection, on scrolling, on
Escape or on a click elsewhere.

Beside the copy button of each turn's last reply, a **…** menu holds what can be done from that
reply; for now, **Fork the thread from here…** (see [Forking a
thread](agents.md#forking-a-thread)). An entry that cannot be used is greyed out, and its tooltip
says why (opencode forks only from its latest reply; a reply from before qcc could fork has no
point to fork from). The dialog asks where the fork works — the same directory, or a new worktree,
the default for a thread that has one, with a word on what does not go along — and for an optional
first message, then opens the fork. A fork has a button back to its thread in the header, and its
timeline starts with a "Forked from …" line; **Show the conversation before** unfolds that
thread's conversation up to the fork above the fork's own, dimmed.

### Mermaid

A ` ```mermaid ` block in an agent's answer is drawn as a diagram (in the light or dark theme),
with a switch to the source. A block with an error is shown as source plus the error text. The
library loads only when a diagram appears; HTML in labels and scripts are forbidden.

### Changes

The list of changed files and the combined diff: for a thread in a worktree, everything it did; for
a thread in its own checkout, what is uncommitted relative to `HEAD`. Once a worktree thread has
commits, a picker at the start of the header (on the tab and in the right panel) narrows the diff
to the uncommitted work, to one commit, or to a run of them: shift-click a second row. The
uncommitted work is a row too, so "this commit and everything after it" is one shift-click. When
the agent amends or rebases away a commit you were looking at, the view goes back to all the
changes. The button at the end of a changed file's header shows the whole file, with the changes
marked in place, and scrolls to the first of them; pressed again, it goes back to the changes
alone. It is per file, kept while you switch between parts of the work, and the whole file is
read again when the agent changes it. While you scroll, the header of the file at the top stays
pinned there until the next file's header pushes it away, so its path, counts and buttons are
always at hand. Code in diffs — here and in
edits in the timeline — is highlighted by the file's language (by extension or name). Collapsed and
very large files stay unhighlighted.

### Line comments

A button next to the line number in a diff (on the Changes tab and in the right panel) opens a
comment field; `Ctrl+Enter` saves. Comments are shown under their lines and follow them when the
code moves. Those whose line is gone are shown above the file or above the diff. While there are
comments, a bar stays above the diff however far you scroll: how many are open (or "Comment 2 of
5" once you step through them), arrows to the previous and next open comment in the order of the
diff (`P`/`N` or `Alt+↑`/`Alt+↓` when not typing; they wrap around at the ends), which open a
folded file, scroll to the comment and mark it for a moment, and **Send to agent**, which
passes the open comments to the agent (see
[Comments on changes](projects-and-worktrees.md#comments-on-changes)). Comments resolved by the
agent are shown struck through, with its note. You can also resolve, reopen or delete a comment
yourself. A comment a reviewer left (see [Reviewers](reviewers.md)) carries the reviewer's name and
the severity it gave; **Outdated** means the changes moved on while it looked, so the line may be
off. A click on the name opens the reviewer's thread. **Sent** marks an open comment already handed
to the agent.

### Right panel

`Ctrl+B` or a button in the thread header. Tabs:

- "Usage": the breakdown of the last request (input, cache, output), a chart of the context per
  request in the current conversation, context compactions, and how many tokens the thread spent
  over all sessions.
- "Context": the window fill and a bar of its parts (conversation, system prompt and tools, MCP,
  memory/skills/agents; the compaction buffer hatched), a list of parts with their share of the
  window and expandable contents, with the "loaded when needed" parts separate. For agents without
  a breakdown, an estimate marked "estimate" (see
  [What the context is made of](agents.md#what-the-context-is-made-of)).
- "Changes": the diff next to the timeline for a thread in a worktree or in its own git checkout,
  re-read as the agent edits files and runs commands.
- "Activity": turns (completed, failed, interrupted, working time), messages, commands, changed
  and read files, tools, MCP calls by server, skills used and programs run (count, failures; the
  tooltip has the average and longest time).
- "Details": agent, model, effort, permission mode, session id, directory and environment, dates.
- "Reviewers" (the `reviewers` plugin): **Run a review**, how each reviewer of the running round
  gets on, and the findings of every round with their status (open, sent, fixed, dismissed),
  grouped by reviewer or severity; `file:line` opens that line in Changes; findings are sent,
  dismissed and restored here. Below, how the thread is reviewed: profiles, agent, model and
  effort, delivery, trigger, delay, rounds at most, and the rounds' history (see
  [Reviewers](reviewers.md)). Reviewers themselves are service threads, kept out of the sidebar
  and the overview; "reasoning" opens one. Plugins can add tabs here the same way.

In a narrow window the panel opens over the thread, and `Esc` closes it. This is the default
layout: the panel's tabs are blocks like everything else (see [Block layout](#block-layout)).

### Open in editor

The button left of the side panel's in the thread header (`Ctrl+Shift+E`, "Open in editor" in
the thread menu, or the palette) opens the directory the thread works in: its `cwd`, which for a
thread in a git worktree is the worktree. It is disabled, with the reason in its tooltip, while
the worktree is not made yet or could not be made, once it is removed, and when the directory is
gone from the disk. If the editor cannot be started or exits with an error at once, the app says
so with what it printed.

The editor is the "Editor" command in Settings, kept in this window like the interface scale:
`code`, `zed`, `idea --new-window`, or a full path. Quotes and backslashes group words as in a
shell. The directory is passed as the last argument, or in place of `{path}` wherever the command
has it (`--folder={path}`). With the field empty the app takes the first of `code`, `cursor`,
`zed`, `zeditor`, `windsurf`, `codium`, `subl`, `idea`, `idea-ultimate`, `idea-community` it finds
(Settings shows which), and with none of them, the system's default app for a directory
(`xdg-open`, `open`, Explorer).

An app started from a desktop menu does not see the PATH of your shell, so programs are looked up
in the app's own PATH, then in the PATH your login shell (`$SHELL -l`, asked once, for at most
three seconds) sets up, then in the usual places: `~/.local/bin`, `~/.nix-profile/bin`,
`/etc/profiles/per-user/$USER/bin`, `/run/current-system/sw/bin`, `/opt/homebrew/bin`,
`/usr/local/bin`, `/usr/bin`, `/snap/bin` and Flatpak's exports. The editor starts with all of them
in its PATH, in the thread's directory, apart from the app: closing the app leaves it open.

### Side panel width

The left (thread list) and right panels can be resized by dragging their edge with the mouse (or
with the arrow keys when the edge has focus); a double click restores the original width. Widths
are remembered, and the thread always stays visible.

### Thread menu

Right-click a thread in the sidebar or a card in the overview: open, open in editor, rename
(`qcc thread rename`), move to a project, mark as read or unread, copy the id or the path, stop the
turn, archive. Modules add items through `ui.threadMenuItem`.

### New thread

`Ctrl+N`: the project and its folder (or any directory), the agent, a separate git worktree (a
switch; off by default and not remembered — decided for each thread) and its base, the model (from
the list the agent returns), effort (the levels of the selected model), the permission mode.

The agent, model, effort and permission mode start as they were for the last thread. The project
does not: the dialog opened with `Ctrl+N`, the "New thread" button or the palette starts with "No
project" and the directory of the last thread started outside a project; opened with a project's
"new thread in the project" button, it starts in that project and the folder last used there (the
first one otherwise).

### Projects

In the sidebar threads are grouped by project. A project has the buttons "new thread in the
project" and "name and folders".

A project can have an icon: the button left of its name in the project dialog opens a grid of
about sixty icons, a palette to color them (the shades follow the light or dark theme) and a field
for an emoji instead; "Default" goes back to the folder icon. The icon is shown in the sidebar,
in the project lists of the new thread and "Move to project" dialogs, and, once one is chosen,
next to the project's name on overview cards, Live tiles and in a thread's details. To create a project: "+ Project" or the palette; to move a thread:
the palette, "Move this thread to a project…".

A project, or a folder of threads outside projects, folds with a click on its heading (or `Enter`
/ `Space` on it; `←` folds, `→` unfolds or goes to its first thread, `←` on a thread goes back to
its heading). `Alt`+click folds or unfolds every group, as does the button next to "+ Project".
Folds are remembered between launches. A folded heading sums up what is hidden: a blue pill with
the number of threads waiting for your answer, red for failed ones, orange for working ones, grey
for threads with a new reply, and how many threads there are. The thread you have open stays in
sight under the heading of its folded group, so going to a thread from the overview or a
notification does not undo a fold; moving a thread into a folded project unfolds it. While the list scrolls,
the heading of the group in view stays at the top.

`Ctrl+K` opens the command palette, `Ctrl+0` the overview.

### Back and forward

Between opened threads and screens, as in a browser: `Alt+←` / `Alt+→`, the side mouse buttons, or
the arrows in the sidebar header.

### Settings

Agents: one folded card per agent. Its header shows the name, a short summary of what its new
threads start with (model, effort, permission mode, how many environment variables), a red mark
when its program is not found, and the "Offer in pickers" switch (whether to show the agent in
lists). A click on the header (or Enter/Space) unfolds the card: the program path, and the model,
effort, permission mode and environment variables of new threads. The default agent; plugins with their version,
description, contributions and permissions, each with a switch that turns it on or off at once
(bundled plugins too); whether to show example plugins (echo). A plugin turned off stops: its
agents, workspaces, commands, skills and screens disappear from the window, and its threads can be
read but take no messages until it is back on. Turning off a plugin whose agents are working asks
first; confirming cuts their turns short. See
[Turning plugins on and off](plugins.md#turning-plugins-on-and-off). Interface scale: `Ctrl+Plus` / `Ctrl+Minus`, as in a
browser, or the buttons on the "Settings" screen; it can be reset to 100% there or from the
palette. Editor: what [Open in editor](#open-in-editor) runs.

### Notifications

When an agent finishes its turn, fails or waits for an answer, the app shows a system notification:
the thread's title, what happened and the start of the agent's last message or question. A click
on it brings the window up and opens the thread (on Linux desktops that report clicks; elsewhere the
notification only informs). Nothing is shown about the thread you have open while the window has
focus. The notifications come from the bundled `notify` plugin: what it tells about (finished turns,
failures, questions, threads started by other agents) is set in its card on the "Settings" screen,
and its switch there turns them off. Notifications reach you only while the app runs; the daemon does
not keep them for later. See [Following every thread and notifying the user](plugins.md#following-every-thread-and-notifying-the-user).

### Automations

The "Automations" screen (calendar-clock icon) and the palette ("Automations", "New automation"):
prompts qcc sends an agent on a schedule (see [automations.md](automations.md)). Each card shows the
name, the schedule (in words on hover for the simple shapes), the prompt, the agent, model and
directory, when it runs next and how its latest run went. Buttons run it now (a run that starts a
thread opens it; a skipped one says why), edit it and delete it (after a confirmation); the switch
turns its schedule on and off — an automation that is off still runs when you run it by hand.
"Recent runs" unfolds the last 20 runs: when, why (scheduled, missed, by hand), what came of it
with the reason, and a link to the thread with its current state.

The dialog for a new or changed automation takes a name, a schedule (an interval like `every 1h`, a
cron expression like `0 9 * * 1-5`, or a shorthand like `@daily`; the buttons fill in examples), the
prompt, the agent, the directory (typed, picked from a project's folders or browsed), a separate git
worktree for each run, model, effort and permission mode, whether each run starts a new thread or
continues one, what to do while the previous run is still at work (skip, or run alongside / queue),
whether to make up for a run missed while qcc was down, and whether it is on. What the daemon
refuses (a schedule it cannot read, a directory that does not exist) is shown in the dialog.

A thread an automation started has a button with the automation's name in its header; it opens the
Automations screen with that automation's runs unfolded.

### MCP servers

The "MCP servers" screen and the palette: qcc servers with an on/off switch, editing and deletion;
an add dialog (a program with arguments and environment, or HTTP with headers); and for each
server, in how many threads it is ready or failed, and why. Below are the servers the agents take
from their own configs. Changes are written to the daemon immediately and reach running threads
too: Claude Code takes them at once, Codex and opencode after the current turn (see
[Changes reach running sessions](agents.md#changes-reach-running-sessions)). A thread has an "MCP" tab
with the status of each server of its session and switches next to the qcc servers that turn them
off in this thread only.

### Skills

The "Skills" screen and the palette: plugin skills with a switch for all threads. A thread has a
"Skills" tab with switches for that thread only.

### Agents started by agents

In the sidebar children stand under their parent, connected to it by a line. A family can be
collapsed with the counter button in the parent's row; a collapsed family shows how many threads
are hidden and whether any of them is waiting for an answer. The selected thread is never hidden.
A thread can be dragged with the mouse: onto the middle of another thread — it becomes its child;
onto the edge between threads — it goes next to them; onto a group header — it becomes standalone.
Moves are allowed within a group. If the daemon would not accept a move (the thread is freer than
the new parent, or the target is its own descendant), a tooltip at the cursor says why before the
button is released. A thread with children has an "Agents" tab (what each one is doing; their
permission requests can be answered right there), and a child has a link to its parent in the
header. Under the input field of such a thread there is a **Started agents** chip: how many
children there are and how many of them are waiting for an answer (in blue, and the chip is
highlighted like the Live button), working, failed and finished. A click opens the list of children
(those waiting first) with a jump to each and a link to the "Agents" tab. In the tab the children
are shown as a tree (grandchildren under their parent; "Direct" keeps only direct children);
archived ones are hidden behind a switch with their count. Filters by status (Waiting, Failed,
Working, Idle, Done) and agent, search by title, sort order (as started, recent, by status). If only
a grandchild matches, its parent stays as a faded path row. The view (archive, depth, order) is
remembered for all threads; filters are remembered per thread while the window is open.

### Plugin commands

In the palette and in the "More…" menu of the thread header; a dialog asks for the parameters.

### Block layout

Screens, thread tabs, right panel tabs and plugin tabs are blocks, and you decide where each one
goes. Slots: the screen buttons at the top of the sidebar and at its foot (MCP, Skills, Settings),
sections at the bottom of the sidebar (collapsible), thread tabs, right panel tabs, and chips under
the input field (a block there is a chip that opens it in a popover). A block that needs a thread
(the timeline, Changes, Usage…) can be placed only in thread slots; the rest (Live, Metrics, plugin
screens…) can go anywhere, for example Live as a sidebar section or Metrics as a thread tab. To
change the layout: Settings → Layout (drag by the handle, arrows, slot choice, "Hidden",
"Reset layout"), or right-click a screen button, a tab or a sidebar section ("Move to…", "Hide",
"Arrange blocks…"). The palette has "Open settings" (`Ctrl+,`), "Arrange blocks" and "Reset the
layout of blocks". The layout is stored in the app itself. Only your changes are remembered, so new
blocks (from a plugin that connected later) take their default places.

## Backend and protocol types

The Tauri backend is a thin proxy: it keeps one connection to `qccd`, passes requests and
notifications through as they are, and starts the daemon itself if it is not running. The one
things it does on its own are what the webview cannot: show system notifications
(`show_notification`) and open a thread's directory in an editor (`open_in_editor`,
`detect_editor`, `directory_exists`; `src-tauri/src/editor.rs`). The daemon
outlives the window: agents keep working when the app is closed. TypeScript types are generated
from `qcc-protocol` (`apps/desktop/src/api/protocol.gen.ts`); a test fails if the file is out of
date:

```sh
QCC_UPDATE_TS=1 cargo test -p qcc-protocol --test typescript   # regenerate
nix develop .#desktop -c ./scripts/check-desktop.sh            # clippy, tsc, vitest, build
```

The interface is modular: everything except the window frame is a contribution to a registry of
extension points. How to add a screen, a thread tab, a timeline item renderer or a command is
described in [apps/desktop/README.md](../apps/desktop/README.md).
