import {
  BrainIcon,
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
import { useEffect, useMemo, useRef } from "react";

import type { Thread, ThreadId } from "../../api/protocol.gen";
import { type TailLine, contextFill, tailOf, wallThreads } from "../../state/live";
import { attentionLabel, attentionOf, cardTime, folderOf, providerName, threadTitle } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { InteractionPrompt, ThreadMenu } from "../../ui/kit";
import { baseName, compactNumber, relativePath, tailPath } from "../../util/format";
import { useNow } from "../../util/useNow";
import { Progress } from "@/components/ui/progress";
import { cn } from "@/lib/utils";

/** Steps shown under each agent. */
const TAIL = 8;

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

function Tail({ thread }: { thread: Thread }) {
  const entries = useApp((state) => state.timelines[thread.id]?.entries);
  const lines = useMemo(() => tailOf(entries ?? [], TAIL), [entries]);
  if (!entries) return <p className="text-xs text-muted-foreground">Reading…</p>;
  if (lines.length === 0) return <p className="text-xs text-muted-foreground">Nothing yet.</p>;
  return (
    <ol className="flex min-w-0 flex-col gap-1 font-mono text-xs">
      {lines.map((line) => {
        const Icon = icons[line.kind];
        const text = line.kind === "edit" || line.kind === "read" ? tailPath(relativePath(line.text, thread.cwd), 60) : line.text;
        return (
          <li key={line.key} className={cn("min-w-0", line.pending && "animate-pulse")}>
            {text && (
              <div className={cn("flex min-w-0 items-center gap-1.5", line.failed ? "text-destructive" : "text-foreground")}>
                <Icon className="size-3 flex-none text-muted-foreground" />
                <span className="truncate">{text}</span>
              </div>
            )}
            {line.detail && (
              <pre
                className={cn(
                  "overflow-hidden font-mono text-[11px] leading-snug whitespace-pre-wrap text-muted-foreground",
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
  );
}

function Tile({ thread, now }: { thread: Thread; now: number }) {
  const plugins = useApp((state) => state.allPlugins);
  const project = useApp((state) => state.projects.find((candidate) => candidate.id === thread.project));
  const attention = attentionOf(thread);
  const fill = contextFill(thread);
  const spent = thread.usage ? thread.usage.spent.input_tokens + thread.usage.spent.output_tokens : 0;
  const open = () => store().select(thread.id);

  return (
    <ThreadMenu
      thread={thread}
      render={
        <article
          className={cn(
            "flex min-w-0 cursor-pointer flex-col gap-2 rounded-lg border bg-card p-3 hover:border-muted-foreground",
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
      <header className="flex min-w-0 items-center gap-2">
        <span className={`dot ${attention}`} title={attentionLabel[attention]} />
        <h3 className="min-w-0 flex-1 truncate text-sm font-medium">{threadTitle(thread)}</h3>
        <span className="flex-none text-xs text-muted-foreground tabular-nums">{cardTime(thread, now)}</span>
      </header>
      <div className="flex min-w-0 flex-wrap gap-x-2 text-xs text-muted-foreground">
        <span className="truncate" title={folderOf(thread)}>
          {project ? project.name : baseName(folderOf(thread))}
        </span>
        <span>{providerName(plugins, thread.provider)}</span>
        {thread.model && <span>{thread.model}</span>}
        {spent > 0 && <span title="Tokens in and out over all its sessions">{compactNumber(spent)} tokens</span>}
      </div>
      {fill !== null && (
        <div
          className="flex items-center gap-2 text-xs text-muted-foreground tabular-nums"
          title={`Context: ${compactNumber(thread.usage?.context_used ?? 0)} of ${compactNumber(thread.usage?.context_window ?? 0)}`}
        >
          <Progress value={Math.round(fill * 100)} aria-label="Context used" className="flex-1" />
          <span className={cn(fill > 0.85 && "text-destructive")}>{Math.round(fill * 100)}%</span>
        </div>
      )}
      <div className="min-h-0 border-t pt-2">
        <Tail thread={thread} />
      </div>
      {(thread.pending_interactions ?? []).map((pending) => (
        <InteractionPrompt key={pending.id} threadId={thread.id} id={pending.id} request={pending.request} />
      ))}
    </ThreadMenu>
  );
}

/** Every agent at work, side by side, each showing its last steps as they happen. */
export function LiveWall() {
  const threads = useApp((state) => state.threads);
  const now = useNow(1000);
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
      </header>
      {shown.length === 0 ? (
        <div className="empty">
          <p>No agent is at work.</p>
          <p className="hint">Agents show up here while they work, wait for you, or have just finished.</p>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-[repeat(auto-fill,minmax(360px,1fr))] gap-3 overflow-y-auto p-4">
          {shown.map((thread) => (
            <Tile key={thread.id} thread={thread} now={now} />
          ))}
        </div>
      )}
    </div>
  );
}
