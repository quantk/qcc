// What the live wall shows: which agents, in which order, and the last few things
// each of them did, in a line or a few.

import type { Thread } from "../api/protocol.gen";
import { attentionOf } from "./selectors";
import type { Entry } from "./timeline";

/** How long a finished agent stays on the wall, so a turn that just ended can be read. */
export const RECENT_MS = 10 * 60_000;

/** At most this many tiles: each one follows its thread's events. */
export const WALL_MAX = 24;

/**
 * How much each tile shows. Compact tiles are short and narrow, so a screen holds a
 * dozen agents or more; detailed ones show more of each agent's latest steps.
 */
export type Density = "compact" | "detailed";

export type DensityLayout = {
  /** Steps shown under each agent. */
  steps: number;
  /** How much of each step's output is kept. */
  detail: DetailSize;
};

export const DENSITY: Record<Density, DensityLayout> = {
  // A narrow tile fits about 45 characters to a line: two of them hold the end of an answer.
  compact: { steps: 5, detail: { lines: 1, chars: 80 } },
  detailed: { steps: 10, detail: { lines: 3, chars: 240 } },
};

export const DENSITY_KEY = "qcc.live-density";

/** The density saved in storage; compact when nothing (or nothing known) is saved. */
export function parseDensity(saved: string | null): Density {
  return saved === "detailed" ? "detailed" : "compact";
}

const rank = { waiting: 0, failed: 1, active: 2, idle: 3, archived: 4 } as const;

/**
 * The agents worth watching now: waiting for an answer, working, or done or failed
 * within `RECENT_MS`. Those that want the user come first; then the ones working,
 * longest-running first; then the ones that finished, latest first.
 */
export function wallThreads(threads: readonly Thread[], now: number): Thread[] {
  return threads
    .filter((thread) => {
      if (thread.archived || thread.hidden) return false;
      const attention = attentionOf(thread);
      if (attention === "waiting" || attention === "active") return true;
      return now - (thread.last_turn_at ?? thread.updated_at) <= RECENT_MS;
    })
    .sort((a, b) => {
      const byRank = rank[attentionOf(a)] - rank[attentionOf(b)];
      if (byRank !== 0) return byRank;
      if (attentionOf(a) === "active") return (a.turn_started_at ?? 0) - (b.turn_started_at ?? 0);
      return (b.last_turn_at ?? b.updated_at) - (a.last_turn_at ?? a.updated_at);
    })
    .slice(0, WALL_MAX);
}

/** One step of an agent in a line, with a little of what it produced. */
export type TailLine = {
  key: string;
  kind: "message" | "thought" | "command" | "edit" | "read" | "search" | "tool" | "plan" | "notice" | "you";
  text: string;
  /** The last lines of what it printed or wrote. */
  detail?: string;
  pending: boolean;
  failed: boolean;
};

/**
 * How much output is kept under a step: its last `lines` lines, and at most `chars`
 * characters of them, since an answer streams as one long line and its end is what
 * matters.
 */
export type DetailSize = { lines: number; chars: number };

function lastLines(text: string | null | undefined, size: DetailSize): string | undefined {
  const lines = (text ?? "").trimEnd().split("\n");
  let kept = lines.slice(-size.lines).join("\n");
  if (kept.length > size.chars) kept = `…${kept.slice(-size.chars).trimStart()}`;
  return kept.trim() ? kept : undefined;
}

function firstLine(text: string): string {
  return text.trim().split("\n", 1)[0] ?? "";
}

function lineOf(entry: Entry, detail: DetailSize): TailLine | null {
  switch (entry.kind) {
    case "user":
      return { key: entry.key, kind: "you", text: firstLine(entry.text), pending: entry.queued, failed: false };
    case "interaction":
      // An open question is answered on the tile itself, not read in its tail.
      return null;
    case "notice":
      return entry.level === "info"
        ? null
        : { key: entry.key, kind: "notice", text: firstLine(entry.text), pending: false, failed: entry.level === "error" };
    case "conversation":
      return null;
    case "item": {
      const { item } = entry;
      const pending = item.status === "pending";
      const failed = item.status === "failed" || item.status === "declined";
      const base = { key: entry.key, pending, failed };
      switch (item.type) {
        case "agentMessage":
          // The answer as it streams: its end is what is being written now.
          return { ...base, kind: "message", text: "", detail: lastLines(item.text, detail) };
        case "reasoning":
          return pending ? { ...base, kind: "thought", text: "Thinking…" } : null;
        case "plan":
          return { ...base, kind: "plan", text: firstLine(item.text) };
        case "command":
          return {
            ...base,
            failed: failed || (item.exit_code != null && item.exit_code !== 0),
            kind: "command",
            text: item.command,
            detail: lastLines(item.output, detail),
          };
        case "fileChange":
          return { ...base, kind: "edit", text: item.path };
        case "fileRead":
          return { ...base, kind: "read", text: item.path };
        case "search":
          return { ...base, kind: "search", text: item.query };
        case "tool":
          return { ...base, kind: "tool", text: item.name, detail: lastLines(item.output, { ...detail, lines: Math.min(2, detail.lines) }) };
        case "extension":
          return { ...base, kind: "tool", text: item.kind };
      }
    }
  }
}

/** The last `count` steps of a timeline, oldest first, each with a little of its output. */
export function tailOf(entries: readonly Entry[], count: number, detail: DetailSize = DENSITY.detailed.detail): TailLine[] {
  const lines: TailLine[] = [];
  for (let index = entries.length - 1; index >= 0 && lines.length < count; index -= 1) {
    const line = lineOf(entries[index]!, detail);
    if (line) lines.push(line);
  }
  return lines.reverse();
}

/** How full the context is, from 0 to 1, or null when the agent has not said. */
export function contextFill(thread: Thread): number | null {
  const used = thread.usage?.context_used;
  const window = thread.usage?.context_window;
  if (used == null || !window) return null;
  return Math.min(1, used / window);
}

/** What the live wall's button counts: agents at work and those waiting for the user. */
export type LiveCount = { working: number; waiting: number };

/** Threads the user can see that are working or waiting for an answer, as the wall's button counts them. */
export function liveCount(threads: readonly Thread[]): LiveCount {
  let working = 0;
  let waiting = 0;
  for (const thread of threads) {
    if (thread.hidden) continue;
    const attention = attentionOf(thread);
    if (attention === "waiting") waiting += 1;
    else if (attention === "active") working += 1;
  }
  return { working, waiting };
}
