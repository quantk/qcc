# qcc desktop

The control panel for qcc: a Tauri window over the daemon's API.

```
src-tauri/        the backend: one connection to qccd, requests and notifications passed through
src/api/          the typed door to the daemon; protocol.gen.ts is generated from qcc-protocol
src/state/        the store (mirror of the daemon) and pure folds: timeline, diff
src/ui/registry   the extension points
src/ui/kit/       shared building blocks: Markdown, DiffView, Disclosure, InteractionPrompt
src/shell/        the frame: sidebar, thread frame, command palette
src/modules/      every feature, each a module that registers contributions
```

Run it with `./dev desktop` from the repository root; check it with
`nix develop .#desktop -c ./scripts/check-desktop.sh`.

## Modules

The shell draws a frame and nothing else. Whatever fills it is contributed through
`UiRegistry` (`src/ui/registry.ts`), and the built-in features use the same calls a
plugin's UI does:

| Extension point | Call | Built-in example |
| --- | --- | --- |
| A screen in the sidebar | `ui.appView` | `modules/overview` |
| A tab of the thread view | `ui.threadTab` | `modules/timeline`, `modules/changes` |
| A control in the thread header | `ui.threadAction` | `modules/thread-controls` |
| How a timeline item is drawn | `ui.itemRenderer` | `modules/timeline/items.tsx` |
| A command (palette, shortcut) | `ui.command` | `modules/new-thread` |
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

Rules that keep modules independent:

- A module talks to the daemon through `store().api.call(...)` and reads state with
  `useApp(...)`. It does not reach into another module's files; shared pieces go to
  `src/ui/kit`.
- Per-thread contributions get a `ThreadContext` (`thread`, `timeline`, `provider`) and
  decide with `when` whether they apply. The shell never asks which provider or which
  environment a thread has.
- A timeline item without a renderer is shown as its data, so a new item kind is
  visible before anyone draws it.
