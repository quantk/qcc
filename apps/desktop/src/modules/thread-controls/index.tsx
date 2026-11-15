// The controls in a thread's header: its agent and model, its permission mode and
// putting it away.

import { useState } from "react";

import type { PermissionMode, Thread } from "../../api/protocol.gen";
import { allowedModes, modeLabel } from "../../state/selectors";
import { store } from "../../state/store";
import { Choice } from "../../ui/kit";
import type { ThreadContext, UiModule } from "../../ui/registry";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
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

function ArchiveButton({ thread }: { thread: Thread }) {
  const [refused, setRefused] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  const archive = async (force: boolean) => {
    setWorking(true);
    const result = await store().archive(thread.id, force);
    setWorking(false);
    setRefused(result?.refused ?? null);
  };

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        disabled={working || thread.status === "active"}
        title={thread.status === "active" ? "Stop the running turn first" : "Release the session and put the thread away"}
        onClick={() => void archive(false)}
      >
        {working ? "Archiving…" : "Archive"}
      </Button>
      <AlertDialog open={refused !== null} onOpenChange={(open) => !open && setRefused(null)}>
        <AlertDialogContent className="data-[size=default]:sm:max-w-lg">
          <AlertDialogHeader>
            <AlertDialogTitle>Archive and discard the changes?</AlertDialogTitle>
            <AlertDialogDescription>
              {refused}. Archiving removes the thread's working directory. Work that is not committed there is lost.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep the thread</AlertDialogCancel>
            <AlertDialogAction variant="destructive" disabled={working} onClick={() => void archive(true)}>
              Archive anyway
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
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
      render: ({ thread }) => <ArchiveButton thread={thread} />,
    });
  },
};
