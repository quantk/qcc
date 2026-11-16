// What agents spent and how their turns went, from the daemon's hourly buckets
// (`usage.history`). Pure folds: the view picks a range and a dimension, these add up.

import type { PluginInfo, Project, Thread, UsageBucket } from "../api/protocol.gen";
import { baseName } from "../util/format";
import { folderOf, providerName } from "./selectors";

/** Days back from today, today included; `all` starts at the first bucket. */
export type Range = 7 | 30 | 90 | "all";

export type Dimension = "project" | "agent" | "model";

export type Totals = {
  input: number;
  cached: number;
  output: number;
  turnsCompleted: number;
  turnsFailed: number;
  turnsInterrupted: number;
  workingMs: number;
};

export const emptyTotals = (): Totals => ({
  input: 0,
  cached: 0,
  output: 0,
  turnsCompleted: 0,
  turnsFailed: 0,
  turnsInterrupted: 0,
  workingMs: 0,
});

export function addBucket(totals: Totals, bucket: UsageBucket): void {
  totals.input += bucket.spent.input_tokens;
  totals.cached += bucket.spent.cached_input_tokens ?? 0;
  totals.output += bucket.spent.output_tokens;
  totals.turnsCompleted += bucket.turns_completed;
  totals.turnsFailed += bucket.turns_failed;
  totals.turnsInterrupted += bucket.turns_interrupted;
  totals.workingMs += bucket.working_ms;
}

/**
 * The measure the chart and the sorting use: input and output. Cached input is left
 * out because agents count it differently — part of the input for some, apart from it
 * for others — so adding it would count some agents' tokens twice.
 */
export const tokensOf = (totals: Totals): number => totals.input + totals.output;

export const turnsEnded = (totals: Totals): number =>
  totals.turnsCompleted + totals.turnsFailed + totals.turnsInterrupted;

/** Failed turns among those that ended; null when none did. */
export function failureRate(totals: Totals): number | null {
  const ended = turnsEnded(totals);
  return ended === 0 ? null : totals.turnsFailed / ended;
}

/** Mean length of an ended turn; null when none did. */
export function averageTurn(totals: Totals): number | null {
  const ended = turnsEnded(totals);
  return ended === 0 ? null : totals.workingMs / ended;
}

/** Local midnight of the day `ts` falls on: days are the user's, not UTC's. */
export function dayStart(ts: number): number {
  const date = new Date(ts);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** The next local midnight. Stepping by calendar day keeps DST days whole. */
function nextDay(day: number): number {
  const date = new Date(day);
  date.setDate(date.getDate() + 1);
  return date.getTime();
}

/** The first day `range` covers, as of `now`. */
export function rangeStart(range: Range, buckets: readonly UsageBucket[], now: number): number {
  if (range === "all") {
    const first = buckets.reduce((min, bucket) => Math.min(min, bucket.hour), Infinity);
    return dayStart(Number.isFinite(first) ? first : now);
  }
  const date = new Date(dayStart(now));
  date.setDate(date.getDate() - (range - 1));
  return date.getTime();
}

export function inRange(buckets: readonly UsageBucket[], range: Range, now: number): UsageBucket[] {
  const start = rangeStart(range, buckets, now);
  return buckets.filter((bucket) => bucket.hour >= start);
}

export type Group = { key: string; label: string };

export const UNKNOWN: Group = { key: "unknown", label: "Unknown (deleted)" };

type Known = {
  threads: Readonly<Record<string, Thread>>;
  projects: readonly Project[];
  plugins: readonly PluginInfo[];
};

/**
 * Which group a thread's spending belongs to. A thread is counted under what it is
 * now: an agent or model it switched away from gets its earlier hours too, because
 * the buckets do not say who spent them.
 */
export function groupOf(dimension: Dimension, known: Known): (threadId: string) => Group {
  return (threadId) => {
    const thread = known.threads[threadId];
    if (!thread) return UNKNOWN;
    switch (dimension) {
      case "project": {
        const project = thread.project ? known.projects.find((candidate) => candidate.id === thread.project) : undefined;
        if (project) return { key: `project:${project.id}`, label: project.name };
        const folder = folderOf(thread);
        return { key: `folder:${folder}`, label: baseName(folder) };
      }
      case "agent":
        return { key: thread.provider, label: providerName(known.plugins, thread.provider) };
      case "model":
        return thread.model ? { key: thread.model, label: thread.model } : { key: "default", label: "agent default" };
    }
  };
}

export type Row = Group & { totals: Totals };

/** Totals per group, most tokens first. */
export function byGroup(buckets: readonly UsageBucket[], keyOf: (threadId: string) => Group): Row[] {
  const rows = new Map<string, Row>();
  for (const bucket of buckets) {
    const group = keyOf(bucket.thread_id);
    let row = rows.get(group.key);
    if (!row) {
      row = { ...group, totals: emptyTotals() };
      rows.set(group.key, row);
    }
    addBucket(row.totals, bucket);
  }
  return [...rows.values()].sort((a, b) => tokensOf(b.totals) - tokensOf(a.totals) || a.label.localeCompare(b.label));
}

export type Day = { day: number; groups: ReadonlyMap<string, Totals>; totals: Totals };

/** Every day of the range, empty ones included, so the chart's gaps are real gaps. */
export function byDay(
  buckets: readonly UsageBucket[],
  keyOf: (threadId: string) => Group,
  range: Range,
  now: number,
): Day[] {
  const days = new Map<number, { groups: Map<string, Totals>; totals: Totals }>();
  const today = dayStart(now);
  for (let day = rangeStart(range, buckets, now); day <= today; day = nextDay(day)) {
    days.set(day, { groups: new Map(), totals: emptyTotals() });
  }
  for (const bucket of buckets) {
    const day = days.get(dayStart(bucket.hour));
    if (!day) continue;
    const key = keyOf(bucket.thread_id).key;
    let totals = day.groups.get(key);
    if (!totals) {
      totals = emptyTotals();
      day.groups.set(key, totals);
    }
    addBucket(totals, bucket);
    addBucket(day.totals, bucket);
  }
  return [...days].map(([day, value]) => ({ day, ...value }));
}

/** Groups that get a color of their own; the rest share "Other". */
export const SERIES = 4;
export const OTHER = "other";

/**
 * The color slot of each group. Slots go by spending over the whole history, not
 * the range shown, so changing the range never repaints a group.
 */
export function seriesSlots(all: readonly Row[]): ReadonlyMap<string, number> {
  return new Map(all.slice(0, SERIES).map((row, index) => [row.key, index]));
}

/** Tokens per thread in the buckets, most first. */
export function topThreads(buckets: readonly UsageBucket[], limit: number): { threadId: string; totals: Totals }[] {
  const threads = new Map<string, Totals>();
  for (const bucket of buckets) {
    let totals = threads.get(bucket.thread_id);
    if (!totals) {
      totals = emptyTotals();
      threads.set(bucket.thread_id, totals);
    }
    addBucket(totals, bucket);
  }
  return [...threads]
    .map(([threadId, totals]) => ({ threadId, totals }))
    .filter((entry) => tokensOf(entry.totals) > 0)
    .sort((a, b) => tokensOf(b.totals) - tokensOf(a.totals))
    .slice(0, limit);
}
