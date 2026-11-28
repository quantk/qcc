// Mentions: the composer's `@` list of the files and folders in the thread's
// working directory (its worktree, for a thread in one), found by the daemon.

import { FileIcon, FolderIcon } from "lucide-react";

import { mentionPath } from "../../state/completion";
import { store } from "../../state/store";
import type { ComposerCompletion, UiModule } from "../../ui/registry";

const pathCompletion: ComposerCompletion = {
  id: "mentions.paths",
  trigger: "@",
  order: 10,
  title: "Files and folders",
  // A thread whose directory is still being prepared has none to offer.
  when: ({ thread }) => thread.environment?.state !== "pending",
  async complete(query, { thread }) {
    const found = await store().findPaths(thread.id, query);
    return found.paths.map(({ path, dir, positions }) => ({
      key: path,
      label: dir ? `${path}/` : path,
      positions,
      icon: dir ? FolderIcon : FileIcon,
      insert: `${mentionPath(path, dir ?? false)} `,
      // Tab goes into a folder: the list goes on with what is in it.
      descend: dir ? mentionPath(path, true) : undefined,
    }));
  },
};

export const mentionsModule: UiModule = {
  id: "core.mentions",
  activate(registry) {
    registry.composerCompletion(pathCompletion);
  },
};
