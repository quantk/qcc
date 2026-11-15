import { open } from "@tauri-apps/plugin-dialog";
import { useMemo, useRef, useState } from "react";

import type { PermissionMode, ProjectId } from "../../api/protocol.gen";
import { environmentsOf, modeLabel, providersOf } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Choice, ModelPicker } from "../../ui/kit";
import { tailPath } from "../../util/format";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";

const REMEMBERED = "qcc.new-thread";
const OTHER = "\u0000other";

type Remembered = {
  provider?: string;
  cwd?: string;
  environment?: string;
  mode?: string;
  model?: string;
  project?: string;
};

function remembered(): Remembered {
  try {
    return JSON.parse(localStorage.getItem(REMEMBERED) ?? "{}") as Remembered;
  } catch {
    return {};
  }
}

export function NewThreadDialog({ close: onClose, project: preset }: { close: () => void; project?: ProjectId }) {
  const plugins = useApp((state) => state.plugins);
  const projects = useApp((state) => state.projects);
  const providers = useMemo(() => providersOf(plugins), [plugins]);
  const environments = useMemo(() => environmentsOf(plugins), [plugins]);
  const last = useMemo(remembered, []);

  const [providerId, setProviderId] = useState(last.provider ?? "");
  const [projectId, setProjectId] = useState<string | null>(preset ?? last.project ?? null);
  const project = projects.find((candidate) => candidate.id === projectId);
  // In a project, the directory is one of its folders unless asked otherwise.
  const [otherDir, setOtherDir] = useState(false);
  const [cwd, setCwd] = useState(() => {
    const first = projects.find((candidate) => candidate.id === (preset ?? last.project));
    if (first && !first.folders.includes(last.cwd ?? "")) return first.folders[0] ?? "";
    return last.cwd ?? "";
  });
  const [environment, setEnvironment] = useState(last.environment ?? "");
  const [base, setBase] = useState("");
  // A model means something only to the agent it was chosen for.
  const [model, setModel] = useState<string | null>(last.model || null);
  const [mode, setMode] = useState(last.mode ?? "");
  const [prompt, setPrompt] = useState("");
  const [starting, setStarting] = useState(false);

  const provider = providers.find((candidate) => candidate.id === providerId) ?? providers[0];
  const modes = provider?.capabilities.permission_modes ?? [];
  const chosenMode = modes.includes(mode as PermissionMode) ? (mode as PermissionMode) : null;
  const chosenEnvironment = environments.some((candidate) => candidate.id === environment) ? environment : "";

  // Focus starts where the first thing is missing: the directory, then the message.
  const directory = useRef<HTMLInputElement>(null);
  const message = useRef<HTMLTextAreaElement>(null);

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
      project: project?.id ?? null,
      environment: chosenEnvironment || null,
      base: chosenEnvironment && base.trim() ? base.trim() : null,
      model,
      permission_mode: chosenMode,
      prompt: prompt.trim() || null,
    });
    setStarting(false);
    if (!thread) return;
    const next: Remembered = {
      provider: provider.id,
      cwd: cwd.trim(),
      environment: chosenEnvironment,
      mode,
      model: model ?? undefined,
      project: project?.id,
    };
    localStorage.setItem(REMEMBERED, JSON.stringify(next));
    onClose();
  };

  const providerOptions = providers.map((candidate) => ({ value: candidate.id, label: candidate.name }));
  const environmentOptions = [
    { value: null, label: "Work in this directory" },
    ...environments.map((candidate) => ({ value: candidate.id, label: `${candidate.name} — a copy of its own` })),
  ];
  const modeOptions = [{ value: null, label: "Agent default" }, ...modes.map((candidate) => ({ value: candidate, label: modeLabel[candidate] }))];

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg" initialFocus={last.cwd ? message : directory}>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void start();
          }}
        >
          <DialogHeader>
            <DialogTitle>New thread</DialogTitle>
          </DialogHeader>
          <FieldGroup className="gap-4">
            <Field>
              <FieldLabel>Agent</FieldLabel>
              <Choice
                aria-label="Agent"
                className="w-full"
                value={provider?.id ?? null}
                options={providerOptions}
                onChange={(value) => {
                  if ((value ?? "") !== provider?.id) setModel(null);
                  setProviderId(value ?? "");
                }}
              />
            </Field>
            {projects.length > 0 && (
              <Field>
                <FieldLabel>Project</FieldLabel>
                <Choice
                  aria-label="Project"
                  className="w-full"
                  value={project?.id ?? null}
                  options={[
                    { value: null, label: "No project" },
                    ...projects.map((candidate) => ({ value: candidate.id, label: candidate.name })),
                  ]}
                  onChange={(value) => {
                    setProjectId(value);
                    setOtherDir(false);
                    const next = projects.find((candidate) => candidate.id === value);
                    if (next && !next.folders.includes(cwd.trim())) setCwd(next.folders[0] ?? "");
                  }}
                />
              </Field>
            )}
            {project && (
              <Field>
                <FieldLabel>Folder</FieldLabel>
                <Choice
                  aria-label="Folder"
                  className="w-full"
                  value={otherDir ? OTHER : cwd.trim()}
                  options={[
                    ...project.folders.map((folder) => ({ value: folder, label: tailPath(folder, 56) })),
                    { value: OTHER, label: "Another directory…" },
                  ]}
                  onChange={(value) => {
                    if (value === OTHER) return setOtherDir(true);
                    setOtherDir(false);
                    if (value) setCwd(value);
                  }}
                />
              </Field>
            )}
            {(!project || otherDir) && (
            <Field>
              <FieldLabel htmlFor="new-thread-cwd">Directory</FieldLabel>
              <div className="flex gap-2">
                <Input
                  id="new-thread-cwd"
                  ref={directory}
                  value={cwd}
                  onChange={(event) => setCwd(event.target.value)}
                  placeholder="/path/to/project"
                  spellCheck={false}
                />
                <Button type="button" variant="outline" onClick={() => void browse()}>
                  Browse…
                </Button>
              </div>
            </Field>
            )}
            <Field>
              <FieldLabel>Workspace</FieldLabel>
              <Choice
                aria-label="Workspace"
                className="w-full"
                value={chosenEnvironment || null}
                options={environmentOptions}
                onChange={(value) => setEnvironment(value ?? "")}
              />
            </Field>
            {chosenEnvironment && (
              <Field>
                <FieldLabel htmlFor="new-thread-base">Start from</FieldLabel>
                <Input
                  id="new-thread-base"
                  value={base}
                  onChange={(event) => setBase(event.target.value)}
                  placeholder="HEAD (a branch, tag or commit)"
                  spellCheck={false}
                />
              </Field>
            )}
            <div className="grid grid-cols-2 gap-3">
              <Field>
                <FieldLabel htmlFor="new-thread-model">Model</FieldLabel>
                <ModelPicker id="new-thread-model" provider={provider?.id} value={model} onChange={setModel} />
              </Field>
              <Field>
                <FieldLabel>Permissions</FieldLabel>
                <Choice
                  aria-label="Permissions"
                  className="w-full"
                  value={chosenMode}
                  options={modeOptions}
                  disabled={modes.length === 0}
                  onChange={(value) => setMode(value ?? "")}
                />
              </Field>
            </div>
            <Field>
              <FieldLabel htmlFor="new-thread-prompt">First message</FieldLabel>
              <Textarea
                id="new-thread-prompt"
                rows={4}
                ref={message}
                value={prompt}
                onChange={(event) => setPrompt(event.target.value)}
                placeholder="What should the agent do? Leave empty to start with a blank thread."
                onKeyDown={(event) => {
                  if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) void start();
                }}
              />
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={starting || !provider || !cwd.trim()} title="Ctrl+Enter">
              {starting ? "Starting…" : "Start thread"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
