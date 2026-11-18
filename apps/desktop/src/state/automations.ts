// Automations as the window shows and edits them: the draft a dialog holds, how a
// schedule and a run read in words. The daemon checks what it is given; this only
// says early what is plainly missing.

import type {
  Automation,
  AutomationEntry,
  AutomationOverlap,
  AutomationRun,
  AutomationSpec,
  AutomationThreadMode,
  PermissionMode,
} from "../api/protocol.gen";

/** What the create / edit dialog holds: text as typed, choices as picked. */
export type AutomationDraft = {
  name: string;
  schedule: string;
  prompt: string;
  provider: string;
  cwd: string;
  environment: string | null;
  model: string | null;
  effort: string | null;
  permissionMode: PermissionMode | null;
  threadMode: AutomationThreadMode;
  overlap: AutomationOverlap;
  catchUp: boolean;
  enabled: boolean;
};

/** Schedules the dialog offers as a start. */
export const SCHEDULE_EXAMPLES: readonly { schedule: string; label: string }[] = [
  { schedule: "every 1h", label: "Every hour" },
  { schedule: "0 9 * * 1-5", label: "Weekdays at 9:00" },
  { schedule: "@daily", label: "Every day at midnight" },
];

/** A draft of the automation given, or of a new one with these defaults. */
export function draftOf(automation: Automation | undefined, defaults: { provider?: string; cwd?: string } = {}): AutomationDraft {
  if (!automation) {
    return {
      name: "",
      schedule: "every 1h",
      prompt: "",
      provider: defaults.provider ?? "",
      cwd: defaults.cwd ?? "",
      environment: null,
      model: null,
      effort: null,
      permissionMode: null,
      threadMode: "new",
      overlap: "skip",
      catchUp: false,
      enabled: true,
    };
  }
  return {
    name: automation.name,
    schedule: automation.schedule,
    prompt: automation.prompt,
    provider: automation.provider,
    cwd: automation.cwd,
    environment: automation.environment ?? null,
    model: automation.model ?? null,
    effort: automation.effort ?? null,
    permissionMode: automation.permission_mode ?? null,
    threadMode: automation.thread_mode ?? "new",
    overlap: automation.overlap ?? "skip",
    catchUp: automation.catch_up ?? false,
    enabled: automation.enabled,
  };
}

/** The spec a draft stands for, or what keeps it from being one. */
export function specOf(draft: AutomationDraft): { spec: AutomationSpec } | { problem: string } {
  const name = draft.name.trim();
  const schedule = draft.schedule.trim();
  const cwd = draft.cwd.trim();
  if (!name) return { problem: "Give the automation a name" };
  if (!schedule) return { problem: "Say when it runs" };
  if (!draft.prompt.trim()) return { problem: "Write the prompt each run sends" };
  if (!draft.provider) return { problem: "Choose an agent" };
  if (!cwd) return { problem: "Choose the directory the agent works in" };
  return {
    spec: {
      name,
      schedule,
      prompt: draft.prompt,
      provider: draft.provider,
      cwd,
      environment: draft.environment,
      model: draft.model,
      effort: draft.effort,
      permission_mode: draft.permissionMode,
      thread_mode: draft.threadMode,
      overlap: draft.overlap,
      catch_up: draft.catchUp,
      enabled: draft.enabled,
    },
  };
}

const UNITS: Record<string, [string, string]> = {
  s: ["second", "seconds"],
  m: ["minute", "minutes"],
  h: ["hour", "hours"],
  d: ["day", "days"],
};

const SHORTHANDS: Record<string, string> = {
  "@hourly": "Every hour, on the hour",
  "@daily": "Every day at midnight",
  "@midnight": "Every day at midnight",
  "@weekly": "Every Sunday at midnight",
  "@monthly": "On the 1st of every month at midnight",
  "@yearly": "Every January 1st at midnight",
  "@annually": "Every January 1st at midnight",
};

/**
 * A schedule in words, for the shapes that say themselves plainly: intervals,
 * shorthands and a daily time. Null for anything else; the text is shown as written.
 */
export function describeSchedule(schedule: string): string | null {
  const text = schedule.trim().toLowerCase();
  if (SHORTHANDS[text]) return SHORTHANDS[text];
  const interval = /^@?every\s+(.+)$/.exec(text)?.[1]?.replace(/\s+/g, "");
  if (interval !== undefined) {
    if (!/^(\d+[smhd])+$/.test(interval)) return null;
    const words = [...interval.matchAll(/(\d+)([smhd])/g)].map(([, count = "", unit = "s"]) => {
      const [one, many] = UNITS[unit] ?? ["", ""];
      return Number(count) === 1 ? one : `${Number(count)} ${many}`;
    });
    return `Every ${words.join(" ")}`;
  }
  const daily = /^(\d{1,2})\s+(\d{1,2})\s+\*\s+\*\s+(\*|1-5)$/.exec(text);
  if (daily) {
    const [, minute = "", hour = "", days] = daily;
    if (Number(minute) > 59 || Number(hour) > 23) return null;
    const at = `${hour.padStart(2, "0")}:${minute.padStart(2, "0")}`;
    return days === "*" ? `Every day at ${at}` : `Weekdays at ${at}`;
  }
  return null;
}

/** How long until a time, in words: "in 5m", "in 3h", or "due" once it has come. */
export function untilTime(ts: number, now: number): string {
  const seconds = Math.round((ts - now) / 1000);
  if (seconds <= 0) return "due now";
  if (seconds < 60) return `in ${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `in ${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `in ${hours}h`;
  return `in ${Math.round(hours / 24)}d`;
}

/** A time as a short local date and time, for tooltips and run lists. */
export function clockTime(ts: number): string {
  return new Date(ts).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export const triggerLabel: Record<AutomationRun["trigger"], string> = {
  schedule: "Scheduled",
  missed: "Missed",
  manual: "Run by hand",
};

export const outcomeLabel: Record<AutomationRun["outcome"], string> = {
  started: "Started",
  skipped: "Skipped",
  failed: "Failed",
};

/** What came of a run, in a line. */
export function runSummary(run: AutomationRun): string {
  const what = run.outcome === "started" ? outcomeLabel.started : `${outcomeLabel[run.outcome]}${run.reason ? `: ${run.reason}` : ""}`;
  return `${triggerLabel[run.trigger]} · ${what}`;
}

/** The name of the automation behind a thread, or null when it was deleted. */
export function automationName(automations: readonly AutomationEntry[], id: string): string | null {
  return automations.find((entry) => entry.automation.id === id)?.automation.name ?? null;
}

/** The list with one automation put in place of its older self, or added at the end. */
export function withAutomation(automations: readonly AutomationEntry[], entry: AutomationEntry): AutomationEntry[] {
  const index = automations.findIndex((known) => known.automation.id === entry.automation.id);
  if (index < 0) return [...automations, entry];
  return automations.map((known, at) => (at === index ? entry : known));
}
