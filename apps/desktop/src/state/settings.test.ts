import { describe, expect, it } from "vitest";

import { envOf, envRows, withProgram, withProvider } from "./settings";

describe("settings drafts", () => {
  it("drop an agent once nothing is set for it", () => {
    let settings = withProvider({}, "codex", { model: "gpt-x" });
    expect(settings.providers).toEqual({ codex: { model: "gpt-x" } });
    settings = withProvider(settings, "codex", { model: null });
    expect(settings.providers).toEqual({});
  });

  it("set and clear program paths", () => {
    const settings = withProgram({}, "claude", " /opt/claude ");
    expect(settings.programs).toEqual({ claude: "/opt/claude" });
    expect(withProgram(settings, "claude", "").programs).toEqual({});
  });

  it("turn environment rows into variables", () => {
    expect(envRows({ A: "1" })).toEqual([{ name: "A", value: "1" }]);
    expect(envOf([{ name: " A ", value: "1" }, { name: "", value: "lost" }])).toEqual({ A: "1" });
  });
});
