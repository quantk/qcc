// What a model picker offers: the agent's default, the models the provider listed,
// and whatever the user typed when it is not one of them.

import type { ModelInfo } from "../api/protocol.gen";

export type ModelOption = {
  /** `null`: no model is chosen and the agent uses its default. */
  value: string | null;
  label: string;
  detail?: string;
};

export function modelLabel(models: readonly ModelInfo[], id: string | null | undefined): string {
  if (!id) {
    const fallback = models.find((model) => model.is_default);
    return fallback ? `Agent default (${fallback.name})` : "Agent default";
  }
  return models.find((model) => model.id === id)?.name ?? id;
}

export function modelOptions(
  models: readonly ModelInfo[],
  current: string | null,
  typed: string,
): ModelOption[] {
  const options: ModelOption[] = [{ value: null, label: modelLabel(models, null) }];
  for (const model of models) {
    const detail = [model.name === model.id ? null : model.id, model.description].filter(Boolean).join(" — ");
    options.push({ value: model.id, label: model.name, detail: detail || undefined });
  }
  const known = (id: string) => options.some((option) => option.value === id);
  // A model chosen earlier stays visible even if the agent no longer lists it.
  if (current && !known(current)) options.push({ value: current, label: current });
  const text = typed.trim();
  if (text && !known(text) && !options.some((option) => option.label === text)) {
    options.push({ value: text, label: text, detail: "Use this model id as typed" });
  }
  return options;
}
