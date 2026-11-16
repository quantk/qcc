import { RefreshCwIcon } from "lucide-react";
import { useRef } from "react";

import type { Thread } from "../../api/protocol.gen";
import { plural } from "../../util/format";
import { DiffView, scrollToFile } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { ChangedFiles } from "./ChangedFiles";
import { DeliverButtons } from "./DeliverButtons";
import { useChanges } from "./useChanges";

export function ChangesView({ thread }: { thread: Thread }) {
  const { loaded, error, loading, active, totals, reload } = useChanges(thread);
  const body = useRef<HTMLDivElement>(null);

  return (
    <div className="changes">
      <div className="changes-head flex-wrap gap-y-2">
        {loaded ? (
          <span>
            {plural(loaded.changes.files.length, "file")} changed
            {loaded.changes.files.length > 0 && (
              <span className="diff-stat">
                {" "}
                <span className="add">+{totals.additions}</span> <span className="del">−{totals.deletions}</span>
              </span>
            )}
            {loaded.changes.branch && (
              <>
                {" "}
                on <code>{loaded.changes.branch}</code>
              </>
            )}
            {loaded.changes.base && (
              <>
                {" "}
                since <code title={loaded.changes.base}>{loaded.changes.base.slice(0, 10)}</code>
              </>
            )}
          </span>
        ) : (
          <span>{loading ? "Reading changes…" : ""}</span>
        )}
        <span className="flex-1" />
        <DeliverButtons thread={thread} disabled={active || !loaded?.changes.files.length} done={reload} />
        <Button variant="outline" size="sm" disabled={loading} onClick={reload}>
          <RefreshCwIcon className={loading ? "animate-spin" : undefined} />
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>
      {error && <div className="notice error">{error}</div>}
      {loaded && (
        <div className="changes-body" ref={body}>
          {loaded.changes.files.length === 0 ? (
            <p className="hint">The agent has not changed any files.</p>
          ) : (
            <ChangedFiles changes={loaded.changes} open={(path) => scrollToFile(body.current, path)} />
          )}
          {loaded.truncated && (
            <div className="notice warning">The diff is too large to show in full; what follows is cut short.</div>
          )}
          <DiffView files={loaded.files} />
        </div>
      )}
    </div>
  );
}
