// How the threads a thread started are doing, counted for a glance: the chip beside
// the composer shows these numbers, and the user reads whether a child waits for
// them without opening its tab.

import type { Thread } from "../api/protocol.gen";
import { plural } from "../util/format";
import { childState } from "./children";

/** Children by what they are up to. Every child is in exactly one count. */
export type ChildrenTally = {
  total: number;
  /** Asking the user something: an approval or a question. */
  waiting: number;
  working: number;
  /** The session or its last turn failed. */
  failed: number;
  /** Finished, stopped or archived: nothing more will happen without the user. */
  done: number;
};

export function tallyChildren(children: readonly Thread[]): ChildrenTally {
  const tally: ChildrenTally = { total: children.length, waiting: 0, working: 0, failed: 0, done: 0 };
  for (const child of children) {
    // The Agents tab's states, folded into four: an archived child is put away, so
    // done whatever it was doing; one that is idle or finished waits on nobody.
    const state = child.archived ? "done" : childState(child);
    if (state === "waiting") tally.waiting += 1;
    else if (state === "working") tally.working += 1;
    else if (state === "failed") tally.failed += 1;
    else tally.done += 1;
  }
  return tally;
}

/** The counts worth naming, most urgent first: a child that waits for the user leads. */
export function tallyParts(tally: ChildrenTally): { key: "waiting" | "working" | "failed" | "done"; count: number; label: string }[] {
  const parts = [
    { key: "waiting" as const, count: tally.waiting, label: `${tally.waiting} waiting for you` },
    { key: "working" as const, count: tally.working, label: `${tally.working} working` },
    { key: "failed" as const, count: tally.failed, label: `${tally.failed} failed` },
    { key: "done" as const, count: tally.done, label: `${tally.done} done` },
  ];
  return parts.filter((part) => part.count > 0);
}

/** The tally in a sentence, for a tooltip or a screen reader. */
export function tallyLabel(tally: ChildrenTally): string {
  const parts = tallyParts(tally).map((part) => part.label);
  return `${plural(tally.total, "agent")} started here${parts.length ? `: ${parts.join(", ")}` : ""}`;
}
