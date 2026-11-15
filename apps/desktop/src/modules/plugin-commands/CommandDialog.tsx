import { useEffect, useRef, useState } from "react";

import type { CommandContribution, ThreadId } from "../../api/protocol.gen";
import { store, useApp } from "../../state/store";

/** Ask for a command's parameters, run it, and open the thread it made. */
export function CommandDialog({
  command,
  threadId,
  close,
}: {
  command: CommandContribution;
  threadId: ThreadId | null;
  close: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const thread = useApp((state) => (threadId ? state.threads[threadId] : undefined));
  const [values, setValues] = useState<Record<string, string>>({});
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => dialog.current?.showModal(), []);

  const missing = command.params.some((param) => param.required && !values[param.name]?.trim());

  const run = async () => {
    setRunning(true);
    setError(null);
    const args = Object.fromEntries(
      Object.entries(values)
        .map(([name, value]) => [name, value.trim()])
        .filter(([, value]) => value !== ""),
    );
    const result = await store().runCommand({
      plugin: command.plugin,
      command: command.id,
      thread_id: command.thread ? threadId : null,
      args,
    });
    setRunning(false);
    if ("error" in result) {
      setError(result.error);
      return;
    }
    close();
    store().notify(result.message);
    if (result.thread_id) store().select(result.thread_id);
  };

  return (
    <dialog ref={dialog} onClose={close}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void run();
        }}
      >
        <h2>{command.title}</h2>
        <p className="hint">{command.description}</p>
        {command.thread && <p>Thread: {thread?.title ?? threadId ?? "none selected"}</p>}
        {command.params.map((param, index) => (
          <label key={param.name}>
            {param.label}
            {param.required ? "" : " (optional)"}
            <input
              autoFocus={index === 0}
              value={values[param.name] ?? ""}
              placeholder={param.placeholder ?? ""}
              spellCheck={false}
              onChange={(event) => setValues({ ...values, [param.name]: event.target.value })}
            />
          </label>
        ))}
        {error && <p className="dialog-error">{error}</p>}
        <div className="dialog-actions">
          <button type="button" onClick={close}>
            Cancel
          </button>
          <button className="primary" disabled={running || missing || (command.thread && !threadId)}>
            {running ? "Running…" : "Run"}
          </button>
        </div>
        <p className="hint">
          From a terminal: <code>qcc {command.plugin} {command.id}</code>
        </p>
      </form>
    </dialog>
  );
}
