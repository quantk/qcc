import { RefreshCwIcon } from "lucide-react";
import { useRef } from "react";

import type { ThreadContext } from "../../ui/registry";
import { DiffStat, DiffView, scrollToFile } from "../../ui/kit";
import { plural } from "../../util/format";
import { Button } from "@/components/ui/button";
import { ChangedFiles } from "./ChangedFiles";
import { useChanges } from "./useChanges";

/**
 * The diff beside the timeline: what the agent has changed so far, kept current while
 * it works. Delivering the changes stays in the Changes tab, where there is room.
 */
export function ChangesPanel({ thread }: ThreadContext) {
  const { loaded, error, loading, totals, reload } = useChanges(thread);
  const body = useRef<HTMLDivElement>(null);
  const files = loaded?.changes.files ?? [];

  return (
    <div ref={body} className="flex flex-col">
      <div className="sticky top-0 z-10 flex items-center gap-2 border-b bg-muted py-1.5 pr-2 pl-4 text-sm">
        <span className="min-w-0 flex-1 truncate">
          {loaded ? (
            <>
              {plural(files.length, "file")}
              {files.length > 0 && (
                <>
                  {" "}
                  <DiffStat additions={totals.additions} deletions={totals.deletions} />
                </>
              )}
              {loaded.changes.branch && <span className="text-muted-foreground"> on {loaded.changes.branch}</span>}
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
      {loaded && (
        <div className="flex flex-col gap-3 p-3">
          {files.length === 0 ? (
            <p className="text-sm text-muted-foreground">The agent has not changed any files.</p>
          ) : (
            <ChangedFiles changes={loaded.changes} open={(path) => scrollToFile(body.current, path)} />
          )}
          {loaded.truncated && <div className="notice warning">The diff is too large to show in full; it is cut short.</div>}
          <DiffView files={loaded.files} />
        </div>
      )}
    </div>
  );
}
