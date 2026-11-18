import { ClockAlertIcon, GaugeIcon, RefreshCwIcon } from "lucide-react";

import type { ProviderId, ProviderLimits } from "../../api/protocol.gen";
import {
  REFRESH_AFTER,
  type LimitLevel,
  type WindowView,
  age,
  allWindows,
  glance,
  isStale,
  knownLimits,
  limitProviders,
  mainWindows,
  shortProviderName,
  worst,
} from "../../state/limits";
import { store, useApp } from "../../state/store";
import { useNow } from "../../util/useNow";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";

// The track is drawn on the sidebar's own color, so it needs a tint of its own to be seen.
const track = "[&_[data-slot=progress-track]]:bg-foreground/10";

const barColor: Record<LimitLevel, string> = {
  ok: `${track} [&_[data-slot=progress-indicator]]:bg-success`,
  high: `${track} [&_[data-slot=progress-indicator]]:bg-primary`,
  full: `${track} [&_[data-slot=progress-indicator]]:bg-destructive`,
};

const textColor: Record<LimitLevel, string> = {
  ok: "",
  high: "text-primary",
  full: "text-destructive",
};

/** The providers that report limits and what is known of each. */
function useKnown() {
  // The example agent (echo) only when examples are shown, as everywhere else.
  const plugins = useApp((state) => state.plugins);
  const limits = useApp((state) => state.limits);
  const providers = limitProviders(plugins);
  return { providers, known: knownLimits(providers, limits) };
}

function RefreshButton({ label = false }: { label?: boolean }) {
  const refreshing = useApp((state) => state.limitsRefreshing);
  const connected = useApp((state) => state.daemon.status === "connected");
  return (
    <Button
      variant="ghost"
      size={label ? "xs" : "icon-xs"}
      disabled={refreshing || !connected}
      title="Ask the agents now (no prompt is sent)"
      onClick={() => void store().refreshLimits()}
    >
      <RefreshCwIcon className={refreshing ? "animate-spin" : undefined} />
      {label && (refreshing ? "Asking…" : "Refresh")}
    </Button>
  );
}

/** One window: its name, a bar and when it starts over. */
function WindowRow({ window }: { window: WindowView }) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline gap-2 text-[13px]">
        <span className="min-w-0 flex-1 truncate">{window.name}</span>
        <span className={cn("font-medium tabular-nums", textColor[window.level])}>{window.percent}%</span>
      </div>
      <Progress value={Math.min(window.percent, 100)} className={barColor[window.level]} />
      {window.reset && <span className="text-xs text-muted-foreground">{window.reset}</span>}
    </div>
  );
}

/** Everything known of one provider's limits. */
export function ProviderLimitsDetails({ id, name, limits }: { id: ProviderId; name: string; limits: ProviderLimits }) {
  const now = useNow(30_000);
  const error = useApp((state) => state.limitErrors.find((entry) => entry.provider === id)?.message);
  const windows = allWindows(limits, now);
  const stale = isStale(limits, now);
  return (
    <section className="flex flex-col gap-2.5" aria-label={`${name} usage limits`}>
      <header className="flex items-center gap-2">
        <span className="font-medium">{name}</span>
        {limits.plan && (
          <Badge variant="secondary" className="capitalize">
            {limits.plan}
          </Badge>
        )}
        <span
          className={cn("ml-auto flex items-center gap-1 text-xs", stale ? "text-primary" : "text-muted-foreground")}
          title={new Date(limits.updated_at).toLocaleString()}
        >
          {stale && <ClockAlertIcon className="size-3" />}
          as of {age(limits.updated_at, now)}
        </span>
      </header>
      {windows.length === 0 ? (
        <p className="text-[13px] text-muted-foreground">
          This account has no subscription limits (an API key or a cloud provider).
        </p>
      ) : (
        windows.map((window) => <WindowRow key={window.id} window={window} />)
      )}
      {error && <p className="text-xs break-words text-destructive">Could not ask again: {error}</p>}
    </section>
  );
}

/** A provider at a glance: its main windows as small bars, the details in a popover. */
function GlanceRow({ id, name, limits }: { id: ProviderId; name: string; limits: ProviderLimits }) {
  const now = useNow(30_000);
  const windows = mainWindows(limits, now);
  const stale = isStale(limits, now);
  const short = shortProviderName(name);
  const level = worst(allWindows(limits, now));
  const title = `${glance(short, windows)}${stale ? ` (as of ${age(limits.updated_at, now)})` : ""}`;
  // Opening the details is a good moment to bring old numbers up to date.
  const opened = (open: boolean) => {
    if (open && Date.now() - limits.updated_at > REFRESH_AFTER) void store().refreshLimits();
  };
  return (
    <Popover onOpenChange={opened}>
      <PopoverTrigger
        render={
          <button
            type="button"
            title={title}
            data-level={level}
            className="flex w-full items-center gap-2 rounded-md px-3 py-1 text-left text-xs hover:bg-muted"
          />
        }
      >
        <span className={cn("w-12 flex-none truncate font-medium", stale && "text-muted-foreground")}>{short}</span>
        {windows.length === 0 && <span className="text-muted-foreground">no limits</span>}
        {windows.map((window) => (
          <span key={window.id} className="flex min-w-0 flex-1 items-center gap-1.5">
            <span className="text-muted-foreground">{window.short}</span>
            <Progress value={Math.min(window.percent, 100)} className={cn("min-w-6 flex-1", barColor[window.level])} />
            <span className={cn("tabular-nums", textColor[window.level], stale && "opacity-70")}>{window.percent}%</span>
          </span>
        ))}
        {stale && <ClockAlertIcon className="size-3 flex-none text-muted-foreground" aria-label="old" />}
      </PopoverTrigger>
      <PopoverContent side="right" align="end" className="w-80">
        <ProviderLimitsDetails id={id} name={name} limits={limits} />
        <div className="flex justify-end border-t pt-2">
          <RefreshButton label />
        </div>
      </PopoverContent>
    </Popover>
  );
}

/** The sidebar section: each provider's 5-hour and weekly windows, always in view. */
export function LimitsGlance() {
  const { providers, known } = useKnown();
  const refreshing = useApp((state) => state.limitsRefreshing);
  if (providers.length === 0) return <p className="px-3 py-1.5 text-xs text-muted-foreground">No agent reports usage limits.</p>;
  return (
    <div className="flex flex-col py-1">
      {known.map((entry) => (
        <GlanceRow key={entry.id} {...entry} />
      ))}
      {known.length === 0 && (
        <p className="px-3 py-1 text-xs text-muted-foreground">{refreshing ? "Asking the agents…" : "Not known yet."}</p>
      )}
    </div>
  );
}

/** Every provider's limits in full: a block to put where the user likes. */
export function LimitsView() {
  const { providers, known } = useKnown();
  return (
    <div className="flex max-w-xl flex-col gap-5 p-4">
      <div className="flex items-center gap-2">
        <GaugeIcon className="size-4 text-muted-foreground" />
        <h2 className="flex-1 text-sm font-semibold">Subscription usage limits</h2>
        <RefreshButton label />
      </div>
      {providers.length === 0 && <p className="text-[13px] text-muted-foreground">No agent reports usage limits.</p>}
      {known.map((entry) => (
        <ProviderLimitsDetails key={entry.id} {...entry} />
      ))}
      {providers
        .filter((provider) => !known.some((entry) => entry.id === provider.id))
        .map((provider) => (
          <p key={provider.id} className="text-[13px] text-muted-foreground">
            {provider.name}: not known yet.
          </p>
        ))}
    </div>
  );
}
