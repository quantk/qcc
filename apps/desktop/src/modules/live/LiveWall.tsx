import {
  BrainIcon,
  LayoutGridIcon,
  LayoutListIcon,
  FileTextIcon,
  ListChecksIcon,
  MessageSquareIcon,
  PencilIcon,
  SearchIcon,
  TerminalIcon,
  TriangleAlertIcon,
  UserIcon,
  WrenchIcon,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { Thread, ThreadId } from "../../api/protocol.gen";
import {
  DENSITY,
  DENSITY_KEY,
  type Density,
  type TailLine,
  contextFill,
  parseDensity,
  tailOf,
  wallThreads,
} from "../../state/live";
import { attentionLabel, attentionOf, cardTime, folderOf, providerName, threadTitle } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { InteractionPrompt, ThreadMenu } from "../../ui/kit";
import { baseName, compactNumber, relativePath, tailPath } from "../../util/format";
import { readText, saveText } from "../../util/storage";
import { useNow } from "../../util/useNow";
import { Progress } from "@/components/ui/progress";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";

const icons: Record<TailLine["kind"], LucideIcon> = {
  message: MessageSquareIcon,
  thought: BrainIcon,
  command: TerminalIcon,
  edit: PencilIcon,
  read: FileTextIcon,
  search: SearchIcon,
  tool: WrenchIcon,
  plan: ListChecksIcon,
  notice: TriangleAlertIcon,
  you: UserIcon,
};

/**
 * Keep the timelines of `ids` current while the wall is open. Holds are taken for
 * newcomers before old ones are let go, so a thread that stays is never dropped
 * and followed again.
 */
function useFollowing(ids: readonly ThreadId[]) {
  const holds = useRef(new Map<ThreadId, () => void>());
  const key = ids.join("\n");
  useEffect(() => {
    const wanted = new Set(key ? key.split("\n") : []);
    for (const id of wanted) if (!holds.current.has(id)) holds.current.set(id, store().hold(id));
    for (const [id, release] of holds.current) {
      if (wanted.has(id)) continue;
      release();
      holds.current.delete(id);
    }
  }, [key]);
  useEffect(() => {
    const held = holds.current;
    return () => {
      for (const release of held.values()) release();
      held.clear();
    };
  }, []);
}

/** Scroll a tail to its latest step, and mark it when earlier steps are cut off above. */
function toEnd(element: HTMLElement) {
  element.scrollTop = element.scrollHeight;
  element.dataset.clipped = String(element.scrollTop > 0);
}

/**
 * The agent's latest steps in a box of bounded height that always shows its end:
 * the latest step is the one worth seeing, and the whole timeline is a click away.
 * While the steps fit, they read from the top like any list.
 */
function Tail({ thread, density }: { thread: Thread; density: Density }) {
  const entries = useApp((state) => state.timelines[thread.id]?.entries);
  const { steps, detail } = DENSITY[density];
  const lines = useMemo(() => tailOf(entries ?? [], steps, detail), [entries, steps, detail]);
  const box = useRef<HTMLDivElement>(null);
  // After every render, as the steps change with each event of the thread.
  useLayoutEffect(() => {
    if (box.current) toEnd(box.current);
  });
  // And when the tile changes width, which rewraps the steps.
  useEffect(() => {
    const element = box.current;
    if (!element) return;
    const observer = new ResizeObserver(() => toEnd(element));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const pathChars = density === "compact" ? 40 : 60;
  return (
    // The rule and its padding stay put while the steps scroll under them.
    <div className={cn("flex-none border-t", density === "compact" ? "pt-1.5" : "pt-2")}>
      <div
        ref={box}
        className={cn(
          "overflow-hidden data-[clipped=true]:[mask-image:linear-gradient(to_bottom,transparent,black_1.25rem)]",
          density === "compact" ? "max-h-28" : "max-h-72",
        )}
      >
        {!entries ? (
          <p className="text-xs text-muted-foreground">Reading…</p>
        ) : lines.length === 0 ? (
          <p className="text-xs text-muted-foreground">Nothing yet.</p>
        ) : (
          <ol className="flex min-w-0 flex-col gap-1 font-mono text-xs">
            {lines.map((line) => {
              const Icon = icons[line.kind];
              const text =
                line.kind === "edit" || line.kind === "read"
                  ? tailPath(relativePath(line.text, thread.cwd), pathChars)
                  : line.text;
              return (
                <li key={line.key} className={cn("min-w-0", line.pending && "animate-pulse")}>
                  {text && (
                    <div
                      className={cn(
                        "flex min-w-0 items-center gap-1.5",
                        line.failed ? "text-destructive" : "text-foreground",
                      )}
                    >
                      <Icon className="size-3 flex-none text-muted-foreground" />
                      <span className="truncate">{text}</span>
                    </div>
                  )}
                  {line.detail && (
                    <pre
                      className={cn(
                        "overflow-hidden font-mono text-[11px] leading-snug whitespace-pre-wrap wrap-anywhere text-muted-foreground",
                        // The tail is cut to its end already; clamping here would cut that end off.
                        text ? "ml-[18px]" : "font-sans text-xs text-foreground",
                      )}
                    >
                      {!text && <Icon className="mr-1.5 inline size-3 align-[-2px] text-muted-foreground" />}
                      {line.detail}
                    </pre>
                  )}
                </li>
              );
            })}
          </ol>
        )}
      </div>
    </div>
  );
}

function Tile({ thread, now, density }: { thread: Thread; now: number; density: Density }) {
  const plugins = useApp((state) => state.allPlugins);
  const project = useApp((state) => state.projects.find((candidate) => candidate.id === thread.project));
  const attention = attentionOf(thread);
  const fill = contextFill(thread);
  const spent = thread.usage ? thread.usage.spent.input_tokens + thread.usage.spent.output_tokens : 0;
  const pending = thread.pending_interactions ?? [];
  const compact = density === "compact";
  const open = () => store().select(thread.id);
  const percent = fill === null ? 0 : Math.round(fill * 100);
  const contextTitle = `Context: ${compactNumber(thread.usage?.context_used ?? 0)} of ${compactNumber(thread.usage?.context_window ?? 0)}`;

  return (
    <ThreadMenu
      thread={thread}
      render={
        <article
          className={cn(
            "flex min-w-0 cursor-pointer flex-col rounded-lg border bg-card hover:border-muted-foreground",
            compact ? "gap-1.5 p-2.5" : "gap-2 p-3",
            attention === "waiting" && "border-(--blue)",
            attention === "failed" && "border-destructive/60",
          )}
          role="button"
          tabIndex={0}
          onClick={open}
          onKeyDown={(event) => {
            if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) open();
          }}
        />
      }
    >
      <header className="flex min-w-0 flex-none items-center gap-2">
        <span className={`dot ${attention}`} title={attentionLabel[attention]} />
        <h3 className="min-w-0 flex-1 truncate text-sm font-medium" title={threadTitle(thread)}>
          {threadTitle(thread)}
        </h3>
        <span className="flex-none text-xs text-muted-foreground tabular-nums">{cardTime(thread, now)}</span>
      </header>
      <div
        className={cn(
          "flex min-w-0 flex-none items-center gap-x-2 text-xs text-muted-foreground",
          compact ? "overflow-hidden whitespace-nowrap" : "flex-wrap",
        )}
      >
        <span className="min-w-0 truncate" title={folderOf(thread)}>
          {project ? project.name : baseName(folderOf(thread))}
        </span>
        <span className="flex-none">{providerName(plugins, thread.provider)}</span>
        {thread.model && <span className={cn(compact ? "min-w-0 truncate" : "")}>{thread.model}</span>}
        {spent > 0 && (
          <span className="flex-none tabular-nums" title="Tokens in and out over all its sessions">
            {compactNumber(spent)}
            {compact ? " tok" : " tokens"}
          </span>
        )}
        {compact && fill !== null && (
          // In a compact tile the context is a short gauge at the end of the line.
          <span className="ml-auto flex flex-none items-center gap-1.5 tabular-nums" title={contextTitle}>
            <Progress value={percent} aria-label="Context used" className="w-10" />
            <span className={cn(fill > 0.85 && "text-destructive")}>{percent}%</span>
          </span>
        )}
      </div>
      {!compact && fill !== null && (
        <div
          className="flex flex-none items-center gap-2 text-xs text-muted-foreground tabular-nums"
          title={contextTitle}
        >
          <Progress value={percent} aria-label="Context used" className="flex-1" />
          <span className={cn(fill > 0.85 && "text-destructive")}>{percent}%</span>
        </div>
      )}
      <Tail thread={thread} density={density} />
      {pending.map((interaction) => (
        <InteractionPrompt
          key={interaction.id}
          threadId={thread.id}
          id={interaction.id}
          request={interaction.request}
        />
      ))}
    </ThreadMenu>
  );
}

function DensityToggle({ value, choose }: { value: Density; choose: (density: Density) => void }) {
  return (
    <ToggleGroup
      size="sm"
      variant="outline"
      spacing={0}
      aria-label="Tile size"
      className="ml-auto self-center"
      value={[value]}
      onValueChange={(values: unknown[]) => {
        if (values[0] === "compact" || values[0] === "detailed") choose(values[0]);
      }}
    >
      <ToggleGroupItem
        value="compact"
        title="Many small tiles"
        className="data-pressed:bg-accent data-pressed:text-foreground"
      >
        <LayoutGridIcon />
        Compact
      </ToggleGroupItem>
      <ToggleGroupItem
        value="detailed"
        title="Fewer, taller tiles with more of each agent's steps"
        className="data-pressed:bg-accent data-pressed:text-foreground"
      >
        <LayoutListIcon />
        Detailed
      </ToggleGroupItem>
    </ToggleGroup>
  );
}

/** Every agent at work, side by side, each showing its last steps as they happen. */
export function LiveWall() {
  const threads = useApp((state) => state.threads);
  const now = useNow(1000);
  const [density, setDensity] = useState(() => parseDensity(readText(DENSITY_KEY)));
  const choose = (next: Density) => {
    setDensity(next);
    saveText(DENSITY_KEY, next);
  };
  // The wall is chosen on a slower clock than the times on it tick: a tile does not
  // vanish under the pointer the second its recent window runs out.
  const minute = Math.floor(now / 60_000);
  const shown = useMemo(() => wallThreads(Object.values(threads), minute * 60_000 + 59_999), [threads, minute]);
  useFollowing(useMemo(() => shown.map((thread) => thread.id), [shown]));
  const working = shown.filter((thread) => thread.status === "active").length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="overview-head">
        <h1>Live</h1>
        <div className="overview-counts">
          <span className="count working">
            <strong>{working}</strong> working
          </span>
          <span className="count">
            <strong>{shown.length - working}</strong> waiting or just finished
          </span>
        </div>
        <DensityToggle value={density} choose={choose} />
      </header>
      {shown.length === 0 ? (
        <div className="empty">
          <p>No agent is at work.</p>
          <p className="hint">Agents show up here while they work, wait for you, or have just finished.</p>
        </div>
      ) : (
        // Each tile's tail has a bounded height, so a tile is only as tall as its
        // header, a few steps and an open question. The class names stay literal for
        // Tailwind to find them.
        <div
          className={cn(
            "grid min-h-0 flex-1 content-start overflow-y-auto",
            density === "compact"
              ? "grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-2 p-3"
              : "grid-cols-[repeat(auto-fill,minmax(420px,1fr))] gap-3 p-4",
          )}
        >
          {shown.map((thread) => (
            <Tile key={thread.id} thread={thread} now={now} density={density} />
          ))}
        </div>
      )}
    </div>
  );
}
