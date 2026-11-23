// How long things in the timeline took: which durations are worth a word, and the
// words. Pure, so the thresholds are tested apart from the rows that show them.

import { elapsed } from "../util/format";
import type { Turns } from "./fork";
import type { TurnRecord } from "./stats";

/** When something started and, once it has, ended; milliseconds since the epoch. */
export type Timing = { startedAt: number; endedAt?: number };

/**
 * A finished action shorter than this is not labelled: a quick one says nothing by
 * its time, and history that was imported or forked, or a provider that reports an
 * item's start and end at once, gives durations of zero that would mislead.
 */
export const SETTLED_MIN_MS = 2000;

/** A running action shows a live count only once it has gone on this long. */
export const RUNNING_MIN_MS = 5000;

/**
 * What to say about how long something took or has been going, or null when it is
 * not worth saying. `running`: it is still under way, measured up to `now`.
 */
export function durationLabel(timing: Timing | undefined, running: boolean, now: number): string | null {
  if (!timing) return null;
  if (running) {
    const took = now - timing.startedAt;
    return took >= RUNNING_MIN_MS ? elapsed(took) : null;
  }
  if (timing.endedAt === undefined) return null;
  const took = timing.endedAt - timing.startedAt;
  return took >= SETTLED_MIN_MS ? elapsed(took) : null;
}

/**
 * How long until a running action crosses `RUNNING_MIN_MS`, so its row can wait
 * without a ticking clock; 0 once it has.
 */
export function untilShown(timing: Timing, now: number): number {
  return Math.max(0, timing.startedAt + RUNNING_MIN_MS - now);
}

/** The exact times behind a duration, for its tooltip. */
export function timingHint(timing: Timing): string {
  const at = (ts: number) =>
    new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const day = (ts: number) => new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const started = `Started ${day(timing.startedAt)} ${at(timing.startedAt)}`;
  if (timing.endedAt === undefined) return started;
  const sameDay = day(timing.startedAt) === day(timing.endedAt);
  return `${started}, ended ${sameDay ? "" : `${day(timing.endedAt)} `}${at(timing.endedAt)}`;
}

/**
 * How long each ended turn took, by the id of its last reply: the reply under which
 * the turn's time is shown. From the turn's own start and end events, so it counts
 * the whole turn, not just the reply.
 */
export function turnTimings(turns: Turns, records: readonly TurnRecord[]): ReadonlyMap<string, Timing> {
  const timings = new Map<string, Timing>();
  for (const turn of turns.ended) {
    if (!turn.replyItem) continue;
    const record = records.findLast((record) => record.id === turn.turnId && record.endedAt !== undefined);
    if (!record) continue;
    timings.set(turn.replyItem, { startedAt: record.startedAt, endedAt: record.endedAt });
  }
  return timings;
}
