import { RefreshCwIcon } from "lucide-react";
import { useMemo } from "react";

import type { Thread } from "../../api/protocol.gen";
import { readFiles } from "../../state/changes";
import { useApp } from "../../state/store";
import { plural } from "../../util/format";
import { Button } from "@/components/ui/button";
import { ChangedFiles } from "./ChangedFiles";
import { ChangesDiff } from "./ChangesDiff";
import { DeliverButtons } from "./DeliverButtons";
import { CommentsBar, ReviewBar, useReview } from "./review";
import { ScopePicker, nothingChanged, picking } from "./ScopePicker";
import { useChanges } from "./useChanges";

export function ChangesView({ thread }: { thread: Thread }) {
  const { reader, state, changes, error, loading, active, totals, reload, setScope } = useChanges(thread);
  const files = useMemo(() => readFiles(state), [state]);
  const order = useMemo(() => changes?.files.map((file) => file.path) ?? [], [changes]);
  const paths = useMemo(() => new Set(order), [order]);
  const review = useReview(thread, files, paths);
  const focus = useApp((app) => (app.changesFocus?.threadId === thread.id ? app.changesFocus : null));

  return (
    <div className="changes">
      <div className="changes-head flex-wrap gap-y-2">
        {picking(state) && <ScopePicker scope={state.scope} revisions={state.revisions} pick={setScope} />}
        {changes ? (
          <span>
            {plural(changes.files.length, "file")} changed
            {changes.files.length > 0 && (
              <span className="diff-stat">
                {" "}
                <span className="add">+{totals.additions}</span> <span className="del">−{totals.deletions}</span>
              </span>
            )}
            {changes.branch && (
              <>
                {" "}
                on <code>{changes.branch}</code>
              </>
            )}
            {changes.base && state.scope.kind === "all" && (
              <>
                {" "}
                since <code title={changes.base}>{changes.base.slice(0, 10)}</code>
              </>
            )}
          </span>
        ) : (
          <span>{loading ? "Reading changes…" : ""}</span>
        )}
        <span className="flex-1" />
        {/* Only work in an environment has somewhere to go back to. */}
        {thread.environment && (
          <DeliverButtons thread={thread} disabled={active || !changes?.files.length} done={reload} />
        )}
        <Button variant="outline" size="sm" disabled={loading} onClick={reload}>
          <RefreshCwIcon className={loading ? "animate-spin" : undefined} />
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>
      {error && <div className="notice error">{error}</div>}
      {changes && (
        <ChangesDiff
          key={thread.id}
          reader={reader}
          state={state}
          review={review}
          focus={focus}
          inset={20}
          bar={(go) => <CommentsBar thread={thread} review={review} paths={order} go={go} />}
          lead={(jump) => (
            <div className="flex flex-col gap-3">
              {changes.files.length === 0 ? (
                <p className="hint">{nothingChanged(state)}</p>
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
