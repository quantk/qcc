// Numbers about a thread: turns, tokens, what the agent did. Folded from the same
// events as the timeline and kept beside it, so a panel never re-reads the journal.

import type {
  Compaction,
  ThreadEvent,
  TokenUsage,
  TurnId,
  TurnStatus,
  UsageSnapshot,
} from "../api/protocol.gen";
import type { Entry } from "./timeline";

export type TurnRecord = { id: TurnId; startedAt: number; endedAt?: number; status?: TurnStatus };

/** One usage report of the current conversation. */
export type UsagePoint = { ts: number; usage: UsageSnapshot };

export type ThreadStats = {
  turns: readonly TurnRecord[];
  /** Reports of the current conversation, oldest first. */
  usage: readonly UsagePoint[];
  /** Compactions of the current conversation, oldest first. */
  compactions: readonly { ts: number; compaction: Compaction }[];
  /**
   * Totals of sessions that are over. A provider counts its total per session, so
   * the thread's total is these plus the current session's.
   */
  banked: TokenUsage;
  /** The total the current session last reported. */
  current: TokenUsage;
};

const zero: TokenUsage = { input_tokens: 0, output_tokens: 0, cached_input_tokens: null };

export const emptyStats: ThreadStats = { turns: [], usage: [], compactions: [], banked: zero, current: zero };

export function addUsage(a: TokenUsage, b: TokenUsage): TokenUsage {
  const cached = a.cached_input_tokens == null && b.cached_input_tokens == null
    ? null
    : (a.cached_input_tokens ?? 0) + (b.cached_input_tokens ?? 0);
  return {
    input_tokens: a.input_tokens + b.input_tokens,
    output_tokens: a.output_tokens + b.output_tokens,
    cached_input_tokens: cached,
  };
}

/** Whether `next` cannot be a later count of the same session as `previous`. */
function restarted(previous: TokenUsage, next: TokenUsage): boolean {
  return (
    next.input_tokens < previous.input_tokens ||
    next.output_tokens < previous.output_tokens ||
    (next.cached_input_tokens ?? 0) < (previous.cached_input_tokens ?? 0)
  );
}

/** Tokens the context held in the last request: fresh input and what came from the cache. */
export function contextUsed(usage: UsageSnapshot): number {
  return usage.last.input_tokens + (usage.last.cached_input_tokens ?? 0);
}

/** Fold one event in. The timeline decides which events are new, so this does not. */
export function applyStats(stats: ThreadStats, event: ThreadEvent): ThreadStats {
  switch (event.type) {
    case "turn/started":
      return { ...stats, turns: [...stats.turns, { id: event.turn_id, startedAt: event.ts }] };
    case "turn/completed": {
      const index = stats.turns.findLastIndex((turn) => turn.id === event.turn_id);
      if (index < 0) return stats;
      const turns = stats.turns.slice();
      turns[index] = { ...turns[index]!, endedAt: event.ts, status: event.status };
      return { ...stats, turns };
    }
    case "usage/updated": {
      const { total } = event.usage;
      // A session that ended without a word and a new one that counts from zero
      // look the same as a count going down; providers that carry their count over
      // a resume never go down, so nothing is counted twice.
      const banked = restarted(stats.current, total) ? addUsage(stats.banked, stats.current) : stats.banked;
      return { ...stats, banked, current: total, usage: [...stats.usage, { ts: event.ts, usage: event.usage }] };
    }
    case "session/compacted":
      return { ...stats, compactions: [...stats.compactions, { ts: event.ts, compaction: event.compaction }] };
    case "thread/reset":
      // The next session starts its count afresh; what this one spent stays counted.
      return { ...stats, usage: [], compactions: [], banked: addUsage(stats.banked, stats.current), current: zero };
    default:
      return stats;
  }
}

/** Everything the thread's sessions spent. */
export function threadTotal(stats: ThreadStats): TokenUsage {
  return addUsage(stats.banked, stats.current);
}

export type TurnSummary = {
  count: number;
  completed: number;
  failed: number;
  interrupted: number;
  /** Time spent in turns that ended, in milliseconds. */
  working: number;
  longest: number;
  /** When the running turn started, if one runs. */
  running: number | null;
};

export function turnSummary(turns: readonly TurnRecord[]): TurnSummary {
  const summary: TurnSummary = {
    count: turns.length,
    completed: 0,
    failed: 0,
    interrupted: 0,
    working: 0,
    longest: 0,
    running: null,
  };
  for (const turn of turns) {
    if (turn.endedAt === undefined) continue;
    const took = Math.max(0, turn.endedAt - turn.startedAt);
    summary.working += took;
    summary.longest = Math.max(summary.longest, took);
    if (turn.status) summary[turn.status] += 1;
  }
  // Only the last turn can still be running; an earlier one without an end was
  // cut off by a session that went away.
  const last = turns.at(-1);
  if (last && last.endedAt === undefined) summary.running = last.startedAt;
  return summary;
}

export type Activity = {
  userMessages: number;
  agentMessages: number;
  commands: number;
  failedCommands: number;
  /** Distinct paths, in the order first touched. */
  filesChanged: readonly string[];
  filesRead: readonly string[];
  searches: number;
  /** Tool calls by name, most used first. */
  tools: readonly { name: string; count: number }[];
  questions: number;
  unanswered: number;
  errors: number;
};

/** What the agent did, counted from the timeline. */
export function activityOf(entries: readonly Entry[]): Activity {
  let userMessages = 0;
  let agentMessages = 0;
  let commands = 0;
  let failedCommands = 0;
  let searches = 0;
  let questions = 0;
  let unanswered = 0;
  let errors = 0;
  const changed = new Set<string>();
  const read = new Set<string>();
  const tools = new Map<string, number>();
  for (const entry of entries) {
    switch (entry.kind) {
      case "user":
        if (!entry.queued) userMessages += 1;
        break;
      case "interaction":
        questions += 1;
        if (!entry.resolution || entry.resolution.type === "cancelled") unanswered += 1;
        break;
      case "notice":
        if (entry.level === "error") errors += 1;
        break;
      case "item": {
        const { item } = entry;
        switch (item.type) {
          case "agentMessage":
            agentMessages += 1;
            break;
          case "command":
            commands += 1;
            if (item.status === "failed" || (item.exit_code != null && item.exit_code !== 0)) failedCommands += 1;
            break;
          case "fileChange":
            changed.add(item.path);
            break;
          case "fileRead":
            read.add(item.path);
            break;
          case "search":
            searches += 1;
            break;
          case "tool":
            tools.set(item.name, (tools.get(item.name) ?? 0) + 1);
            break;
          default:
            break;
        }
        break;
      }
      case "conversation":
        break;
    }
  }
  return {
    userMessages,
    agentMessages,
    commands,
    failedCommands,
    filesChanged: [...changed],
    filesRead: [...read],
    searches,
    tools: [...tools]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)),
    questions,
    unanswered,
    errors,
  };
}
