// What agents used across threads: commands, tools, MCP tools and skills, in the
// range the Metrics view shows, for every thread or one group of them.

import { useMemo, useState } from "react";

import type { ToolBucket } from "../../api/protocol.gen";
import type { Group, Row } from "../../state/metrics";
import {
  type KindFilter,
  type ToolRow,
  byServer,
  failureShare,
  kindLabel,
  ofKind,
  rowLabel,
  toolsOfBuckets,
} from "../../state/tools";
import { compactNumber, elapsed } from "../../util/format";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const KINDS: { value: KindFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "tools", label: "Tools" },
  { value: "mcp", label: "MCP" },
  { value: "skills", label: "Skills" },
  { value: "commands", label: "Programs" },
];

/** Rows listed before the rest is left out. */
const LISTED = 25;
const ALL = "all";

const percent = (value: number | null) => (value === null ? "—" : `${Math.round(value * 100)}%`);
/** Quick tools take less than a second; "0s" would read as nothing. */
const duration = (ms: number) => (ms <= 0 ? "—" : ms < 1000 ? "<1s" : elapsed(ms));

export function ToolsSection({
  buckets,
  groups,
  groupOf,
  dimension,
}: {
  /** Already cut to the range shown. */
  buckets: readonly ToolBucket[];
  /** The groups of the current dimension, to narrow the counts to one. */
  groups: readonly Row[];
  groupOf: (threadId: string) => Group;
  dimension: string;
}) {
  const [kind, setKind] = useState<KindFilter>("all");
  const [group, setGroup] = useState<string>(ALL);
  // A group from another dimension, or one gone from the range, means all of them.
  const scope = groups.some((row) => row.key === group) ? group : ALL;
  const rows = useMemo(
    () => toolsOfBuckets(buckets, scope === ALL ? undefined : (threadId) => groupOf(threadId).key === scope),
    [buckets, scope, groupOf],
  );
  const shown = ofKind(rows, kind);
  const servers = kind === "mcp" ? byServer(rows) : [];
  const items = [{ value: ALL, label: `Every ${dimension}` }, ...groups.map((row) => ({ value: row.key, label: row.label }))];

  return (
    <section className="flex flex-col gap-2" aria-label="Tools used">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">What agents used</h2>
        <span className="flex-1" />
        <ToggleGroup
          size="sm"
          variant="outline"
          spacing={0}
          aria-label="Kind"
          value={[kind]}
          onValueChange={(values: unknown[]) => {
            const picked = KINDS.find((option) => option.value === values[0]);
            if (picked) setKind(picked.value);
          }}
        >
          {KINDS.map((option) => (
            <ToggleGroupItem key={option.value} value={option.value} className="data-pressed:bg-accent data-pressed:text-foreground">
              {option.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <Select items={items} value={scope} onValueChange={(value) => setGroup(String(value ?? ALL))}>
          <SelectTrigger size="sm" className="min-w-40" aria-label={`Narrow to one ${dimension}`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {items.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {servers.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {servers.map((server) => (
            <div key={server.server} className="rounded-lg border bg-card px-3 py-2 text-sm">
              <div className="text-xs text-muted-foreground">{server.server}</div>
              <div className="tabular-nums">
                <span className="font-semibold">{compactNumber(server.calls)}</span> {server.calls === 1 ? "call" : "calls"} ·{" "}
                {server.tools} {server.tools === 1 ? "tool" : "tools"}
                {server.failed > 0 && ` · ${server.failed} failed`}
              </div>
            </div>
          ))}
        </div>
      )}
      {shown.length === 0 ? (
        <p className="text-sm text-muted-foreground">Nothing of this kind was used in this range.</p>
      ) : (
        <ToolTable rows={shown} />
      )}
    </section>
  );
}

function ToolTable({ rows }: { rows: readonly ToolRow[] }) {
  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full text-sm tabular-nums">
        <thead className="bg-muted text-xs text-muted-foreground">
          <tr className="[&>th]:px-3 [&>th]:py-1.5 [&>th]:text-right [&>th]:font-medium">
            <th className="text-left!">Name</th>
            <th className="text-left!">Kind</th>
            <th>Uses</th>
            <th>Threads</th>
            <th title="Failed, a command's non-zero exit included">Failed</th>
            <th title="Refused by the user when asked">Declined</th>
            <th>Average</th>
            <th>Longest</th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, LISTED).map((row) => (
            <tr key={row.key} className="border-t [&>td]:px-3 [&>td]:py-1.5 [&>td]:text-right">
              <td className="max-w-80 truncate text-left!" title={row.name || undefined}>
                {rowLabel(row)}
              </td>
              <td className="text-left! text-muted-foreground">{kindLabel(row.kind)}</td>
              <td>{row.calls.toLocaleString("en")}</td>
              <td>{row.threads}</td>
              <td title={`${row.failed} failed, ${row.interrupted} interrupted`}>{percent(failureShare(row))}</td>
              <td>{row.declined || "—"}</td>
              <td>{duration(row.calls > 0 ? row.durationMs / row.calls : 0)}</td>
              <td>{duration(row.longestMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length > LISTED && (
        <p className="border-t px-3 py-1.5 text-xs text-muted-foreground">and {rows.length - LISTED} more</p>
      )}
    </div>
  );
}
