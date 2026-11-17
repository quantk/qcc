// What a thread's context window holds, ready to draw: the agent's own breakdown
// when it gives one, else a reckoning from its usage reports, marked as such.

import type { ContextCategory, ContextEntry, ContextKind, ContextUsage, Thread } from "../api/protocol.gen";
import { type ThreadStats, type UsagePoint, contextUsed } from "./stats";

/**
 * The color groups of the parts. A part keeps its group whatever else the window
 * holds, so a color always means the same thing; parts outside them are "other".
 */
export type Group = "messages" | "system" | "mcp" | "memory" | "other";

export const GROUPS: readonly { group: Exclude<Group, "other">; label: string }[] = [
  { group: "messages", label: "Conversation" },
  { group: "system", label: "System prompt and tools" },
  { group: "mcp", label: "MCP" },
  { group: "memory", label: "Memory, skills, agents" },
];

const GROUP_OF: Record<string, Group> = {
  messages: "messages",
  system_prompt: "system",
  system_tools: "system",
  mcp_tools: "mcp",
  mcp_instructions: "mcp",
  memory_files: "memory",
  skills: "memory",
  custom_agents: "memory",
  // The reckoned start of a conversation is mostly the system prompt and tools.
  start: "system",
};

export const groupOf = (id: string): Group => GROUP_OF[id] ?? "other";

export type Part = ContextCategory & {
  group: Group;
  /** What it is made of, largest first. */
  entries: readonly ContextEntry[];
};

export type ContextView = {
  used: number;
  window: number | null;
  /** Reckoned from usage reports rather than said by the agent. */
  estimated: boolean;
  /** In the window, in the agent's order. */
  usedParts: readonly Part[];
  /** Kept back for compacting. */
  buffer: number;
  /** Free, as the agent says or as the window less the rest. */
  free: number | null;
  /** Known to the agent, loaded only when needed: not in the window. */
  deferred: readonly Part[];
};

function view(context: ContextUsage): ContextView {
  const entries = context.entries ?? [];
  const parts = context.categories.map(
    (category): Part => ({
      ...category,
      group: groupOf(category.id),
      entries: entries.filter((entry) => entry.category === category.id),
    }),
  );
  const sum = (kind: ContextKind) =>
    parts.filter((part) => part.kind === kind).reduce((total, part) => total + part.tokens, 0);
  const window = context.window ?? null;
  const buffer = sum("buffer");
  const said = parts.some((part) => part.kind === "free");
  return {
    used: context.used,
    window,
    estimated: context.estimated ?? false,
    usedParts: parts.filter((part) => part.kind === "used" && part.tokens > 0),
    buffer,
    free: said ? sum("free") : window === null ? null : Math.max(0, window - context.used - buffer),
    deferred: parts.filter((part) => part.kind === "deferred" && part.tokens > 0),
  };
}

/**
 * Without the agent's word, the first request of a conversation is taken as what
 * it starts with — system prompt, tools, the first message — and the rest as the
 * conversation since. The smallest request stands in for the first after a
 * compaction, which can empty the context below where it started.
 */
export function estimate(points: readonly UsagePoint[], window: number | null): ContextUsage | null {
  const last = points.at(-1);
  if (!last) return null;
  const used = contextUsed(last.usage);
  const start = Math.min(...points.map((point) => contextUsed(point.usage)));
  const categories: ContextCategory[] = [
    { id: "start", name: "Start: system, tools, first message", tokens: start, kind: "used" },
  ];
  if (used > start) categories.push({ id: "messages", name: "Conversation since", tokens: used - start, kind: "used" });
  return { used, window: window ?? last.usage.model_context_window ?? null, estimated: true, categories, entries: [] };
}

/**
 * The best there is for a thread: the breakdown in its timeline, which has the
 * entries; the one in its projection while the timeline is loading; a reckoning
 * from its usage when its agent gives none.
 */
export function contextOf(thread: Thread, stats: ThreadStats | undefined): ContextView | null {
  const said = stats?.context ?? thread.context;
  if (said) return view(said);
  const reckoned = estimate(stats?.usage ?? [], thread.usage?.context_window ?? null);
  if (reckoned) return view(reckoned);
  const used = thread.usage?.context_used;
  if (used == null) return null;
  return view({
    used,
    window: thread.usage?.context_window ?? null,
    estimated: true,
    categories: [{ id: "unknown", name: "Context", tokens: used, kind: "used" }],
  });
}

/** Share of the window, 0–100, or null when the window is unknown. */
export const share = (tokens: number, window: number | null): number | null =>
  window ? (tokens / window) * 100 : null;
