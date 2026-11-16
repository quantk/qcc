import { RefreshCwIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type { ProviderId } from "../../api/protocol.gen";
import { modelLabel, modelOptions } from "../../state/models";
import { store, useApp } from "../../state/store";
import { Button } from "@/components/ui/button";
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/ui/combobox";

const DEFAULT = "\u0000default";

/**
 * A model of one provider: what the agent says it offers, or any id typed in.
 * `null` leaves the choice to the agent.
 */
export function ModelPicker(props: {
  provider: ProviderId | undefined;
  value: string | null;
  onChange: (value: string | null) => void;
  disabled?: boolean;
  id?: string;
  className?: string;
}) {
  const { provider, value, onChange } = props;
  const list = useApp((state) => (provider ? state.models[provider] : undefined));
  const [typed, setTyped] = useState("");

  useEffect(() => {
    if (provider) void store().loadModels(provider);
  }, [provider]);

  const models = list?.items ?? [];
  const options = useMemo(() => modelOptions(models, value, typed), [models, value, typed]);
  // Items are keys, so they stay equal from one keystroke to the next. The agent's
  // default is an item to pick, but choosing it selects nothing: the input then
  // shows the default as its placeholder and stays free to type into.
  const byKey = useMemo(() => new Map(options.map((option) => [option.value ?? DEFAULT, option])), [options]);
  const keys = useMemo(() => [...byKey.keys()], [byKey]);
  const label = (key: string) => byKey.get(key)?.label ?? key;

  return (
    <div className={`flex gap-1 ${props.className ?? ""}`}>
      <Combobox<string>
        items={keys}
        value={value}
        disabled={props.disabled}
        itemToStringLabel={label}
        onInputValueChange={(text, details) => {
          // Typing filters; picking an item puts its label in the input, which is
          // not something the user typed.
          if (details.reason === "input-change") setTyped(text);
        }}
        onOpenChange={(open) => !open && setTyped("")}
        onValueChange={(key) => onChange(key && key !== DEFAULT ? key : null)}
      >
        <ComboboxInput
          id={props.id}
          className="w-full min-w-0"
          placeholder={list?.loading ? "Asking the agent…" : modelLabel(models, null)}
          disabled={props.disabled}
          spellCheck={false}
        />
        <ComboboxContent className="min-w-80">
          <ComboboxEmpty>{list?.error ? `Could not list models: ${list.error}` : "No such model."}</ComboboxEmpty>
          <ComboboxList>
            {(key: string) => {
              const option = byKey.get(key);
              return (
                <ComboboxItem key={key} value={key}>
                  <div className="flex min-w-0 flex-col">
                    <span className="truncate">{option?.label ?? key}</span>
                    {option?.detail && (
                      <span className="truncate text-xs text-muted-foreground">{option.detail}</span>
                    )}
                  </div>
                </ComboboxItem>
              );
            }}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label="Ask the agent for its models again"
        title={list?.error ? `Could not list models: ${list.error}` : "Ask the agent for its models again"}
        disabled={!provider || list?.loading || props.disabled}
        onClick={() => provider && void store().loadModels(provider, true)}
      >
        <RefreshCwIcon className={list?.loading ? "animate-spin" : undefined} />
      </Button>
    </div>
  );
}
