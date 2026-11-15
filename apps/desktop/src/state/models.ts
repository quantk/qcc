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

/** The model a choice stands for: the one named, or the agent's default. */
function chosenModel(models: readonly ModelInfo[], model: string | null): ModelInfo | undefined {
  return model ? models.find((candidate) => candidate.id === model) : models.find((candidate) => candidate.is_default);
}

/**
 * The effort levels a model takes, the agent's default first. Empty when effort
 * cannot be chosen for it. A level chosen earlier stays on the list, so that what
 * the thread has is always shown, even for a model the agent no longer lists.
 */
export function effortOptions(
  models: readonly ModelInfo[],
  model: string | null,
  current: string | null,
): ModelOption[] {
  const info = chosenModel(models, model);
  const levels = info?.efforts ?? [];
  if (levels.length === 0 && !current) return [];
  const fallback = info?.default_effort;
  const options: ModelOption[] = [{ value: null, label: fallback ? `Agent default (${fallback})` : "Agent default" }];
  for (const level of levels) options.push({ value: level, label: level });
  if (current && !levels.includes(current)) options.push({ value: current, label: current });
  return options;
}

/**
 * What becomes of a chosen effort when the model changes: kept if the new model
 * takes it (or the levels are not known yet), otherwise back to the default.
 */
export function effortFor(models: readonly ModelInfo[], model: string | null, effort: string | null): string | null {
  if (!effort) return null;
  const info = chosenModel(models, model);
  if (!info) return effort;
  return info.efforts?.includes(effort) ? effort : null;
}
