import { describe, expect, it } from "vitest";

import type { Automation, AutomationEntry, AutomationRun } from "../api/protocol.gen";
import { automationName, describeSchedule, draftOf, runSummary, specOf, untilTime, withAutomation } from "./automations";

const automation: Automation = {
  id: "aut_1",
  name: "Nightly review",
  schedule: "0 2 * * *",
  prompt: "Review yesterday's commits",
  provider: "claude-code",
  cwd: "/src/app",
  environment: "git-worktree",
  model: "haiku",
  effort: null,
  permission_mode: "accept-edits",
  thread_mode: "continue",
  overlap: "allow",
  catch_up: true,
  enabled: false,
  created_at: 1,
  anchor: 1,
};

const entry = (id: string, name: string): AutomationEntry => ({ automation: { ...automation, id, name } });

describe("drafts", () => {
  it("come from an automation and go back to the same spec", () => {
    const result = specOf(draftOf(automation));
    expect(result).toEqual({
      spec: {
        name: "Nightly review",
        schedule: "0 2 * * *",
        prompt: "Review yesterday's commits",
        provider: "claude-code",
        cwd: "/src/app",
        environment: "git-worktree",
        model: "haiku",
        effort: null,
        permission_mode: "accept-edits",
        thread_mode: "continue",
        overlap: "allow",
        catch_up: true,
        enabled: false,
      },
    });
  });

  it("of a new automation start hourly, on, and say what is missing", () => {
    const draft = draftOf(undefined, { provider: "codex", cwd: "/src" });
    expect(draft).toMatchObject({ schedule: "every 1h", enabled: true, threadMode: "new", overlap: "skip", catchUp: false });
    expect(specOf(draft)).toEqual({ problem: "Give the automation a name" });
    expect(specOf({ ...draft, name: "x" })).toEqual({ problem: "Write the prompt each run sends" });
    expect(specOf({ ...draft, name: "x", prompt: "p", cwd: " " })).toEqual({
      problem: "Choose the directory the agent works in",
    });
    expect(specOf({ ...draft, name: " x ", prompt: "p" })).toHaveProperty("spec.name", "x");
  });
});

describe("schedules in words", () => {
  it("cover intervals, shorthands and daily times", () => {
    expect(describeSchedule("every 1h")).toBe("Every hour");
    expect(describeSchedule("every 1h30m")).toBe("Every hour 30 minutes");
    expect(describeSchedule("@every 2d")).toBe("Every 2 days");
    expect(describeSchedule("@daily")).toBe("Every day at midnight");
    expect(describeSchedule("0 9 * * 1-5")).toBe("Weekdays at 09:00");
    expect(describeSchedule("30 7 * * *")).toBe("Every day at 07:30");
  });

  it("leave the rest as written", () => {
    expect(describeSchedule("*/15 * * * *")).toBeNull();
    expect(describeSchedule("every 5w")).toBeNull();
    expect(describeSchedule("0 25 * * *")).toBeNull();
  });
});

describe("times and runs", () => {
  it("say how long until the next run", () => {
    expect(untilTime(1_000, 2_000)).toBe("due now");
    expect(untilTime(31_000, 1_000)).toBe("in 30s");
    expect(untilTime(5 * 60_000, 0)).toBe("in 5m");
    expect(untilTime(3 * 3_600_000, 0)).toBe("in 3h");
    expect(untilTime(3 * 86_400_000, 0)).toBe("in 3d");
  });

  it("sum a run up with its reason", () => {
    const run: AutomationRun = { automation_id: "aut_1", at: 1, trigger: "schedule", outcome: "started", thread_id: "thr_1" };
    expect(runSummary(run)).toBe("Scheduled · Started");
    expect(runSummary({ ...run, trigger: "missed", outcome: "skipped", reason: "the daemon was down" })).toBe(
      "Missed · Skipped: the daemon was down",
    );
  });
});

describe("the list", () => {
  it("replaces an automation in place, adds a new one at the end, and names them", () => {
    const list = [entry("a", "A"), entry("b", "B")];
    expect(withAutomation(list, entry("a", "A2")).map((e) => e.automation.name)).toEqual(["A2", "B"]);
    expect(withAutomation(list, entry("c", "C")).map((e) => e.automation.name)).toEqual(["A", "B", "C"]);
    expect(automationName(list, "b")).toBe("B");
    expect(automationName(list, "gone")).toBeNull();
  });
});
