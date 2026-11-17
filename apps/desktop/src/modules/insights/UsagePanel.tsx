import { useState } from "react";

import type { TokenUsage } from "../../api/protocol.gen";
import { type UsagePoint, contextUsed, threadTotal } from "../../state/stats";
import type { ThreadContext } from "../../ui/registry";
import { compactNumber, plural } from "../../util/format";
import { Fact, Quiet, Section } from "./Facts";

/** The chart keeps to the latest requests: older ones would be slivers. */
const CHART_POINTS = 48;

const exact = (value: number) => value.toLocaleString("en");
const time = (ts: number) => new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

export function UsagePanel({ timeline }: ThreadContext) {
  if (!timeline) return <Quiet>Loading…</Quiet>;
  const { stats, usage } = timeline;
  const total = threadTotal(stats);
  const spent = total.input_tokens + total.output_tokens + (total.cached_input_tokens ?? 0) > 0;
  if (!usage && !spent) {
    return (
      <Section title="Context">
        <Quiet>The agent has not reported token usage yet.</Quiet>
      </Section>
    );
  }
  const limit = usage?.model_context_window ?? null;
  return (
    <>
      {usage && (
        <Section title="Last request">
          <Breakdown usage={usage.last} />
        </Section>
      )}
      {stats.usage.length > 1 && <ContextChart points={stats.usage} limit={limit} />}
      {stats.compactions.length > 0 && (
        <Section title="Compactions" aside={plural(stats.compactions.length, "time")}>
          {stats.compactions
            .slice(-3)
            .reverse()
            .map(({ ts, compaction }) => (
              <Fact key={ts} label={`${time(ts)}${compaction.automatic ? " · auto" : ""}`}>
                {compaction.tokens_before != null && compaction.tokens_after != null
                  ? `${compactNumber(compaction.tokens_before)} → ${compactNumber(compaction.tokens_after)}`
                  : "—"}
              </Fact>
            ))}
        </Section>
      )}
      {spent && (
        <Section title="Spent in this thread">
          <Breakdown usage={total} />
          <Fact label="Total">
            <span className="font-medium">
              {exact(total.input_tokens + total.output_tokens + (total.cached_input_tokens ?? 0))}
            </span>
          </Fact>
        </Section>
      )}
    </>
  );
}

function Breakdown({ usage }: { usage: TokenUsage }) {
  return (
    <>
      <Fact label="Input">{exact(usage.input_tokens)}</Fact>
      {usage.cached_input_tokens != null && <Fact label="From cache">{exact(usage.cached_input_tokens)}</Fact>}
      <Fact label="Output">{exact(usage.output_tokens)}</Fact>
    </>
  );
}

/** Context held at each request of the conversation: how fast it fills. */
function ContextChart({ points, limit }: { points: readonly UsagePoint[]; limit: number | null }) {
  const [hovered, setHovered] = useState<number | null>(null);
  const shown = points.slice(-CHART_POINTS);
  const offset = points.length - shown.length;
  const values = shown.map((point) => contextUsed(point.usage));
  const scale = Math.max(limit ?? 0, ...values, 1);
  const focus = hovered ?? values.length - 1;
  const point = shown[focus]!;
  return (
    <Section title="Context per request" aside={plural(points.length, "request")}>
      <div className="text-xs text-muted-foreground tabular-nums">
        #{offset + focus + 1} · {time(point.ts)} · {exact(values[focus]!)} tokens
        {limit ? ` · ${Math.round((values[focus]! / limit) * 100)}%` : ""}
      </div>
      <div
        className="flex h-16 items-end gap-[2px] border-b border-border"
        role="img"
        aria-label={`Context per request, the latest ${compactNumber(values.at(-1)!)} tokens`}
        onMouseLeave={() => setHovered(null)}
      >
        {values.map((value, index) => (
          <div
            key={offset + index}
            className="flex h-full max-w-4 min-w-0 flex-1 items-end"
            onMouseEnter={() => setHovered(index)}
          >
            <div
              className={`w-full rounded-t-[4px] ${index === focus ? "bg-primary" : "bg-primary/45"}`}
              style={{ height: `${Math.max(2, (value / scale) * 100)}%` }}
            />
          </div>
        ))}
      </div>
      {limit && <div className="text-right text-xs text-muted-foreground">full height: {compactNumber(limit)}</div>}
    </Section>
  );
}
