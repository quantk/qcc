# qcc desktop

The control panel for qcc: a Tauri window over the daemon's API.

```
src-tauri/        the backend: one connection to qccd, requests and notifications passed through
src/api/          the typed door to the daemon; protocol.gen.ts is generated from qcc-protocol
src/state/        the store (mirror of the daemon) and pure folds: timeline, diff
src/ui/registry   the extension points
src/ui/kit/       shared building blocks: Markdown, DiffView, Disclosure, InteractionPrompt,
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
app's global `button` and `select` rules skip elements with a `data-slot`, which is how
shadcn components mark themselves.

Run it with `./dev desktop` from the repository root; check it with
`nix develop .#desktop -c ./scripts/check-desktop.sh`.

## Modules

The shell draws a frame and nothing else. Whatever fills it is contributed through
`UiRegistry` (`src/ui/registry.ts`), and the built-in features use the same calls a
plugin's UI does:

| Extension point | Call | Built-in example |
| --- | --- | --- |
| A screen in the sidebar | `ui.appView` | `modules/overview`, `modules/mcp` |
| A tab of the thread view | `ui.threadTab` | `modules/timeline`, `modules/changes`, `modules/lineage`, `modules/mcp` |
| A control in the thread header | `ui.threadAction` | `modules/thread-controls`, `modules/lineage` |
| An entry of a thread's right-click menu | `ui.threadMenuItem` | `modules/thread-menu`, `modules/projects` |
| How a timeline item is drawn | `ui.itemRenderer` | `modules/timeline/items.tsx` |
| A command (palette, shortcut) | `ui.command` | `modules/new-thread`, `modules/settings` |
| A modal dialog | `ui.showDialog` | `modules/new-thread` |

A module is an id and an `activate` function:

```tsx
// src/modules/notes/index.tsx
import type { UiModule } from "../../ui/registry";

export const notesModule: UiModule = {
  id: "notes",
  activate(ui) {
    ui.threadTab({
      id: "notes",
      title: "Notes",
      order: 20,
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
`ui.contributions` on every connection and turns each entry into a registration: an app
view, a thread tab (offered for all threads or only for the plugin's own providers) or
the renderer of the plugin's extension items. What a plugin draws is a tree of UI nodes;
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
- A timeline item without a renderer is shown as its data, so a new item kind is
  visible before anyone draws it.
