import { describe, expect, it } from "vitest";

import { agentSummary, envOf, envRows, missingPrograms, withProgram, withProvider } from "./settings";

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

  it("sum up a folded agent card", () => {
    const label = (mode: string) => `mode ${mode}`;
    expect(agentSummary({}, label)).toBe("Agent's own defaults");
    expect(agentSummary({ hidden: true }, label)).toBe("Agent's own defaults");
    expect(agentSummary({ model: "opus", effort: "high", permission_mode: "auto", env: { A: "1", B: "2" } }, label)).toBe(
      "opus · high · mode auto · 2 variables",
    );
    expect(agentSummary({ env: { A: "1" } }, label)).toBe("1 variable");
  });

  it("name the programs that cannot run", () => {
    expect(
      missingPrograms([
        { name: "claude", plugins: [], path: "/bin/claude" },
        { name: "codex", plugins: [], path: null, error: "Not found" },
      ]),
    ).toEqual(["codex"]);
  });
});
