# qcc desktop

The control panel for qcc: a Tauri window over the daemon's API.

```
src-tauri/        the backend: one connection to qccd, requests and notifications passed through
src/api/          the typed door to the daemon; protocol.gen.ts is generated from qcc-protocol
src/state/        the store (mirror of the daemon) and pure folds: timeline, diff, block layout
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
applies to a thread. The shell has five **slots** that show blocks (`state/blocks.ts`):

| Slot | Where | Gives |
| --- | --- | --- |
| `views` | the screens' buttons at the top of the sidebar | nothing |
| `views.foot` | the screens' buttons at its foot: setup and preferences | nothing |
| `sidebar` | folding sections at the bottom of the sidebar | nothing |
| `thread.tabs` | the tabs of the thread view | the thread |
| `thread.panel` | the tabs of the side panel at the right of a thread | the thread |

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
| A block: a screen, a sidebar section, a thread tab or a side panel tab | `ui.block` | `modules/overview`, `modules/live`, `modules/metrics`, `modules/timeline`, `modules/changes`, `modules/insights`, `modules/lineage`, `modules/mcp`, `modules/skills` |
| A control in the thread header | `ui.threadAction` | `modules/thread-controls`, `modules/lineage` |
| An entry of a thread's right-click menu | `ui.threadMenuItem` | `modules/thread-menu`, `modules/projects` |
| A button on a project or folder in the sidebar | `ui.groupAction` | `modules/new-thread`, `modules/projects` |
| How a timeline item is drawn | `ui.itemRenderer` | `modules/timeline/items.tsx` |
| A command (palette, shortcut, sidebar buttons) | `ui.command` | `modules/new-thread`, `modules/projects`, `modules/settings` |
| A modal dialog | `ui.showDialog` | `modules/new-thread` |

A screen is a button in the sidebar, so a block gives an `icon` (a lucide component;
plugin blocks get a generic one) and may give `useBadge`, a count on the button (a
hook, called while the button is drawn; see `modules/live`). Whether a screen's
button stands in the top row or at the foot with setup and preferences is not a
property of the block but its slot, `views` or `views.foot`, so the user can move it.
The navigation module binds `Alt+1`…`Alt+9` to the screens in the order of their
buttons (`src/ui/views.ts`).

Header controls, menu entries and item renderers are not blocks: they are small parts of
a frame the shell already draws, not units the user would move.

The side panel stays in view beside whichever thread tab is open, so it is the place
for what is worth glancing at while reading the timeline. By default
`modules/insights` puts three tabs there: Usage (how full the context is, the context at each request, compactions,
tokens the thread spent over all its sessions), Activity (turns and their times,
messages, commands, files, tools) and Details (agent, model, permissions, session,
directory, dates). The numbers come from `state/stats.ts`, folded from the same events
as the timeline. `modules/changes` adds a compact Changes there (block `changes.panel`; the full
view is the `changes` tab): the diff of a thread's worktree, or of
the uncommitted work in its own git checkout,
read again whenever the agent settles an edit or a command, so it keeps up beside the
timeline. Diffs (`ui/kit/DiffView`) are highlighted with shiki by the language the file
name suggests (`state/languages.ts`); a view adds its own rows to a diff through a
`DiffAddon`, which is how `modules/changes/review.tsx` puts comments under lines.
Where a comment goes once the code has moved is decided in `state/comments.ts`; the
comments themselves live in the store, read with `comment.list` and kept current by
`comments.updated`. Ctrl+B or the button in the thread header shows and hides the panel;
in a window too narrow for it, it opens over the thread and Escape puts it away.

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

List it in `src/modules/index.ts` and it is part of the app. `ui.deactivate(id)` takes
out everything the module registered; activating the same id again replaces it, which
is how contributions that arrive at runtime stay current.

## Plugin UI

Plugins of the daemon contribute through the same registry. `modules/plugin-ui` reads
`ui.contributions` on every connection and turns each entry into a registration: a
block (an `app` view becomes a block of the screens, a `threadTab` a thread block
offered for all threads or only for the plugin's own providers; the user can move
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
