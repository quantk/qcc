import { describe, expect, it } from "vitest";

import { effortFor, effortOptions, modelLabel, modelOptions } from "./models";

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

describe("effort options", () => {
  const withEfforts = [
    { id: "big", name: "Big", is_default: true, efforts: ["low", "high"], default_effort: "low" },
    { id: "small", name: "Small" },
  ];

  it("lists the levels of the chosen model, or of the default one", () => {
    expect(effortOptions(withEfforts, null, null).map((option) => option.value)).toEqual([null, "low", "high"]);
    expect(effortOptions(withEfforts, "big", null)[0]?.label).toBe("Agent default (low)");
    expect(effortOptions(withEfforts, "small", null)).toEqual([]);
  });

  it("keeps a level the model does not list", () => {
    expect(effortOptions(withEfforts, "small", "max").map((option) => option.value)).toEqual([null, "max"]);
  });

  it("drops a level the next model does not take", () => {
    expect(effortFor(withEfforts, "big", "high")).toBe("high");
    expect(effortFor(withEfforts, "small", "high")).toBeNull();
    expect(effortFor(withEfforts, "unknown", "high")).toBe("high");
  });
});
