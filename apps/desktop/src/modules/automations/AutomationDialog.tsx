import { open } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";

import type { Automation, PermissionMode } from "../../api/protocol.gen";
import { type AutomationDraft, SCHEDULE_EXAMPLES, describeSchedule, draftOf, specOf } from "../../state/automations";
import { effortFor } from "../../state/models";
import { environmentsOf, findProvider, modeLabel, pickableProviders, providerName } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Choice, EffortPicker, ModelPicker } from "../../ui/kit";
import { tailPath } from "../../util/format";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

/** Make an automation (no `automation`), or change the one given. */
export function AutomationDialog({ close, automation }: { close: () => void; automation?: Automation }) {
  const plugins = useApp((state) => state.plugins);
  const allPlugins = useApp((state) => state.allPlugins);
  const projects = useApp((state) => state.projects);
  const settings = useApp((state) => state.settings?.settings);
  const providers = useMemo(() => pickableProviders(plugins, settings), [plugins, settings]);
  const environments = useMemo(() => environmentsOf(plugins), [plugins]);
  const [draft, setDraft] = useState<AutomationDraft>(() =>
    draftOf(automation, { provider: settings?.default_provider ?? providers[0]?.id, cwd: projects[0]?.folders[0] }),
  );
  const [saving, setSaving] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(null);
  const update = (patch: Partial<AutomationDraft>) => {
    setDraft((current) => ({ ...current, ...patch }));
    setRefusal(null);
  };

  // The automation's own agent stays on offer even when it is one the window hides
  // (an example plugin, a program not found now).
  const provider = findProvider(allPlugins, draft.provider);
  const providerOptions = providers.map((candidate) => ({ value: candidate.id, label: candidate.name }));
  if (draft.provider && !providers.some((candidate) => candidate.id === draft.provider)) {
    providerOptions.push({ value: draft.provider, label: providerName(allPlugins, draft.provider) });
  }
  const modelList = useApp((state) => (provider ? state.models[provider.id] : undefined));
  const modes = provider?.capabilities.permission_modes ?? [];
  const built = specOf(draft);
  const problem = "problem" in built ? built.problem : null;
  const described = describeSchedule(draft.schedule);
  const folders = projects.flatMap((project) =>
    project.folders.map((folder) => ({ value: folder, label: `${project.name} — ${tailPath(folder, 40)}` })),
  );

  const browse = async () => {
    const picked = await open({ directory: true, defaultPath: draft.cwd || undefined, title: "Working directory" });
    if (typeof picked === "string") update({ cwd: picked });
  };

  const save = async () => {
    if ("problem" in built) return;
    setSaving(true);
    const result = await store().saveAutomation(automation?.id ?? null, built.spec);
    setSaving(false);
    if ("error" in result) setRefusal(result.error);
    else close();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <DialogHeader>
            <DialogTitle>{automation ? `Automation ${automation.name}` : "New automation"}</DialogTitle>
            <DialogDescription>
              On its schedule, qcc sends the prompt to an agent, as if you had started the thread yourself.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup className="gap-4">
            <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-3">
              <Field>
                <FieldLabel htmlFor="automation-name">Name</FieldLabel>
                <Input
                  id="automation-name"
                  value={draft.name}
                  onChange={(event) => update({ name: event.target.value })}
                  placeholder="Nightly review"
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="automation-schedule">Schedule</FieldLabel>
                <Input
                  id="automation-schedule"
                  value={draft.schedule}
                  onChange={(event) => update({ schedule: event.target.value })}
                  placeholder="every 1h"
                  spellCheck={false}
                  className="font-mono"
                />
              </Field>
            </div>
            <div className="-mt-2 flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
              <span>{described ?? "A cron expression (minute hour day month weekday), local time."}</span>
              <span className="flex-1" />
              {SCHEDULE_EXAMPLES.map((example) => (
                <Button
                  key={example.schedule}
                  type="button"
                  variant="outline"
                  size="xs"
                  className="font-mono"
                  title={example.label}
                  onClick={() => update({ schedule: example.schedule })}
                >
                  {example.schedule}
                </Button>
              ))}
            </div>
            <Field>
              <FieldLabel htmlFor="automation-prompt">Prompt</FieldLabel>
              <Textarea
                id="automation-prompt"
                rows={4}
                value={draft.prompt}
                onChange={(event) => update({ prompt: event.target.value })}
                placeholder="What should the agent do each time?"
              />
            </Field>
            <Field>
              <FieldLabel>Agent</FieldLabel>
              <Choice
                aria-label="Agent"
                className="w-full"
                value={draft.provider || null}
                options={providerOptions}
                onChange={(value) => {
                  if ((value ?? "") === draft.provider) return;
                  // What the user set for that agent, if anything: a model means something only to its agent.
                  const defaults = value ? settings?.providers?.[value] : undefined;
                  update({
                    provider: value ?? "",
                    model: defaults?.model ?? null,
                    effort: defaults?.effort ?? null,
                    permissionMode: defaults?.permission_mode ?? null,
                  });
                }}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="automation-cwd">Directory</FieldLabel>
              <div className="flex gap-2">
                <Input
                  id="automation-cwd"
                  value={draft.cwd}
                  onChange={(event) => update({ cwd: event.target.value })}
                  placeholder="/path/to/project"
                  spellCheck={false}
                />
                {folders.length > 0 && (
                  <Choice
                    aria-label="A folder of a project"
                    className="w-56 shrink-0"
                    value={folders.some((folder) => folder.value === draft.cwd) ? draft.cwd : null}
                    options={[{ value: null, label: "Project folder…" }, ...folders]}
                    onChange={(value) => value && update({ cwd: value })}
                  />
                )}
                <Button type="button" variant="outline" onClick={() => void browse()}>
                  Browse…
                </Button>
              </div>
            </Field>
            {environments.length > 0 && (
              <Field orientation="horizontal" className="items-start justify-between gap-4">
                <div className="flex flex-col gap-0.5">
                  <FieldLabel htmlFor="automation-isolated">Separate git worktree</FieldLabel>
                  <p className="text-[13px] text-muted-foreground">
                    {draft.environment
                      ? "Each new thread works in a copy on a branch of its own; your checkout stays as it is."
                      : "Off: the agent works right in this directory."}
                  </p>
                </div>
                <Switch
                  id="automation-isolated"
                  checked={draft.environment !== null}
                  onCheckedChange={(checked) => update({ environment: checked ? (environments[0]?.id ?? null) : null })}
                />
              </Field>
            )}
            <div className="grid grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)_minmax(0,1fr)] gap-3">
              <Field>
                <FieldLabel htmlFor="automation-model">Model</FieldLabel>
                <ModelPicker
                  id="automation-model"
                  provider={provider?.id}
                  value={draft.model}
                  onChange={(model) => update({ model, effort: effortFor(modelList?.items ?? [], model, draft.effort) })}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="automation-effort">Effort</FieldLabel>
                <EffortPicker
                  id="automation-effort"
                  provider={provider?.id}
                  model={draft.model}
                  value={draft.effort}
                  onChange={(effort) => update({ effort })}
                />
              </Field>
              <Field>
                <FieldLabel>Permissions</FieldLabel>
                <Choice
                  aria-label="Permissions"
                  className="w-full"
                  value={draft.permissionMode && modes.includes(draft.permissionMode) ? draft.permissionMode : null}
                  options={[
                    { value: null, label: "Agent default" },
                    ...modes.map((mode) => ({ value: mode, label: modeLabel[mode] })),
                  ]}
                  disabled={modes.length === 0}
                  onChange={(value) => update({ permissionMode: value as PermissionMode | null })}
                />
              </Field>
            </div>
            <FieldDescription className="-mt-2">
              Nobody may be there to answer when it runs: a mode that asks for approval leaves the run waiting for you.
            </FieldDescription>
            <Field>
              <FieldLabel>Each run</FieldLabel>
              <ToggleGroup
                variant="outline"
                value={[draft.threadMode]}
                onValueChange={(value: string[]) => value[0] && update({ threadMode: value[0] as AutomationDraft["threadMode"] })}
              >
                <ToggleGroupItem value="new">Starts a new thread</ToggleGroupItem>
                <ToggleGroupItem value="continue">Continues one thread</ToggleGroupItem>
              </ToggleGroup>
              <FieldDescription>
                {draft.threadMode === "new"
                  ? "Runs do not see each other; each can be read and archived on its own."
                  : "The agent remembers earlier runs. A new thread starts when that one is archived."}
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel>While the previous run is still at work</FieldLabel>
              <ToggleGroup
                variant="outline"
                value={[draft.overlap]}
                onValueChange={(value: string[]) => value[0] && update({ overlap: value[0] as AutomationDraft["overlap"] })}
              >
                <ToggleGroupItem value="skip">Skip this run</ToggleGroupItem>
                <ToggleGroupItem value="allow">{draft.threadMode === "new" ? "Run alongside" : "Queue the message"}</ToggleGroupItem>
              </ToggleGroup>
            </Field>
            <Field orientation="horizontal">
              <Checkbox id="automation-catch-up" checked={draft.catchUp} onCheckedChange={(catchUp) => update({ catchUp })} />
              <FieldLabel htmlFor="automation-catch-up" className="font-normal">
                Make up for a run missed while qcc was not running (once, however many were missed)
              </FieldLabel>
            </Field>
            <Field orientation="horizontal">
              <Switch id="automation-enabled" checked={draft.enabled} onCheckedChange={(enabled) => update({ enabled })} />
              <FieldLabel htmlFor="automation-enabled" className="font-normal">
                {draft.enabled ? "On: runs on its schedule" : "Off: runs only when you run it"}
              </FieldLabel>
            </Field>
          </FieldGroup>
          {refusal && <p className="text-[13px] text-destructive">{refusal}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" disabled={saving || problem !== null} title={problem ?? undefined}>
              {saving ? "Saving…" : automation ? "Save" : "Create automation"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
