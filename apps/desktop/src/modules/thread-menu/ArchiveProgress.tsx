// How an archive under way is going, as the daemon reports it: a bar, then each
// thread of the tree with its step. Shown only once an archive has taken longer
// than `PROGRESS_DELAY_MS`, so a quick one does not flash anything.

import { CircleCheckIcon, CircleDashedIcon, CircleMinusIcon, CircleXIcon, LoaderCircleIcon } from "lucide-react";
import { useEffect, useState } from "react";

import type { ArchiveProgress, ArchiveStep } from "../../api/protocol.gen";
import { archiveSummary, isFinal, PROGRESS_DELAY_MS, stepLabel, treeOrder } from "../../state/archive";
import { useApp } from "../../state/store";
import { Progress, ProgressLabel, ProgressValue } from "@/components/ui/progress";

/** True once `active` has stayed true for `ms`; false again as soon as it is not. */
export function useDelayed(active: boolean, ms: number = PROGRESS_DELAY_MS): boolean {
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (!active) {
      setLate(false);
      return;
    }
    const timer = setTimeout(() => setLate(true), ms);
    return () => clearTimeout(timer);
  }, [active, ms]);
  return active && late;
}

function StepIcon({ step }: { step: ArchiveStep }) {
  const className = "size-4 shrink-0";
  switch (step.kind) {
    case "waiting":
      return <CircleDashedIcon className={`${className} text-muted-foreground`} aria-label="waiting" />;
    case "archived":
      return <CircleCheckIcon className={`${className} text-primary`} aria-label="archived" />;
    case "refused":
      return <CircleXIcon className={`${className} text-destructive`} aria-label="not archived" />;
    case "kept":
      return <CircleMinusIcon className={`${className} text-muted-foreground`} aria-label="kept" />;
    default:
      return <LoaderCircleIcon className={`${className} animate-spin text-muted-foreground`} aria-label="in progress" />;
  }
}

/** The bar and, for a tree, the list of its threads. */
export function ArchiveProgressView({ threads }: { threads: readonly ArchiveProgress[] }) {
  const summary = archiveSummary(threads);
  const known = useApp((state) => state.threads);
  const status = summary.current ? stepLabel(summary.current.step) : summary.done === summary.total ? "Done" : "Starting…";
  return (
    <div className="flex flex-col gap-3" aria-live="polite">
      <Progress value={summary.percent} aria-label={summary.headline}>
        <ProgressLabel className="min-w-0 flex-1 truncate font-normal text-muted-foreground">
          {summary.total > 1 && summary.current
            ? `${status}: ${summary.current.title ?? summary.current.thread_id}`
            : status}
        </ProgressLabel>
        {summary.total > 1 ? (
          <span className="ml-auto text-sm text-muted-foreground tabular-nums">
            {summary.done} of {summary.total}
          </span>
        ) : (
          <ProgressValue className="ml-auto text-sm text-muted-foreground tabular-nums" />
        )}
      </Progress>
      {summary.total > 1 && (
        <ul className="max-h-64 overflow-y-auto text-sm" aria-label="Threads being archived">
          {treeOrder(threads, (id) => known[id]?.created_at).map(({ thread, depth }) => (
            <li key={thread.thread_id} className="flex items-center gap-2 py-0.5" style={{ paddingLeft: `${depth}rem` }}>
              <StepIcon step={thread.step} />
              <span className={`truncate ${thread.step.kind === "waiting" ? "text-muted-foreground" : ""}`}>
                {thread.title ?? thread.thread_id}
              </span>
              {thread.step.kind !== "waiting" && thread.step.kind !== "archived" && (
                <span
                  className={`ml-auto max-w-1/2 truncate text-xs ${
                    thread.step.kind === "refused" ? "text-destructive" : "text-muted-foreground"
                  } ${isFinal(thread.step) ? "" : "animate-pulse"}`}
                  title={stepLabel(thread.step)}
                >
                  {stepLabel(thread.step)}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
