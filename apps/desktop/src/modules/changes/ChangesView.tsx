import { RefreshCwIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import type { ChangeKind, EnvironmentChanges, Thread } from "../../api/protocol.gen";
import { type DiffFile, parseDiff } from "../../state/diff";
import { store } from "../../state/store";
import { errorMessage, plural } from "../../util/format";
import { DiffStat, DiffView, fileAnchor } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import { DeliverButtons } from "./DeliverButtons";

type Loaded = { changes: EnvironmentChanges; files: DiffFile[]; truncated: boolean };

const kindMark: Record<ChangeKind, string> = { added: "A", modified: "M", deleted: "D", untracked: "N" };
const kindTitle: Record<ChangeKind, string> = {
  added: "Added",
  modified: "Modified",
  deleted: "Deleted",
  untracked: "New, not yet tracked",
};

export function ChangesView({ thread }: { thread: Thread }) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const active = thread.status === "active";

  // Read again whenever a turn ends: that is when the files stop moving.
  useEffect(() => {
    let stale = false;
    setLoading(true);
    const { api } = store();
    const params = { thread_id: thread.id };
    Promise.all([api.call("thread.changes", params), api.call("thread.diff", params)])
      .then(([{ changes }, { diff, truncated }]) => {
        if (stale) return;
        setLoaded({ changes, files: parseDiff(diff), truncated: truncated ?? false });
        setError(null);
      })
      .catch((error: unknown) => {
        if (!stale) setError(errorMessage(error));
      })
      .finally(() => {
        if (!stale) setLoading(false);
      });
    return () => {
      stale = true;
    };
  }, [thread.id, active, refresh]);

  const totals = useMemo(() => {
    const files = loaded?.changes.files ?? [];
    return {
      additions: files.reduce((sum, file) => sum + (file.additions ?? 0), 0),
      deletions: files.reduce((sum, file) => sum + (file.deletions ?? 0), 0),
    };
  }, [loaded]);

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
        <DeliverButtons thread={thread} disabled={active || !loaded?.changes.files.length} done={() => setRefresh(refresh + 1)} />
        <Button variant="outline" size="sm" disabled={loading} onClick={() => setRefresh(refresh + 1)}>
          <RefreshCwIcon className={loading ? "animate-spin" : undefined} />
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>
      {error && <div className="notice error">{error}</div>}
      {loaded && (
        <div className="changes-body">
          {loaded.changes.files.length === 0 ? (
            <p className="hint">The agent has not changed any files.</p>
          ) : (
            <ul className="changed-files">
              {loaded.changes.files.map((file) => (
                <li key={file.path}>
                  <button
                    onClick={() =>
                      document.getElementById(fileAnchor(file.path))?.scrollIntoView({ block: "start", behavior: "smooth" })
                    }
                  >
                    <span className={`kind ${file.kind}`} title={kindTitle[file.kind]}>
                      {kindMark[file.kind]}
                    </span>
                    <span className="diff-path">{file.path}</span>
                    {file.additions !== undefined && file.additions !== null && (
                      <DiffStat additions={file.additions} deletions={file.deletions ?? 0} />
                    )}
                  </button>
                </li>
              ))}
            </ul>
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
