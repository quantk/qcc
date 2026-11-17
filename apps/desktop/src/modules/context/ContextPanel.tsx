import { ChevronRightIcon } from "lucide-react";
import { useState } from "react";

import { type ContextView, GROUPS, type Group, type Part, contextOf, share } from "../../state/context";
import type { ThreadContext } from "../../ui/registry";
import { compactNumber } from "../../util/format";
import { Fact, Quiet, Section } from "../insights/Facts";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/** The fill of each group: the palette's series in a fixed order, the rest gray. */
const FILL: Record<Group, string> = {
  messages: "bg-chart-1",
  system: "bg-chart-2",
  mcp: "bg-chart-3",
  memory: "bg-chart-4",
  other: "bg-muted-foreground/45",
};

const exact = (value: number) => value.toLocaleString("en");
const percentText = (tokens: number, window: number | null) => {
  const value = share(tokens, window);
  if (value === null) return "";
  return value > 0 && value < 1 ? "<1%" : `${Math.round(value)}%`;
};

export function ContextPanel({ thread, timeline, provider }: ThreadContext) {
  const view = contextOf(thread, timeline?.stats);
  if (!view) {
    return (
      <Section title="Context">
        <Quiet>The agent has not said how full its context is yet.</Quiet>
      </Section>
    );
  }
  const breaksDown = provider?.capabilities.context_usage ?? false;
  return (
    <>
      <Headline view={view} />
      <Parts view={view} />
      {view.deferred.length > 0 && (
        <Section title="Loaded when needed" aside="not in the window">
          {view.deferred.map((part) => (
            <PartRow key={part.id} part={part} window={view.window} />
          ))}
        </Section>
      )}
      {view.estimated && (
        <Section title="About these numbers">
          <Quiet>
            {breaksDown
              ? "The agent has not broken its context down yet; until it does, the parts are reckoned from its token usage."
              : "This agent does not break its context down. The parts are reckoned from its token usage: the smallest request of the conversation is taken as what it starts with."}
          </Quiet>
        </Section>
      )}
    </>
  );
}

function Headline({ view }: { view: ContextView }) {
  const percent = share(view.used, view.window);
  return (
    <Section title="Context window" aside={view.estimated ? <Badge variant="outline">estimate</Badge> : undefined}>
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-2xl font-semibold tabular-nums">
          {percent !== null ? `${Math.round(percent)}%` : compactNumber(view.used)}
        </span>
        <span className="text-sm text-muted-foreground tabular-nums">
          {view.window ? `${compactNumber(view.used)} of ${compactNumber(view.window)}` : "tokens, window unknown"}
        </span>
      </div>
      <Bar view={view} />
    </Section>
  );
}

/**
 * The window as one bar: the parts in it, in their groups' colors and the agent's
 * order, then what is kept back for compacting, then what is free. Hovering a
 * segment names it.
 */
function Bar({ view }: { view: ContextView }) {
  const [hovered, setHovered] = useState<string | null>(null);
  const scale = view.window ?? Math.max(1, view.used + view.buffer);
  const focus = view.usedParts.find((part) => part.id === hovered);
  const width = (tokens: number) => `${Math.max(0.5, (tokens / scale) * 100)}%`;
  return (
    <div className="flex flex-col gap-1">
      <div
        className="flex h-3 w-full gap-[2px] overflow-hidden rounded-[4px] bg-muted"
        role="img"
        aria-label={`Context: ${view.usedParts.map((part) => `${part.name} ${exact(part.tokens)}`).join(", ")}`}
        onMouseLeave={() => setHovered(null)}
      >
        {view.usedParts.map((part) => (
          <div
            key={part.id}
            className={`h-full ${FILL[part.group]} ${hovered && hovered !== part.id ? "opacity-60" : ""}`}
            style={{ width: width(part.tokens) }}
            onMouseEnter={() => setHovered(part.id)}
          />
        ))}
        {view.buffer > 0 && view.window && (
          <div
            className="h-full opacity-40"
            style={{
              width: width(view.buffer),
              // Hatched: room that is neither used nor free.
              background: "repeating-linear-gradient(135deg, var(--color-muted-foreground) 0 1px, transparent 1px 5px)",
            }}
            title={`Kept for compacting: ${exact(view.buffer)} tokens`}
          />
        )}
      </div>
      <div className="min-h-4 text-xs text-muted-foreground tabular-nums">
        {focus
          ? `${focus.name} · ${exact(focus.tokens)} tokens ${percentText(focus.tokens, view.window)}`
          : view.free !== null
            ? `${compactNumber(view.free)} free${view.buffer > 0 ? ` · ${compactNumber(view.buffer)} kept for compacting` : ""}`
            : ""}
      </div>
    </div>
  );
}

function Parts({ view }: { view: ContextView }) {
  const groups = new Set(view.usedParts.map((part) => part.group));
  const legend = GROUPS.filter((entry) => groups.has(entry.group));
  return (
    <Section title="In the window">
      {legend.length > 1 && (
        <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {legend.map((entry) => (
            <span key={entry.group} className="flex items-center gap-1.5">
              <span className={`size-2 rounded-[2px] ${FILL[entry.group]}`} />
              {entry.label}
            </span>
          ))}
        </div>
      )}
      {view.usedParts.map((part) => (
        <PartRow key={part.id} part={part} window={view.window} swatch />
      ))}
      {view.buffer > 0 && <Fact label="Kept for compacting">{exact(view.buffer)}</Fact>}
      {view.free !== null && <Fact label="Free">{exact(view.free)}</Fact>}
    </Section>
  );
}

/** A part with its tokens and share; one with entries opens to list them. */
function PartRow({ part, window, swatch }: { part: Part; window: number | null; swatch?: boolean }) {
  const label = (
    <span className="flex min-w-0 items-center gap-1.5">
      {swatch && <span className={`size-2 shrink-0 rounded-[2px] ${FILL[part.group]}`} />}
      <span className="truncate">{part.name}</span>
    </span>
  );
  const value = (
    <span className="shrink-0 tabular-nums">
      {exact(part.tokens)}
      <span className="ml-2 inline-block w-9 text-right text-muted-foreground">{percentText(part.tokens, window)}</span>
    </span>
  );
  if (part.entries.length === 0) {
    return (
      <div className="flex items-baseline justify-between gap-3 pl-[18px] text-sm">
        {label}
        {value}
      </div>
    );
  }
  return (
    <Collapsible>
      <CollapsibleTrigger className="group flex w-full items-baseline justify-between gap-3 text-left text-sm">
        <span className="flex min-w-0 items-center gap-0.5">
          <ChevronRightIcon className="size-4 shrink-0 self-center text-muted-foreground transition-transform group-data-[panel-open]:rotate-90" />
          {label}
        </span>
        {value}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ul className="mt-1 mb-1 flex flex-col gap-0.5 pl-[18px] text-xs text-muted-foreground">
          {part.entries.map((entry) => (
            <li key={entry.name} className="flex justify-between gap-3" title={entry.name}>
              <span className="min-w-0 truncate">{entry.name}</span>
              <span className="shrink-0 tabular-nums">{exact(entry.tokens)}</span>
            </li>
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  );
}
