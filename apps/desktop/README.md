# qcc desktop

The control panel for qcc: a Tauri window over the daemon's API.

```
src-tauri/        the backend: one connection to qccd, requests and notifications passed through;
                  system notifications (notifications.rs), which the webview cannot show, and
                  starting the user's editor on a directory (editor.rs)
src/api/          the typed door to the daemon; protocol.gen.ts is generated from qcc-protocol
src/state/        the store (mirror of the daemon) and pure folds: timeline, diff, block layout,
                  delegated threads (children.ts: states, counts, filters of the Agents tab)
src/ui/registry   the extension points; src/ui/blocks: slots drawn under the user's layout
src/ui/kit/       shared building blocks: Markdown, DiffView, InteractionPrompt,
                  ItemTrace and its kin (the AI Elements Terminal, Tool and Reasoning patterns)
src/components/ui shadcn components, added with the shadcn CLI and owned by us from then on
src/shell/        the frame: sidebar, thread frame, command palette
src/modules/      every feature, each a module that registers contributions
```

## Styling

The app's own styles live in `src/styles.css`, inside Tailwind's `base` layer.
Components come from [shadcn/ui](https://ui.shadcn.com) on Base UI: `pnpm dlx shadcn@latest
add <name>` copies one into `src/components/ui`, where it is ours to change. Patterns of
[AI Elements](https://ui.shadcn.com/docs/ai-elements) (Tool, Terminal, Reasoning, …) are not
installed; they are compositions of those components, written in `src/ui/kit` against
timeline items rather than AI SDK messages.

`src/theme.css` maps shadcn's tokens (`--background`, `--primary`, `--muted`, …) onto the
palette, so a component follows the app's colors and its light and dark scheme. Because
the app's rules sit in a layer, a utility class on a component always wins over them; the
app's global `button` rule skips elements with a `data-slot`, which is how shadcn
components mark themselves.

Run it with `./dev desktop` from the repository root; check it with
`nix develop .#desktop -c ./scripts/check-desktop.sh`.

## Modules

The shell draws a frame and nothing else. Whatever fills it is contributed through
`UiRegistry` (`src/ui/registry.ts`), and the built-in features use the same calls a
plugin's UI does.

### Blocks and slots

Most of the window is made of **blocks**: a block is a unit of UI with an id, a title,
the context it needs and a `render`. A block of scope `app` needs nothing; a block of
scope `thread` is drawn with a `ThreadContext` and may say with `when` whether it
applies to a thread. The window has six **slots** that show blocks (`state/blocks.ts`):

| Slot | Where | Gives |
| --- | --- | --- |
| `views` | the screens' buttons at the top of the sidebar | nothing |
| `views.foot` | the screens' buttons at its foot: setup and preferences | nothing |
| `sidebar` | folding sections at the bottom of the sidebar | nothing |
| `thread.tabs` | the tabs of the thread view | the thread |
| `thread.panel` | the tabs of the side panel at the right of a thread | the thread |
| `thread.composer` | chips in the bar under the message box, each opening its block in a popover | the thread |

A block goes into any slot that gives what it needs: an app's block anywhere, a
thread's block only beside a thread. Each block names its default `placement` (a slot
and an order, or no slot: offered but hidden); the user's **layout** moves, orders and
hides blocks over those defaults. The layout keeps only what the user changed, by
block id, in the window's storage (`qcc.block-layout`), so a block that comes later (a
plugin's, on the next connection) takes its default place, and one that is gone for now
keeps its place for when it comes back. The logic is pure and tested in
`state/blocks.ts`; `ui/blocks.tsx` has the hooks the shell draws slots with
(`useSlot`) and `placeBlock`.

The user changes the layout in Settings → Layout (drag a block by its handle, or use
its arrows and its slot menu; Reset layout puts everything back), or by right-clicking
a screen's button, a tab or a sidebar section (move it to another slot, hide it, Arrange blocks…). The
palette has `Open settings` (Ctrl+,), `Arrange blocks` and `Reset the layout of blocks`, so
a screen hidden by mistake, Settings included, is always one step from coming back. A
screen opened by name (a command or a link) shows even when its block was moved out of
the screens' list.

A slot wraps each block in a column that scrolls: a block that scrolls by itself (the
timeline, a diff) fills it, and one that does not scrolls in it, so the same block
works as a screen, a tab or a panel tab.

### Extension points

| Extension point | Call | Built-in example |
| --- | --- | --- |
| A block: a screen, a sidebar section, a thread tab, a side panel tab or a chip beside the composer | `ui.block` | `modules/overview`, `modules/live`, `modules/metrics`, `modules/limits`, `modules/timeline`, `modules/changes`, `modules/insights`, `modules/context`, `modules/lineage`, `modules/mcp`, `modules/skills` |
| A control in the thread header | `ui.threadAction` | `modules/thread-controls`, `modules/lineage`, `modules/open-in-editor` |
| An entry of a thread's right-click menu | `ui.threadMenuItem` | `modules/thread-menu`, `modules/projects` |
| A button on a project or folder in the sidebar | `ui.groupAction` | `modules/new-thread`, `modules/projects` |
| How a timeline item is drawn | `ui.itemRenderer` | `modules/timeline/items.tsx` |
| An entry of the "…" menu under a reply, disabled with a reason when it cannot run | `ui.itemMenuItem` | `modules/fork` |
| A completion list in the composer, opened by a trigger character (`/`, `@`) | `ui.composerCompletion` | `modules/skills`, `modules/mentions` |
| A command (palette, shortcut, sidebar buttons) | `ui.command` | `modules/new-thread`, `modules/projects`, `modules/settings` |
| A modal dialog | `ui.showDialog` | `modules/new-thread` |

A screen is a button in the sidebar, so a block gives an `icon` (a lucide component;
plugin blocks get a generic one) and may give `useBadge`, a count on the button (a
hook, called while the button is drawn; see `modules/live`). Whether a screen's
button stands in the top row or at the foot with setup and preferences is not a
property of the block but its slot, `views` or `views.foot`, so the user can move it.
The navigation module binds `Alt+1`…`Alt+9` to the screens in the order of their
buttons (`src/ui/views.ts`).

A block beside the composer stands as a chip: its `summary` (a component given the
same context as `render`, drawn inside the chip's button), or its icon and title when
it has none; a click opens `render` in a popover. A summary that marks an element
with `data-attention` tints the chip, for something that waits for the user. The
timeline module draws this slot under its message box (`BlockChips` from
`ui/blocks.tsx`), so the chips are there wherever the timeline is. By default
`modules/lineage` puts **Started agents** there (block `agents.glance`), shown only
for a thread that started others: how many it started, with how many wait for the
user (blue, and the chip is tinted, as on the Live button), work, failed or are done
(`state/lineage.ts`). Its popover lists them, the ones that want the user first, each
leading to its thread, and leads to the full **Agents** tab where their questions can
be answered. A block can be brought into view where the layout put it with
`showThreadBlock(id)` from `ui/blocks.tsx`.

The `sidebar` slot is for what is worth having always in view whatever is open. By
default `modules/limits` puts **Usage limits** there (block `limits.glance`): the
subscription windows of each agent's account, which belong to the provider and not to a
thread, as small bars with a popover of details, and offers `limits.view` with all of
them. The store mirrors the daemon's `provider.limits` (kept current by
`limits.updated`) and asks the agents again (`refreshLimits`) when it connects or the
details open and the numbers are older than five minutes; what to show, how worrying
it is and how old it is comes from `state/limits.ts`.

Header controls, menu entries and item renderers are not blocks: they are small parts of
a frame the shell already draws, not units the user would move.

The side panel stays in view beside whichever thread tab is open, so it is the place
for what is worth glancing at while reading the timeline. By default
`modules/insights` puts three tabs there: Usage (the last request, the context at each
request, compactions, tokens the thread spent over all its sessions), Activity (turns and
their times, messages, commands, files, and from `state/tools.ts` the tools, MCP calls by
server, skills and programs used) and Details (agent, model, permissions, session,
directory, dates). The numbers come from `state/stats.ts`, folded from the same events
as the timeline. `modules/context` adds Context (block `context.window`): how full the
window is and what it holds, from `state/context.ts` — the agent's own breakdown
(`context/usage` events, or the thread's projection while the timeline loads), else a
reckoning from the usage reports marked as an estimate. Each part keeps one color group
whatever else the window holds. The Metrics view adds up the same tool rows across threads
from the daemon's `usage.tools` buckets (`modules/metrics/ToolsSection.tsx`).
`modules/changes` adds a compact Changes there (block `changes.panel`; the full
view is the `changes` tab): the diff of a thread's worktree, or of
the uncommitted work in its own git checkout,
read again whenever the agent settles an edit or a command, so it keeps up beside the
timeline. Diffs (`ui/kit/DiffView`) are highlighted with shiki by the language the file
name suggests (`state/languages.ts`); a view adds its own rows to a diff through a
`DiffAddon`, which is how `modules/changes/review.tsx` puts comments under lines.

Changes stay fast however much the agent changed. The tab and the panel share one reader
per thread (`state/changes.ts`): it reads the list of files (`thread.changes`) and then
the diff of only the files in view, a batch at a time (`thread.diff` with `paths`), keeps
what it read while the agent works and reads a file again only when it is back in view,
keeping the very object on screen when nothing in it changed. Lockfiles, build output and
files with more than 400 changed lines start folded; binary files are not read at all.
`modules/changes/ChangesDiff.tsx` draws the diff as one virtualized list of rows
(`state/diffRows.ts`: file headers, lines of a fixed height, comments, notes) that scrolls
sideways as one, and keeps the row at the top in place while diffs above it come in. The
lines in view are highlighted by a worker (`lib/diffHighlights.ts`,
`lib/highlight.worker.ts`), a hunk side at a time; long lines and files over 10 000 lines
stay plain. `bench/` measures it in headless Chrome (see `bench/README.md`).
Where a comment goes once the code has moved is decided in `state/comments.ts`; the
comments themselves live in the store, read with `comment.list` and kept current by
`comments.updated`. Ctrl+B or the button in the thread header shows and hides the panel;
in a window too narrow for it, it opens over the thread and Escape puts it away.

The composer takes attachments: the paperclip opens the system file dialog, `Ctrl+V`
pastes images or files from the clipboard (a file manager's copied files come as a
`text/uri-list`), and files dropped anywhere on the window (Tauri's drag-drop event)
join the message being written. Picked files go to the daemon by path; pasted ones by
their bytes. Thumbnails of picked pictures are read by the backend command
`attachment_preview`, since the webview cannot read files. The pure part lives in
`state/attachments.ts`; `ui/kit/Attachments.tsx` draws the tiles in the composer and
under a sent message, whose pictures are read back with `thread.attachment`. Whether
the agent looks at images is its provider's `images` capability; the paperclip's hint
says when it does not.

The composer completes a word that starts with a trigger character at the start of
the message or after whitespace: `/` for skills, `@` for paths. The lists come from
`ui.composerCompletion` sources, each with its trigger, a heading and an (often async)
`complete(query, { thread, provider })`; an answer that arrives after a newer question
is dropped. `ui/kit/Completion.tsx` holds the hook that finds the word under the caret,
asks the sources and takes the keys (↑/↓, Enter, Tab, Escape) before the composer's
own Enter, and the list, a cmdk `Command` whose selection the hook controls while the
focus stays in the textarea. The word and its replacement are `state/completion.ts`;
`state/fuzzy.ts` ranks what the window holds (skills) the way the daemon ranks paths
(`thread.paths`, in `crates/qcc-daemon/src/paths.rs`). The `/` list has two sources in
`modules/skills`: the thread's qcc skills that are on with its agent's own, and the
agent's commands (minus the names that are skills). Both come from
`store.agentCatalogs` (`state/agentCatalog.ts`), which asks `provider.skills` and
`provider.commands` about the thread's directory and keeps the answer. A source's
`opened` runs once each time the list opens for a word: there the catalog looks again
with `refresh` in the background, throttled per thread, since asking may run the
agent. A source's `subscribe` tells the hook that it found something new, and the
hook asks `complete` again, keeping the chosen entry. The `@` list asks the daemon on
each keystroke and lets it keep the directory's scan.

The timeline reaches its composer through `modules/timeline/composerInput.ts`: a
thread's composer listens while it is drawn and takes the caret and the focus, and
text sent while it is not drawn waits in the thread's draft. That is how the Reply
button over a selection (`QuoteSelection.tsx`, drawn outside the scrolling viewport,
whose paint containment would clip it) adds a quote, formatted by `state/quote.ts`.
Copying goes through `util/clipboard.ts`, the webview's clipboard API, and `CopyButton`
in `ui/kit` (the reply's copy button under each agent message).

A project's icon is drawn everywhere by `ProjectIconView` (`ui/kit/ProjectIcon.tsx`),
which also holds the curated set of lucide icons the picker offers and the palette: its
colors are the `--brand`, `--red`, `--orange`, … variables of `styles.css`, so a project
follows the light or dark theme. A name outside the set draws the default icon. What a
pick in the picker (`modules/projects/IconPicker.tsx`) does is in `state/projectIcon.ts`.

Both sides of the window can be resized by dragging their edge (`ui/kit/ResizeHandle`,
arrow keys when it has focus, a double click resets it). The widths live in the store
and are bounded by `state/layout.ts`, which also keeps room for the thread.

A module is an id and an `activate` function:

```tsx
// src/modules/notes/index.tsx
import type { UiModule } from "../../ui/registry";

export const notesModule: UiModule = {
  id: "notes",
  activate(ui) {
    ui.block({
      id: "notes",
      title: "Notes",
      hint: "Notes kept with the thread",
      scope: "thread",
      placement: { slot: "thread.tabs", order: 20 },
      when: ({ thread }) => !thread.archived,
      render: ({ thread }) => <NotesView thread={thread} />,
    });
    ui.itemRenderer("extension:notes/card", ({ item }) => <NoteCard item={item} />);
    ui.command({ id: "notes.open", title: "Open notes", shortcut: "Ctrl+Shift+N", run: openNotes });
  },
};
```

An item renderer is given the item, its thread and `timing`, when the item started and
settled, which `ItemTrace` turns into a duration after the title (`timing` prop; the
thresholds are in `state/durations.ts`); the last reply of an ended turn also gets
`turn`, the whole turn's times.

List it in `src/modules/index.ts` and it is part of the app. `ui.deactivate(id)` takes
out everything the module registered; activating the same id again replaces it, which
is how contributions that arrive at runtime stay current.

## Plugin UI

Plugins of the daemon contribute through the same registry. `modules/plugin-ui` reads
`ui.contributions` on every connection and turns each entry into a registration: a
block (an `app` view becomes a block of the screens, a `threadTab` a thread block
offered for all threads or only for the plugin's own providers, a `threadPanel` the
same in the panel beside the thread; the user can move
either to any slot that fits it, like a built-in block) or the renderer of the plugin's
extension items. What a plugin draws is a tree of UI nodes;
`ui/kit/NodeView.tsx` draws the closed set of node types with the app's own components
and ignores everything else, so a plugin's view is data and never code.

A view can also be a page of the plugin's web bundle (`web` in the contribution).
`modules/plugin-ui/PluginWebView.tsx` shows it in a sandboxed frame served by the
backend under `qcc-plugin://<plugin-id>/`: an origin of its own, a response policy
without network access, no Tauri IPC. The page reaches its plugin through
`postMessage({ qcc: 1, id, action, value })`; the app forwards it as `ui.action` for
that plugin only and posts the answer back.

Adding a node type means adding a case to `NodeView` and a builder to
`crates/qcc-plugin-sdk/src/ui.rs`.

A view is asked for again when its thread settles and when its plugin says it changed
(`ui.changed`, counted per view and per thread in `state/uiChanges.ts`).

Plugin commands (`command.list`) go the same way: `modules/plugin-commands` registers
each as a palette command, offers the ones that act on a thread in the thread header,
and asks for their parameters in a dialog before `command.run`.

Rules that keep modules independent:

- A module talks to the daemon through `store().api.call(...)` and reads state with
  `useApp(...)`. It does not reach into another module's files; shared pieces go to
  `src/ui/kit`.
- Per-thread contributions get a `ThreadContext` (`thread`, `timeline`, `provider`,
  `parent`, `children`) and
  decide with `when` whether they apply. The shell never asks which provider or which
  environment a thread has.
- A block does not assume its slot: it may be a screen today and a narrow panel tab
  tomorrow. Draw for any width, and keep block ids stable, since the user's layout
  remembers places by them.
- A timeline item without a renderer is shown as its data, so a new item kind is
  visible before anyone draws it.
- The timeline is virtualized (`@tanstack/react-virtual`): only entries near the view
  are mounted, so a renderer is unmounted when its item scrolls away. State that should
  survive that, like whether something is unfolded, goes through `useRemembered` from
  `src/ui/kit` instead of `useState`; `ItemTrace` already does this.
- WebKit has no scroll anchoring. Something in the timeline that opens or closes on a
  click calls `keep(element)` from `useKeepInPlace` (`src/ui/kit`) right before the
  change: the element stays put on the screen, and the timeline stops following its end
  until the reader scrolls. Do not put a box that scrolls by itself into the timeline:
  it takes the wheel from it; show the start and a button for the rest, as `Code` in
  `AgentTrace.tsx` does.
