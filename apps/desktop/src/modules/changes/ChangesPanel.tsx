import { RefreshCwIcon } from "lucide-react";
import { useMemo } from "react";

import { readFiles } from "../../state/changes";
import type { ThreadContext } from "../../ui/registry";
import { DiffStat } from "../../ui/kit";
import { plural } from "../../util/format";
import { Button } from "@/components/ui/button";
import { ChangedFiles } from "./ChangedFiles";
import { ChangesDiff } from "./ChangesDiff";
import { CommentsBar, ReviewBar, useReview } from "./review";
import { ScopePicker, nothingChanged, picking } from "./ScopePicker";
import { useChanges } from "./useChanges";

/**
 * The diff beside the timeline: what the agent has changed so far, kept current while
 * it works. Delivering the changes stays in the Changes tab, where there is room.
 */
export function ChangesPanel({ thread }: ThreadContext) {
  const { reader, state, changes, error, loading, totals, reload, setScope } = useChanges(thread);
  const files = useMemo(() => readFiles(state), [state]);
  const order = useMemo(() => changes?.files.map((file) => file.path) ?? [], [changes]);
  const paths = useMemo(() => new Set(order), [order]);
  const review = useReview(thread, files, paths);
  const count = changes?.files.length ?? 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-none items-center gap-2 border-b bg-muted py-1.5 pr-2 pl-4 text-sm">
        {picking(state) && <ScopePicker scope={state.scope} revisions={state.revisions} pick={setScope} size="xs" />}
        <span className="min-w-0 flex-1 truncate">
          {changes ? (
            <>
              {plural(count, "file")}
              {count > 0 && (
                <>
                  {" "}
                  <DiffStat additions={totals.additions} deletions={totals.deletions} />
                </>
              )}
              {changes.branch && <span className="text-muted-foreground"> on {changes.branch}</span>}
            </>
          ) : (
            <span className="text-muted-foreground">{loading ? "Reading changes…" : ""}</span>
          )}
        </span>
        <Button variant="ghost" size="icon-xs" disabled={loading} title="Read the changes again" onClick={reload}>
          <RefreshCwIcon className={loading ? "animate-spin" : undefined} />
        </Button>
      </div>
      {error && <div className="notice error m-3">{error}</div>}
      {changes && (
        <ChangesDiff
          key={thread.id}
          reader={reader}
          state={state}
          review={review}
          inset={12}
          bar={(go) => <CommentsBar thread={thread} review={review} paths={order} go={go} compact />}
          lead={(jump) => (
            <div className="flex flex-col gap-3">
              {count === 0 ? (
                <p className="text-sm text-muted-foreground">{nothingChanged(state)}</p>
              ) : (
                <ChangedFiles changes={changes} open={jump} />
              )}
              <ReviewBar thread={thread} review={review} />
            </div>
          )}
        />
      )}
    </div>
  );
}
