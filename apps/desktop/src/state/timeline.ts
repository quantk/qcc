// Folds a thread's events into what the timeline shows. Pure: the same events
// always give the same entries, whether they arrive live or from the journal.

import type {
  InteractionId,
  InteractionRequest,
  InteractionResolution,
  Item,
  ThreadEvent,
  UsageSnapshot,
} from "../api/protocol.gen";

export type Entry =
  | {
      kind: "user";
      key: string;
      ts: number;
      text: string;
      steered: boolean;
      /** Waiting for the running turn to end. */
      queued: boolean;
      queueId?: string;
    }
  | { kind: "item"; key: string; ts: number; item: Item }
  | {
      kind: "interaction";
      key: string;
      ts: number;
      id: InteractionId;
      request: InteractionRequest;
      resolution?: InteractionResolution;
    }
  | { kind: "notice"; key: string; ts: number; level: "info" | "warning" | "error"; text: string };

export type Timeline = {
  /** Sequence number of the last event folded in. */
  lastSeq: number;
  entries: readonly Entry[];
  turnOpen: boolean;
  usage?: UsageSnapshot;
};

export const emptyTimeline: Timeline = { lastSeq: 0, entries: [], turnOpen: false };

function replaceLast(
  entries: readonly Entry[],
  matches: (entry: Entry) => boolean,
  update: (entry: Entry) => Entry | null,
): readonly Entry[] | null {
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index]!;
    if (!matches(entry)) continue;
    const next = entries.slice();
    const updated = update(entry);
    if (updated) next[index] = updated;
    else next.splice(index, 1);
    return next;
  }
  return null;
}

const isItem = (id: string) => (entry: Entry) => entry.kind === "item" && entry.item.id === id;
const isQueued = (queueId: string) => (entry: Entry) =>
  entry.kind === "user" && entry.queued && entry.queueId === queueId;

/** Fold one event in. Events already seen are ignored, so replays are harmless. */
export function applyEvent(timeline: Timeline, event: ThreadEvent): Timeline {
  if (event.seq <= timeline.lastSeq) return timeline;
  const key = String(event.seq);
  const { ts } = event;
  let { entries, turnOpen, usage } = timeline;
  const push = (entry: Entry) => {
    entries = [...entries, entry];
  };
  const notice = (level: "info" | "warning" | "error", text: string) =>
    push({ kind: "notice", key, ts, level, text });

  switch (event.type) {
    case "input/submitted": {
      if (event.queue_id) entries = replaceLast(entries, isQueued(event.queue_id), () => null) ?? entries;
      push({ kind: "user", key, ts, text: event.text, steered: event.steered ?? false, queued: false });
      break;
    }
    case "input/queued":
      push({ kind: "user", key, ts, text: event.text, steered: false, queued: true, queueId: event.queue_id });
      break;
    case "input/dropped":
      entries = replaceLast(entries, isQueued(event.queue_id), () => null) ?? entries;
      break;
    case "turn/started":
      turnOpen = true;
      break;
    case "turn/completed":
      turnOpen = false;
      if (event.status === "failed") notice("error", event.error ?? "The turn failed");
      else if (event.status === "interrupted") notice("info", "Interrupted");
      break;
    case "item/started":
      push({ kind: "item", key: event.item.id, ts, item: event.item });
      break;
    case "item/text/delta": {
      const { text } = event;
      entries =
        replaceLast(entries, isItem(event.item_id), (entry) => {
          if (entry.kind !== "item" || !("text" in entry.item)) return entry;
          return { ...entry, item: { ...entry.item, text: entry.item.text + text } };
        }) ?? entries;
      break;
    }
    case "item/completed": {
      const { item } = event;
      const replaced = replaceLast(entries, isItem(item.id), (entry) => ({ ...entry, item }) as Entry);
      if (replaced) entries = replaced;
      else push({ kind: "item", key: item.id, ts, item });
      break;
    }
    case "usage/updated":
      usage = event.usage;
      break;
    case "interaction/requested":
      push({ kind: "interaction", key, ts, id: event.interaction_id, request: event.request });
      break;
    case "interaction/resolved": {
      const { interaction_id: id, resolution } = event;
      entries =
        replaceLast(
          entries,
          (entry) => entry.kind === "interaction" && entry.id === id,
          (entry) => ({ ...entry, resolution }) as Entry,
        ) ?? entries;
      break;
    }
    case "session/started":
      if (event.resumed) notice("info", "Session resumed");
      break;
    case "session/ended":
      if (event.reason) notice("info", `Session ended: ${event.reason}`);
      break;
    case "thread/permission-mode":
      notice("info", `Permission mode: ${event.permission_mode ?? "agent default"}`);
      break;
    case "thread/agent":
      notice("info", `Agent: ${event.provider}${event.model ? `, model ${event.model}` : ", default model"}`);
      break;
    case "thread/archived":
      notice("info", "Thread archived");
      break;
    case "environment/preparing":
      notice("info", "Preparing the environment…");
      break;
    case "environment/ready":
      notice("info", "The environment is ready");
      break;
    case "environment/failed":
      notice("error", `Environment failed: ${event.message}`);
      break;
    case "environment/removed":
      notice("info", "Environment removed");
      break;
    case "provider/error":
      notice("error", event.message);
      break;
    case "provider/warning":
      notice("warning", event.message);
      break;
    case "thread/created":
    case "thread/status":
    case "session/identified":
    case "provider/unhandled":
      break;
  }
  return { lastSeq: event.seq, entries, turnOpen, usage };
}

export function applyEvents(timeline: Timeline, events: readonly ThreadEvent[]): Timeline {
  return events.reduce(applyEvent, timeline);
}

/** Interactions nobody answered yet, oldest first. */
export function pendingInteractions(timeline: Timeline): Entry[] {
  return timeline.entries.filter((entry) => entry.kind === "interaction" && !entry.resolution);
}

export type QueuedMessage = { queueId: string; text: string };

/** Messages waiting for the running turn to end, oldest first. */
export function queuedMessages(timeline: Timeline | undefined): QueuedMessage[] {
  const queued: QueuedMessage[] = [];
  for (const entry of timeline?.entries ?? []) {
    if (entry.kind === "user" && entry.queued && entry.queueId) queued.push({ queueId: entry.queueId, text: entry.text });
  }
  return queued;
}
