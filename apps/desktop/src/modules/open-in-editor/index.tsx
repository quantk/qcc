// Opening the directory a thread works in (its worktree, for a thread in one) in
// the user's editor: a button in the thread's header, an entry of its menu and a
// command. The backend finds and starts the editor; see src-tauri/src/editor.rs.

import { isTauri } from "@tauri-apps/api/core";
import { SquareArrowOutUpRightIcon } from "lucide-react";
import { useEffect, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { directoryExists, openInEditor } from "../../api/editor";
import { editorTarget } from "../../state/editor";
import { store } from "../../state/store";
import type { ThreadContext, UiModule } from "../../ui/registry";
import { errorMessage } from "../../util/format";
import { Button } from "@/components/ui/button";
import { getEditor, useEditor } from "./preference";

/** Open the thread's directory, or say on screen why it did not open. */
export async function openThreadInEditor(thread: Thread): Promise<void> {
  const exists = await directoryExists(thread.cwd).catch(() => undefined);
  const target = editorTarget(thread, exists);
  if ("reason" in target) {
    store().showError(`Cannot open in an editor: ${target.reason}`);
    return;
  }
  try {
    await openInEditor(target.dir, getEditor());
  } catch (error) {
    store().showError(`Cannot open ${target.dir} in an editor: ${errorMessage(error)}`);
  }
}

/** Whether the directory is there, asked again when the thread's directory or environment changes. */
function useDirectoryExists(thread: Thread): boolean | undefined {
  const [exists, setExists] = useState<{ path: string; exists: boolean }>();
  const state = thread.environment?.state;
  useEffect(() => {
    let current = true;
    directoryExists(thread.cwd)
      .then((exists) => current && setExists({ path: thread.cwd, exists }))
      .catch(() => undefined);
    return () => {
      current = false;
    };
  }, [thread.cwd, state]);
  return exists?.path === thread.cwd ? exists.exists : undefined;
}

function OpenInEditorButton({ thread }: ThreadContext) {
  const command = useEditor();
  const target = editorTarget(thread, useDirectoryExists(thread));
  const [opening, setOpening] = useState(false);
  const editor = command ? command.split(/\s+/)[0] : "the editor";
  const title = "reason" in target ? target.reason : `Open ${target.dir} in ${editor} (Ctrl+Shift+E)`;
  const open = async () => {
    setOpening(true);
    await openThreadInEditor(thread);
    setOpening(false);
  };
  return (
    // A disabled button takes no pointer events, so the tooltip is on its wrapper.
    <span title={title} className="inline-flex">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Open in editor"
        disabled={"reason" in target || opening}
        onClick={() => void open()}
      >
        <SquareArrowOutUpRightIcon />
      </Button>
    </span>
  );
}

function selectedThread(): Thread | undefined {
  const { selected, threads } = store().getState();
  return selected ? threads[selected] : undefined;
}

export const openInEditorModule: UiModule = {
  id: "core.open-in-editor",
  activate(ui) {
    // Only the app's backend can start a program; the frontend alone in a browser offers nothing.
    if (!isTauri()) return;
    ui.threadAction({
      id: "open-in-editor",
      // Last of the contributed controls, beside the side panel's button the frame draws after them.
      order: 1000,
      render: OpenInEditorButton,
    });
    ui.threadMenuItem({
      id: "open-in-editor",
      title: "Open in editor",
      order: 1,
      group: "open",
      when: (thread) => "dir" in editorTarget(thread, undefined),
      run: (thread) => void openThreadInEditor(thread),
    });
    ui.command({
      id: "thread.open-in-editor",
      title: "Open the thread's directory in the editor",
      shortcut: "Ctrl+Shift+E",
      enabled: () => {
        const thread = selectedThread();
        return thread !== undefined && "dir" in editorTarget(thread, undefined);
      },
      run: () => {
        const thread = selectedThread();
        if (thread) void openThreadInEditor(thread);
      },
    });
  },
};
