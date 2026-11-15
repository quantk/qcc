import { describe, expect, it } from "vitest";

import type { SkillStatus, Thread } from "../api/protocol.gen";
import { skillOff, threadSkills, visibleSkills, withSkillOff } from "./skills";

const skills: SkillStatus[] = [
  { name: "plan", description: "Planning", plugin: "orchestration", path: "/s/plan" },
  { name: "demo", description: "Demo", plugin: "provider-echo", on_request: true, path: "/s/demo" },
];

const thread = (patch: Partial<Thread> = {}) => ({ id: "t", ...patch }) as Thread;

describe("skills", () => {
  it("turns a skill off for all and back, keeping nothing for the default", () => {
    const off = withSkillOff({}, "plan", true);
    expect(skillOff(off, "plan")).toBe(true);
    expect(withSkillOff(off, "plan", false).skills).toEqual({});
  });

  it("says what a thread's sessions get", () => {
    const rows = (patch: Partial<Thread>, settings = {}) =>
      threadSkills(thread(patch), skills, settings).map((row) => [row.skill.name, row.on]);
    expect(rows({})).toEqual([
      ["plan", true],
      ["demo", false],
    ]);
    expect(rows({ skills_enabled: ["demo"], skills_disabled: ["plan"] })).toEqual([
      ["plan", false],
      ["demo", true],
    ]);
    expect(rows({ skills_enabled: ["demo"] }, withSkillOff({}, "demo", true))).toEqual([
      ["plan", true],
      ["demo", false],
    ]);
  });

  it("leaves out the skills of hidden plugins", () => {
    const plugins = [{ id: "orchestration" }] as never[];
    expect(visibleSkills(skills, plugins).map((skill) => skill.name)).toEqual(["plan"]);
  });
});
