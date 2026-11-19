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
whose worktree has uncommitted work (it is discarded). If a thread still refuses, the app says
which ones stayed and offers to archive them anyway.

A command's or tool's output unfolds to its first 12 lines with "Show all N lines" under them;
it has no scrolling of its own, so the wheel always scrolls the timeline. What you open or close
stays where it is on the screen: the timeline stops following the end until you scroll.

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
changes. Code in diffs — here and in
edits in the timeline — is highlighted by the file's language (by extension or name). Collapsed and
very large files stay unhighlighted.

### Line comments

A button next to the line number in a diff (on the Changes tab and in the right panel) opens a
comment field; `Ctrl+Enter` saves. Comments are shown under their lines and follow them when the
code moves. Those whose line is gone are shown above the file or above the diff. **Send to agent**
passes the open comments to the agent (see
[Comments on changes](projects-and-worktrees.md#comments-on-changes)). Comments resolved by the
agent are shown struck through, with its note. You can also resolve, reopen or delete a comment
yourself.

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

In a narrow window the panel opens over the thread, and `Esc` closes it. This is the default
layout: the panel's tabs are blocks like everything else (see [Block layout](#block-layout)).

### Side panel width

The left (thread list) and right panels can be resized by dragging their edge with the mouse (or
with the arrow keys when the edge has focus); a double click restores the original width. Widths
are remembered, and the thread always stays visible.

### Thread menu

Right-click a thread in the sidebar or a card in the overview: open, rename
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
project" and "name and folders". To create a project: "+ Project" or the palette; to move a thread:
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

Agents: the program path, and the model, effort, permission mode and environment variables of new
threads, and whether to show the agent in lists; the default agent; plugins with their version,
description, contributions and permissions, each with a switch that turns it on or off at once
(bundled plugins too); whether to show example plugins (echo). A plugin turned off stops: its
agents, workspaces, commands, skills and screens disappear from the window, and its threads can be
read but take no messages until it is back on. Turning off a plugin whose agents are working asks
first; confirming cuts their turns short. See
[Turning plugins on and off](plugins.md#turning-plugins-on-and-off). Interface scale: `Ctrl+Plus` / `Ctrl+Minus`, as in a
browser, or the buttons on the "Settings" screen; it can be reset to 100% there or from the
palette.

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
off in this thread only. A failed server is also visible on the card in the overview.

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
notifications through as they are, and starts the daemon itself if it is not running. The daemon
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
