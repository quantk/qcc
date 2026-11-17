import { RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import type { UsageHistoryResult, UsageToolsResult } from "../../api/protocol.gen";
import {
  type Day,
  type Dimension,
  OTHER,
  type Range,
  type Row,
  type Totals,
  addBucket,
  averageTurn,
  byDay,
  byGroup,
  emptyTotals,
  failureRate,
  groupOf,
  inRange,
  seriesSlots,
  tokensOf,
  topThreads,
  turnsEnded,
} from "../../state/metrics";
import { threadTitle } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { compactNumber, elapsed, errorMessage } from "../../util/format";
import { ToolsSection } from "./ToolsSection";
import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const RANGES: { value: Range; label: string }[] = [
  { value: 7, label: "7 days" },
  { value: 30, label: "30 days" },
  { value: 90, label: "90 days" },
  { value: "all", label: "All" },
];

const DIMENSIONS: { value: Dimension; label: string }[] = [
  { value: "project", label: "Project" },
  { value: "agent", label: "Agent" },
  { value: "model", label: "Model" },
];

/** The fill of each series slot, in the palette's order; the rest share a neutral gray. */
const SLOT_FILL = ["bg-chart-1", "bg-chart-2", "bg-chart-3", "bg-chart-4"];
const OTHER_FILL = "bg-muted-foreground/45";

const TOP_THREADS = 8;

const exact = (value: number) => value.toLocaleString("en");
const percent = (value: number | null) => (value === null ? "—" : `${Math.round(value * 100)}%`);
const dayLabel = (day: number, long = false) =>
  new Date(day).toLocaleDateString(undefined, long ? { weekday: "short", month: "short", day: "numeric" } : { month: "short", day: "numeric" });

type Series = { key: string; label: string; fill: string };

/** Spending, turns and the tools used over time, across all agents. */
export function MetricsView() {
  const [history, setHistory] = useState<UsageHistoryResult | null>(null);
  const [tools, setTools] = useState<UsageToolsResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [range, setRange] = useState<Range>(30);
  const [dimension, setDimension] = useState<Dimension>("project");
  const threads = useApp((state) => state.threads);
  const projects = useApp((state) => state.projects);
  const plugins = useApp((state) => state.allPlugins);
  const connected = useApp((state) => state.daemon.status === "connected");

  const load = useCallback(() => {
    setLoading(true);
    const { api } = store();
    Promise.all([api.call("usage.history", {}), api.call("usage.tools", {})])
      .then(([history, tools]) => {
        setHistory(history);
        setTools(tools);
        setError(null);
      })
      .catch((failure: unknown) => setError(errorMessage(failure)))
      .finally(() => setLoading(false));
  }, []);

  // Read on opening, and again when the daemon comes back.
  useEffect(() => {
    if (connected) load();
  }, [connected, load]);

  const view = useMemo(() => {
    const buckets = history?.buckets ?? [];
    const now = Date.now();
    const keyOf = groupOf(dimension, { threads, projects, plugins });
    const slots = seriesSlots(byGroup(buckets, keyOf));
    const shown = inRange(buckets, range, now);
    const rows = byGroup(shown, keyOf);
    const totals = emptyTotals();
    for (const bucket of shown) addBucket(totals, bucket);
    // Series in slot order, so the stack and the legend read the same way every time.
    const named = rows.filter((row) => slots.has(row.key)).sort((a, b) => slots.get(a.key)! - slots.get(b.key)!);
    const series: Series[] = named.map((row) => ({ key: row.key, label: row.label, fill: SLOT_FILL[slots.get(row.key)!]! }));
    if (rows.some((row) => !slots.has(row.key))) series.push({ key: OTHER, label: "Other", fill: OTHER_FILL });
    // Groups without a slot of their own are drawn together as "Other".
    const keyed = (threadId: string) => {
      const group = keyOf(threadId);
      return slots.has(group.key) ? group : { key: OTHER, label: "Other" };
    };
    return {
      keyOf,
      tools: inRange(tools?.buckets ?? [], range, now),
      rows,
      series,
      totals,
      slots,
      days: byDay(shown, keyed, range, now),
      top: topThreads(shown, TOP_THREADS),
    };
  }, [history, tools, range, dimension, threads, projects, plugins]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-wrap items-center gap-x-6 gap-y-2 border-b px-6 py-3">
        <h1 className="text-lg font-semibold">Metrics</h1>
        <span className="flex-1" />
        <Picker label="Range" options={RANGES} value={range} choose={setRange} />
        <Picker label="Group by" options={DIMENSIONS} value={dimension} choose={setDimension} />
        <Button variant="outline" size="sm" disabled={loading || !connected} onClick={load}>
          <RefreshCwIcon className={loading ? "animate-spin" : undefined} />
          Refresh
        </Button>
      </header>
      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-6 py-5">
        {error && <div className="notice error">{error}</div>}
        {!history ? (
          <p className="text-sm text-muted-foreground">{loading ? "Reading the journal…" : ""}</p>
        ) : history.buckets.length === 0 ? (
          <p className="text-sm text-muted-foreground">No agent has reported spending yet.</p>
        ) : (
          <>
            <Summary totals={view.totals} />
            <TokenChart days={view.days} series={view.series} />
            <GroupTable rows={view.rows} slots={view.slots} dimension={dimension} />
            <TopThreads top={view.top} />
            <ToolsSection buckets={view.tools} groups={view.rows} groupOf={view.keyOf} dimension={dimension} />
          </>
        )}
      </div>
    </div>
  );
}

function Picker<T extends string | number>({
  label,
  options,
  value,
  choose,
}: {
  label: string;
  options: { value: T; label: string }[];
  value: T;
  choose: (value: T) => void;
}) {
  return (
    <ToggleGroup
      size="sm"
      variant="outline"
      spacing={0}
      aria-label={label}
      value={[String(value)]}
      onValueChange={(values: unknown[]) => {
        const picked = options.find((option) => String(option.value) === values[0]);
        if (picked) choose(picked.value);
      }}
    >
      {options.map((option) => (
        <ToggleGroupItem
          key={String(option.value)}
          value={String(option.value)}
          className="data-pressed:bg-accent data-pressed:text-foreground"
        >
          {option.label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}

/** The range in four numbers. */
function Summary({ totals }: { totals: Totals }) {
  const tiles = [
    { label: "Tokens in + out", value: compactNumber(tokensOf(totals)), title: exact(tokensOf(totals)) },
    { label: "Turns", value: exact(turnsEnded(totals)), title: `${totals.turnsInterrupted} interrupted` },
    { label: "Failed turns", value: percent(failureRate(totals)), title: `${totals.turnsFailed} failed` },
    { label: "Time working", value: elapsed(totals.workingMs), title: `Average turn ${averageText(totals)}` },
  ];
  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
      {tiles.map((tile) => (
        <div key={tile.label} className="rounded-lg border bg-card px-4 py-3" title={tile.title}>
          <div className="text-xs text-muted-foreground">{tile.label}</div>
          <div className="text-2xl font-semibold tabular-nums">{tile.value}</div>
        </div>
      ))}
    </div>
  );
}

const averageText = (totals: Totals) => {
  const average = averageTurn(totals);
  return average === null ? "—" : elapsed(average);
};

/** Tokens per day, stacked by group. Hovering a day spells it out above the bars. */
function TokenChart({ days, series }: { days: readonly Day[]; series: readonly Series[] }) {
  const [hovered, setHovered] = useState<number | null>(null);
  const scale = Math.max(1, ...days.map((day) => tokensOf(day.totals)));
  const focus = hovered ?? days.length - 1;
  const day = days[focus];
  return (
    <section className="flex flex-col gap-2" aria-label="Tokens per day">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">Tokens per day</h2>
        {series.length > 1 && <Legend series={series} />}
      </div>
      {day && (
        <div className="flex min-h-5 flex-wrap gap-x-4 text-xs text-muted-foreground tabular-nums">
          <span className="text-foreground">
            {dayLabel(day.day, true)} · {exact(tokensOf(day.totals))} tokens
          </span>
          {series
            .filter((entry) => day.groups.has(entry.key))
            .map((entry) => (
              <span key={entry.key} className="flex items-center gap-1.5">
                <span className={`size-2 rounded-[2px] ${entry.fill}`} />
                {entry.label} {compactNumber(tokensOf(day.groups.get(entry.key)!))}
              </span>
            ))}
        </div>
      )}
      <div
        className="flex h-40 items-end gap-[2px] border-b"
        role="img"
        aria-label={`Tokens per day over ${days.length} days; the table below lists them by group`}
        onMouseLeave={() => setHovered(null)}
      >
        {days.map((entry, index) => (
          <div
            key={entry.day}
            className="flex h-full max-w-6 min-w-0 flex-1 flex-col justify-end"
            onMouseEnter={() => setHovered(index)}
          >
            {/* Bottom-up in slot order; a 2px gap of surface keeps segments apart. */}
            <div
              className={`flex flex-col-reverse gap-[2px] ${index === focus ? "" : "opacity-75"}`}
              style={{ height: `${(tokensOf(entry.totals) / scale) * 100}%` }}
            >
              {series
                .filter((item) => entry.groups.has(item.key))
                .map((item, position, shown) => (
                  <div
                    key={item.key}
                    className={`min-h-[2px] w-full ${item.fill} ${position === shown.length - 1 ? "rounded-t-[4px]" : ""}`}
                    style={{ flexGrow: tokensOf(entry.groups.get(item.key)!), flexBasis: 0 }}
                  />
                ))}
            </div>
          </div>
        ))}
      </div>
      <div className="flex justify-between text-xs text-muted-foreground tabular-nums">
        <span>{days[0] && dayLabel(days[0].day)}</span>
        <span>tallest: {compactNumber(scale)}</span>
        <span>{days.at(-1) && dayLabel(days.at(-1)!.day)}</span>
      </div>
    </section>
  );
}

function Legend({ series }: { series: readonly Series[] }) {
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
      {series.map((entry) => (
        <li key={entry.key} className="flex items-center gap-1.5">
          <span className={`size-2.5 rounded-[3px] ${entry.fill}`} />
          {entry.label}
        </li>
      ))}
    </ul>
  );
}

const dimensionTitle: Record<Dimension, string> = { project: "Project", agent: "Agent", model: "Model" };

/** Everything the chart shows, as numbers: also how to read it without color. */
function GroupTable({
  rows,
  slots,
  dimension,
}: {
  rows: readonly Row[];
  slots: ReadonlyMap<string, number>;
  dimension: Dimension;
}) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">By {dimension}</h2>
      <div className="overflow-x-auto rounded-lg border">
        <table className="w-full text-sm tabular-nums">
          <thead className="bg-muted text-xs text-muted-foreground">
            <tr className="[&>th]:px-3 [&>th]:py-1.5 [&>th]:text-right [&>th]:font-medium">
              <th className="text-left!">{dimensionTitle[dimension]}</th>
              <th title="As the agent counts it; for some agents cached input is part of it">Input</th>
              <th>Cached</th>
              <th>Output</th>
              <th>Turns</th>
              <th>Failed</th>
              <th>Average turn</th>
              <th>Working</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => {
              const slot = slots.get(row.key);
              return (
                <tr key={row.key} className="border-t [&>td]:px-3 [&>td]:py-1.5 [&>td]:text-right">
                  <td className="text-left!">
                    <span className="flex items-center gap-2">
                      <span className={`size-2.5 flex-none rounded-[3px] ${slot === undefined ? OTHER_FILL : SLOT_FILL[slot]}`} />
                      <span className="truncate">{row.label}</span>
                    </span>
                  </td>
                  <td title={exact(row.totals.input)}>{compactNumber(row.totals.input)}</td>
                  <td title={exact(row.totals.cached)}>{compactNumber(row.totals.cached)}</td>
                  <td title={exact(row.totals.output)}>{compactNumber(row.totals.output)}</td>
                  <td>{turnsEnded(row.totals)}</td>
                  <td title={`${row.totals.turnsFailed} failed, ${row.totals.turnsInterrupted} interrupted`}>
                    {percent(failureRate(row.totals))}
                  </td>
                  <td>{averageText(row.totals)}</td>
                  <td>{elapsed(row.totals.workingMs)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function TopThreads({ top }: { top: readonly { threadId: string; totals: Totals }[] }) {
  const threads = useApp((state) => state.threads);
  if (top.length === 0) return null;
  const most = tokensOf(top[0]!.totals);
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">Threads that spent most</h2>
      <ul className="flex flex-col">
        {top.map(({ threadId, totals }) => {
          const thread = threads[threadId];
          return (
            <li key={threadId}>
              <Button
                variant="ghost"
                className="h-auto w-full justify-start gap-3 py-1.5 font-normal"
                disabled={!thread}
                onClick={() => store().select(threadId)}
              >
                <span className="w-40 min-w-0 flex-none truncate text-left">{thread ? threadTitle(thread) : "Deleted thread"}</span>
                <span className="h-2 min-w-0 flex-1">
                  <span className="block h-full rounded-r-[4px] bg-chart-1" style={{ width: `${(tokensOf(totals) / most) * 100}%` }} />
                </span>
                <span className="w-16 flex-none text-right text-muted-foreground tabular-nums">
                  {compactNumber(tokensOf(totals))}
                </span>
              </Button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
