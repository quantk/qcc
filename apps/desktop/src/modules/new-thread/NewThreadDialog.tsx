import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";

import type { PermissionMode } from "../../api/protocol.gen";
import { environmentsOf, modeLabel, providersOf } from "../../state/selectors";
import { store, useApp } from "../../state/store";

const REMEMBERED = "qcc.new-thread";

type Remembered = { provider?: string; cwd?: string; environment?: string; mode?: string; model?: string };

function remembered(): Remembered {
  try {
    return JSON.parse(localStorage.getItem(REMEMBERED) ?? "{}") as Remembered;
  } catch {
    return {};
  }
}

export function NewThreadDialog({ close: onClose }: { close: () => void }) {
  const plugins = useApp((state) => state.plugins);
  const providers = useMemo(() => providersOf(plugins), [plugins]);
  const environments = useMemo(() => environmentsOf(plugins), [plugins]);
  const last = useMemo(remembered, []);
  const dialog = useRef<HTMLDialogElement>(null);

  const [providerId, setProviderId] = useState(last.provider ?? "");
  const [cwd, setCwd] = useState(last.cwd ?? "");
  const [environment, setEnvironment] = useState(last.environment ?? "");
  const [base, setBase] = useState("");
  const [model, setModel] = useState(last.model ?? "");
  const [mode, setMode] = useState(last.mode ?? "");
  const [prompt, setPrompt] = useState("");
  const [starting, setStarting] = useState(false);

  const provider = providers.find((candidate) => candidate.id === providerId) ?? providers[0];
  const modes = provider?.capabilities.permission_modes ?? [];
  const chosenMode = modes.includes(mode as PermissionMode) ? (mode as PermissionMode) : null;
  const chosenEnvironment = environments.some((candidate) => candidate.id === environment) ? environment : "";

  const directory = useRef<HTMLInputElement>(null);
  const message = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    dialog.current?.showModal();
    // Start where the first thing is missing: the directory, then the message.
    (last.cwd ? message : directory).current?.focus();
  }, [last]);

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: cwd || undefined, title: "Working directory" });
    if (typeof picked === "string") setCwd(picked);
  };

  const start = async () => {
    if (!provider || !cwd.trim()) return;
    setStarting(true);
    const thread = await store().spawn({
      provider: provider.id,
      cwd: cwd.trim(),
      environment: chosenEnvironment || null,
      base: chosenEnvironment && base.trim() ? base.trim() : null,
      model: model.trim() || null,
      permission_mode: chosenMode,
      prompt: prompt.trim() || null,
    });
    setStarting(false);
    if (!thread) return;
    const next: Remembered = { provider: provider.id, cwd: cwd.trim(), environment: chosenEnvironment, mode, model };
    localStorage.setItem(REMEMBERED, JSON.stringify(next));
    onClose();
  };

  return (
    <dialog ref={dialog} className="new-thread" onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <h2>New thread</h2>
        <label>
          Agent
          <select value={provider?.id ?? ""} onChange={(event) => setProviderId(event.target.value)}>
            {providers.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Directory
          <span className="row">
            <input ref={directory} value={cwd} onChange={(event) => setCwd(event.target.value)} placeholder="/path/to/project" spellCheck={false} />
            <button type="button" onClick={() => void browse()}>
              Browse…
            </button>
          </span>
        </label>
        <label>
          Workspace
          <select value={chosenEnvironment} onChange={(event) => setEnvironment(event.target.value)}>
            <option value="">Work in this directory</option>
            {environments.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.name} — a copy of its own
              </option>
            ))}
          </select>
        </label>
        {chosenEnvironment && (
          <label>
            Start from
            <input value={base} onChange={(event) => setBase(event.target.value)} placeholder="HEAD (a branch, tag or commit)" spellCheck={false} />
          </label>
        )}
        <div className="row two">
          <label>
            Model
            <input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Agent default" spellCheck={false} />
          </label>
          <label>
            Permissions
            <select value={chosenMode ?? ""} onChange={(event) => setMode(event.target.value)} disabled={modes.length === 0}>
              <option value="">Agent default</option>
              {modes.map((candidate) => (
                <option key={candidate} value={candidate}>
                  {modeLabel[candidate]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label>
          First message
          <textarea
            rows={4}
            ref={message}
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="What should the agent do? Leave empty to start with a blank thread."
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) void start();
            }}
          />
        </label>
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" disabled={starting || !provider || !cwd.trim()} title="Ctrl+Enter">
            {starting ? "Starting…" : "Start thread"}
          </button>
        </div>
      </form>
    </dialog>
  );
}
