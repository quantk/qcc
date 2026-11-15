import { useEffect } from "react";

import type { ProviderId } from "../../api/protocol.gen";
import { effortOptions } from "../../state/models";
import { store, useApp } from "../../state/store";
import { Choice } from "./Choice";

/**
 * How hard the chosen model thinks, out of the levels its provider lists for it.
 * `null` leaves the choice to the agent. Disabled when the model takes no levels.
 */
export function EffortPicker(props: {
  provider: ProviderId | undefined;
  model: string | null;
  value: string | null;
  onChange: (value: string | null) => void;
  disabled?: boolean;
  id?: string;
  className?: string;
}) {
  const { provider } = props;
  const list = useApp((state) => (provider ? state.models[provider] : undefined));

  useEffect(() => {
    if (provider) void store().loadModels(provider);
  }, [provider]);

  const options = effortOptions(list?.models ?? [], props.model, props.value);
  const unavailable = options.length === 0;
  return (
    <Choice
      id={props.id}
      aria-label="Effort"
      className={props.className ?? "w-full"}
      value={unavailable ? null : props.value}
      options={unavailable ? [{ value: null, label: list?.loading ? "Asking the agent…" : "Not offered" }] : options}
      disabled={props.disabled || unavailable}
      title={unavailable ? "The agent does not let effort be chosen for this model" : undefined}
      onChange={props.onChange}
    />
  );
}
