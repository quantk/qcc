import { BotIcon } from "lucide-react";
import { useMemo, useState } from "react";

import type { Thread } from "../../api/protocol.gen";
import { modelLabel } from "../../state/models";
import { findProvider, pickableProviders } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { Choice, ModelPicker } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Popover, PopoverContent, PopoverDescription, PopoverHeader, PopoverTitle, PopoverTrigger } from "@/components/ui/popover";

/** Go on with another agent or model, between turns. */
export function AgentSwitch({ thread }: { thread: Thread }) {
  const plugins = useApp((state) => state.plugins);
  const allPlugins = useApp((state) => state.allPlugins);
  const settings = useApp((state) => state.settings?.settings);
  // The slice must be stable: select the list, not a fallback made on every read.
  const list = useApp((state) => state.models[thread.provider]);
  const models = list?.models ?? [];
  // The thread's own agent stays on the list even when its plugin is hidden.
  const providers = useMemo(() => {
    const shown = pickableProviders(plugins, settings);
    const own = findProvider(allPlugins, thread.provider);
    return own && !shown.some((candidate) => candidate.id === own.id) ? [own, ...shown] : shown;
  }, [plugins, allPlugins, settings, thread.provider]);
  const [open, setOpen] = useState(false);
  const [provider, setProvider] = useState(thread.provider);
  const [model, setModel] = useState<string | null>(thread.model ?? null);
  const [saving, setSaving] = useState(false);
  const active = thread.status === "active";
  const locked = active || thread.archived;
  const current = findProvider(allPlugins, thread.provider)?.name ?? thread.provider;
  const switching = provider !== thread.provider;
  const changed = switching || model !== (thread.model ?? null);

  const apply = async () => {
    setSaving(true);
    const done = await store().setAgent(thread.id, provider, model);
    setSaving(false);
    if (done) setOpen(false);
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        // Every opening starts from what the thread has now.
        if (next) {
          setProvider(thread.provider);
          setModel(thread.model ?? null);
        }
        setOpen(next);
      }}
    >
      <PopoverTrigger
        render={
          <Button
            variant="outline"
            size="sm"
            disabled={locked}
            title={
              thread.archived
                ? "The agent of this archived thread"
                : active
                  ? "Changes once the running turn ends"
                  : "Go on with another agent or model"
            }
          />
        }
      >
        <BotIcon />
        <span className="max-w-48 truncate">
          {current} · {thread.model ? modelLabel(models, thread.model) : "default model"}
        </span>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void apply();
          }}
        >
          <PopoverHeader>
            <PopoverTitle>Agent and model</PopoverTitle>
            <PopoverDescription>
              {switching
                ? "The new agent starts its own session in the same directory and is given the conversation so far."
                : "The session resumes with the new model on the next message."}
            </PopoverDescription>
          </PopoverHeader>
          <FieldGroup className="gap-3">
            <Field>
              <FieldLabel>Agent</FieldLabel>
              <Choice
                aria-label="Agent"
                className="w-full"
                value={provider}
                options={providers.map((candidate) => ({ value: candidate.id, label: candidate.name }))}
                onChange={(value) => {
                  if (!value || value === provider) return;
                  setProvider(value);
                  setModel(null);
                }}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`agent-model-${thread.id}`}>Model</FieldLabel>
              <ModelPicker id={`agent-model-${thread.id}`} provider={provider} value={model} onChange={setModel} />
            </Field>
          </FieldGroup>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" size="sm" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={!changed || saving || locked}>
              {saving ? "Switching…" : switching ? "Switch agent" : "Change model"}
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}
