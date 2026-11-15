import { describe, expect, it } from "vitest";

import type { PluginInfo } from "../api/protocol.gen";
import { effectiveSetting, pluginSetting, withPluginSetting } from "./pluginSettings";

const plugin = {
  id: "assistant",
  settings: [{ id: "model", label: "Model", kind: { type: "text" }, default: "small" }],
} as unknown as PluginInfo;

describe("plugin settings", () => {
  it("falls back to the default and drops what goes back to it", () => {
    expect(effectiveSetting({}, plugin, "model")).toBe("small");
    const set = withPluginSetting({}, "assistant", "model", "big");
    expect(pluginSetting(set, "assistant", "model")).toBe("big");
    expect(effectiveSetting(set, plugin, "model")).toBe("big");
    expect(withPluginSetting(set, "assistant", "model", "").plugins).toEqual({});
  });
});
