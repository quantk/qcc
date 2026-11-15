import { describe, expect, it } from "vitest";

import { modelLabel, modelOptions } from "./models";

const models = [
  { id: "opus", name: "Opus", is_default: true },
  { id: "haiku", name: "Haiku", description: "Fast" },
];

describe("model options", () => {
  it("starts with the agent's default and names it", () => {
    const options = modelOptions(models, null, "");
    expect(options.map((option) => option.value)).toEqual([null, "opus", "haiku"]);
    expect(options[0]?.label).toBe("Agent default (Opus)");
    expect(options[2]?.detail).toBe("haiku — Fast");
  });

  it("keeps a chosen model the agent does not list", () => {
    expect(modelOptions(models, "old-model", "").map((o) => o.value)).toContain("old-model");
  });

  it("offers what the user typed", () => {
    expect(modelOptions(models, null, " custom ").at(-1)).toMatchObject({ value: "custom" });
    expect(modelOptions(models, null, "haiku").filter((o) => o.value === "haiku")).toHaveLength(1);
    expect(modelOptions(models, null, "Haiku").filter((o) => o.label === "Haiku")).toHaveLength(1);
  });

  it("labels a model by its name", () => {
    expect(modelLabel(models, "haiku")).toBe("Haiku");
    expect(modelLabel(models, "x")).toBe("x");
    expect(modelLabel([], null)).toBe("Agent default");
  });
});
