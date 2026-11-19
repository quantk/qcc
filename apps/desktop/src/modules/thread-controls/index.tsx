// The controls in a thread's header: its agent and model, its permission mode and
// putting it away.

import { useState } from "react";

import type { PermissionMode, Thread } from "../../api/protocol.gen";
import { allowedModes, modeLabel } from "../../state/selectors";
import { store } from "../../state/store";
import { Choice } from "../../ui/kit";
import type { ThreadContext, UiModule, UiRegistry } from "../../ui/registry";
import { archiveThread } from "../thread-menu/ArchiveDialog";
import { Button } from "@/components/ui/button";
import { AgentSwitch } from "./AgentSwitch";

function ModeSelect({ thread, provider, parent }: ThreadContext) {
  // A child may not run looser than its parent, so looser modes are not offered.
  const { modes, agentDefault } = allowedModes(provider?.capabilities.permission_modes ?? [], parent);
  const active = thread.status === "active";
  return (
    <Choice
      size="sm"
      aria-label="Permission mode"
      value={thread.permission_mode ?? null}
      disabled={active}
      title={
        active
          ? "Changes once the running turn ends"
          : parent
            ? "How much the agent may do unasked; no more than the thread that started it"
            : "How much the agent may do unasked"
      }
      options={[
        ...(agentDefault ? [{ value: null, label: "Agent default" }] : []),
        ...modes.map((mode) => ({ value: mode, label: modeLabel[mode] })),
      ]}
      onChange={(value) => void store().setMode(thread.id, value as PermissionMode | null)}
    />
  );
}

function ArchiveButton({ thread, ui }: { thread: Thread; ui: UiRegistry }) {
  const [working, setWorking] = useState(false);
  const archive = async () => {
    setWorking(true);
    await archiveThread(ui, thread);
    setWorking(false);
  };
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={working || thread.status === "active"}
      title={
        thread.status === "active"
          ? "Stop the running turn first"
          : "Release the session and put the thread away, with the threads it started"
      }
      onClick={() => void archive()}
    >
      {working ? "Archiving…" : "Archive"}
    </Button>
  );
}

export const threadControlsModule: UiModule = {
  id: "core.thread-controls",
  activate(ui) {
    ui.threadAction({
      id: "agent",
      order: -10,
      render: ({ thread }) => <AgentSwitch thread={thread} />,
    });
    ui.threadAction({
      id: "permission-mode",
      order: 0,
      when: ({ thread, provider }) => !thread.archived && (provider?.capabilities.permission_modes ?? []).length > 0,
      render: ModeSelect,
    });
    ui.threadAction({
      id: "archive",
      order: 100,
      when: ({ thread }) => !thread.archived,
      render: ({ thread }) => <ArchiveButton thread={thread} ui={ui} />,
    });
  },
};
