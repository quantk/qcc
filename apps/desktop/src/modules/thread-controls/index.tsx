// The controls in a thread's header: its permission mode and putting it away.

import { useEffect, useRef, useState } from "react";

import type { PermissionMode, Thread } from "../../api/protocol.gen";
import { modeLabel } from "../../state/selectors";
import { store } from "../../state/store";
import type { ThreadContext, UiModule } from "../../ui/registry";

function ModeSelect({ thread, provider }: ThreadContext) {
  const modes = provider?.capabilities.permission_modes ?? [];
  const active = thread.status === "active";
  return (
    <select
      className="small"
      value={thread.permission_mode ?? ""}
      disabled={active}
      title={active ? "Changes once the running turn ends" : "How much the agent may do unasked"}
      onChange={(event) => void store().setMode(thread.id, (event.target.value || null) as PermissionMode | null)}
    >
      <option value="">Agent default</option>
      {modes.map((mode) => (
        <option key={mode} value={mode}>
          {modeLabel[mode]}
        </option>
      ))}
    </select>
  );
}

function ArchiveButton({ thread }: { thread: Thread }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [refused, setRefused] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  useEffect(() => {
    if (refused) dialog.current?.showModal();
  }, [refused]);

  const archive = async (force: boolean) => {
    setWorking(true);
    const result = await store().archive(thread.id, force);
    setWorking(false);
    setRefused(result?.refused ?? null);
  };

  return (
    <>
      <button
        className="small"
        disabled={working || thread.status === "active"}
        title={thread.status === "active" ? "Stop the running turn first" : "Release the session and put the thread away"}
        onClick={() => void archive(false)}
      >
        {working ? "Archiving…" : "Archive"}
      </button>
      {refused && (
        <dialog ref={dialog} className="confirm" onClose={() => setRefused(null)}>
          <h2>Archive and discard the changes?</h2>
          <p>{refused}.</p>
          <p>Archiving removes the thread's working directory. Work that is not committed there is lost.</p>
          <div className="dialog-actions">
            <button onClick={() => setRefused(null)}>Keep the thread</button>
            <button className="danger" disabled={working} onClick={() => void archive(true)}>
              Archive anyway
            </button>
          </div>
        </dialog>
      )}
    </>
  );
}

export const threadControlsModule: UiModule = {
  id: "core.thread-controls",
  activate(ui) {
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
