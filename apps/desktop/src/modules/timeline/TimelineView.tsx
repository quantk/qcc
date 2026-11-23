import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, GitForkIcon } from "lucide-react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { Item, Thread } from "../../api/protocol.gen";
import { type Timing, turnTimings } from "../../state/durations";
import { estimateEntryHeight, newTurnKey, turnRoom } from "../../state/scroll";
import { store, useApp } from "../../state/store";
import { type Entry, type Timeline, conversationStart, earlierConversations, forkedEntries } from "../../state/timeline";
import {
  DataTrace,
  InteractionOutcome,
  InteractionPrompt,
  KeepInPlace,
  MessageAttachments,
  type Place,
  Remembered,
} from "../../ui/kit";
import { itemKey, useUi } from "../../ui/registry";
import { deferScrollWrites } from "./deferScrollWrites";
import { QuoteSelection } from "./QuoteSelection";
import { Bubble, BubbleContent } from "@/components/ui/bubble";
import { Button } from "@/components/ui/button";
import { Message, MessageContent, MessageFooter } from "@/components/ui/message";
import { cn } from "@/lib/utils";

/** An item nobody registered a renderer for: shown as the data it is. */
function UnknownItem({ item, timing }: { item: Item; timing: Timing }) {
  return (
    <DataTrace
      title={item.type === "extension" ? item.kind : item.type}
      status={item.status}
      value={item}
      timing={timing}
    />
  );
}

function ItemView({ entry, thread, turn }: { entry: Extract<Entry, { kind: "item" }>; thread: Thread; turn?: Timing }) {
  const { item } = entry;
  const Renderer = useUi((snapshot) => snapshot.items.get(itemKey(item)));
  // Rebuilt only when the entry changes, which the memoised row needs anyway.
  const timing = useMemo(() => ({ startedAt: entry.ts, endedAt: entry.endTs }), [entry.ts, entry.endTs]);
  return Renderer ? (
    <Renderer item={item} thread={thread} timing={timing} turn={turn} />
  ) : (
    <UnknownItem item={item} timing={timing} />
  );
}

function InteractionView({ thread, entry }: { thread: Thread; entry: Extract<Entry, { kind: "interaction" }> }) {
  const { request, resolution } = entry;
  if (!resolution) return <InteractionPrompt threadId={thread.id} id={entry.id} request={request} />;
  return <InteractionOutcome request={request} resolution={resolution} />;
}

const EntryView = memo(function EntryView({ thread, entry, turn }: { thread: Thread; entry: Entry; turn?: Timing }) {
  switch (entry.kind) {
    case "user":
      return (
        <Message align="end">
          <MessageContent className="gap-1">
            {entry.text.trim() !== "" && (
              <Bubble
                align="end"
                variant="muted"
                className="*:data-[slot=bubble-content]:bg-accent"
              >
                <BubbleContent className="whitespace-pre-wrap">{entry.text}</BubbleContent>
              </Bubble>
            )}
            <MessageAttachments threadId={thread.id} attachments={entry.attachments} />
            {entry.notes.map((note, index) => (
              <MessageFooter
                key={index}
                className="block whitespace-pre-wrap"
                title="What qcc told the agent along with this message"
              >
                qcc: {note}
              </MessageFooter>
            ))}
            {entry.steered && <MessageFooter>steered into the running turn</MessageFooter>}
          </MessageContent>
        </Message>
      );
    case "item":
      return <ItemView entry={entry} thread={thread} turn={turn} />;
    case "interaction":
      return <InteractionView thread={thread} entry={entry} />;
    case "notice":
      return <div className={`notice ${entry.level}`}>{entry.text}</div>;
    case "conversation":
      return (
        <div className="flex items-center gap-3 py-2 text-xs text-muted-foreground" role="separator">
          <span className="h-px flex-1 bg-border" />
          A new conversation
          <span className="h-px flex-1 bg-border" />
        </div>
      );
  }
});

/**
 * One row of the list: an entry (of the thread a fork came from, for the
 * conversation before the fork), the button that shows earlier conversations, or
 * the line that says where a fork came from.
 */
type Row =
  | { key: string; entry: Entry; source?: Thread }
  | { key: "earlier"; entry: null }
  | { key: "forked"; entry: null };

/** What a fork came from, read once its conversation from there is asked for. */
type Origin = { source: Thread; timeline: Timeline };

/** The thread a fork came from and its conversation up to the fork; read when `wanted`. */
function useForkOrigin(thread: Thread, wanted: boolean): Origin | null {
  const [origin, setOrigin] = useState<{ fork: string; origin: Origin } | null>(null);
  const read = wanted && thread.forked_from !== undefined && thread.forked_from !== null && origin?.fork !== thread.id;
  useEffect(() => {
    if (!read) return;
    let current = true;
    void store()
      .forkOrigin(thread)
      .then((found) => current && found && setOrigin({ fork: thread.id, origin: found }));
    return () => {
      current = false;
    };
  }, [read, thread]);
  // Its timeline is kept current while shown, for the menus under its replies.
  const source = origin?.fork === thread.id ? origin.origin.source.id : null;
  useEffect(() => (wanted && source ? store().hold(source) : undefined), [wanted, source]);
  return origin?.fork === thread.id ? origin.origin : null;
}

/** The line at the top of a fork: where it came from, and its conversation from there on request. */
function ForkedFrom(props: { thread: Thread; origin: Origin | null; shown: boolean; toggle: () => void }) {
  const { thread, origin, shown } = props;
  const from = thread.forked_from;
  const listed = useApp((state) => (from ? state.threads[from.thread] : undefined));
  const title = origin?.source.title ?? listed?.title ?? "another thread";
  return (
    <div className="flex min-w-0 items-center gap-2 py-1 text-xs text-muted-foreground" role="separator">
      <span className="h-px min-w-6 flex-1 bg-border" />
      <GitForkIcon className="size-3.5 shrink-0" />
      <span className="shrink-0">Forked from</span>
      <Button
        variant="link"
        size="xs"
        className="h-auto min-w-0 shrink px-0 text-xs text-muted-foreground hover:text-foreground"
        title={title}
        onClick={() => from && store().select(from.thread)}
      >
        <span className="truncate">{title}</span>
      </Button>
      <span aria-hidden>·</span>
      <Button variant="ghost" size="xs" className="shrink-0" onClick={props.toggle}>
        {shown ? "Hide the conversation before" : "Show the conversation before"}
      </Button>
      <span className="h-px min-w-6 flex-1 bg-border" />
    </div>
  );
}

/** Space above the first row, between rows and below the last one. */
const PADDING_START = 20;
const GAP = 10;
const PADDING_END = 28;
/** How much of the entry above a new turn's message stays in view. */
const PEEK = 64;
/** How far from the end the view may be and still count as at the end. A touchpad
 *  scrolls a few pixels at a time, so with more its first steps up were taken back
 *  by the next streamed text. */
const END_THRESHOLD = 1;
/** Keys that scroll the view; pressing one is the reader taking over. */
const SCROLL_KEYS = new Set(["ArrowDown", "ArrowUp", "End", "Home", "PageDown", "PageUp", " "]);

/**
 * The conversation. Only what is in view (and a few entries around it) is drawn, so a
 * long thread costs no more to stream into than a short one. It follows the reply
 * while the reader is at the end, keeps their place when they scroll up, and brings
 * each new message of the user near the top with the answer under it.
 */
export function TimelineView({ thread, timeline }: { thread: Thread; timeline: Timeline }) {
  // Earlier conversations stay in the journal; they are shown only when asked for.
  // Asked for one thread, not for the next one shown in the same view.
  const [shownFor, setShownFor] = useState<string | null>(null);
  const start = shownFor === thread.id ? 0 : conversationStart(timeline.entries);
  const earlier = earlierConversations(timeline.entries);
  // Queued messages are listed above the composer, where they can be changed.
  const entries = useMemo(
    () => timeline.entries.slice(start).filter((entry) => !(entry.kind === "user" && entry.queued)),
    [timeline.entries, start],
  );
  // A fork's conversation from before it is the thread's it came from, shown on request.
  const [originFor, setOriginFor] = useState<string | null>(null);
  const originShown = originFor === thread.id;
  const origin = useForkOrigin(thread, originShown);
  const rows = useMemo<Row[]>(() => {
    const list: Row[] = entries.map((entry) => ({ key: entry.key, entry }));
    const own: Row[] = start > 0 ? [{ key: "earlier", entry: null }, ...list] : list;
    if (!thread.forked_from) return own;
    const before: Row[] =
      originShown && origin
        ? forkedEntries(origin.timeline).map((entry) => ({ key: `origin:${entry.key}`, entry, source: origin.source }))
        : [];
    return [{ key: "forked", entry: null }, ...before, ...own];
  }, [entries, start, thread.forked_from, originShown, origin]);
  // How long each ended turn took, shown under its last reply; the fork's origin
  // has its own turns.
  const { turns, stats } = timeline;
  const turnsHere = useMemo(() => turnTimings(turns, stats.turns), [turns, stats.turns]);
  const turnsThere = useMemo(
    () => (origin ? turnTimings(origin.timeline.turns, origin.timeline.stats.turns) : null),
    [origin],
  );

  const viewportRef = useRef<HTMLDivElement>(null);
  const frameRef = useRef<HTMLDivElement>(null);
  // What the user opened in entries that scrolled out, for when they come back.
  const [memory] = useState(() => new Map<string, unknown>());

  // The view can be held so that one entry stays `offset` pixels below its top
  // while what is around it is measured, until the reader scrolls: a new turn holds
  // its message near the top, and showing earlier conversations holds what was in
  // view where it was.
  const hold = useRef<{ key: string; offset: number } | null>(null);
  // The user's message that starts the turn under way, while it has room below it.
  const turn = useRef<string | null>(null);
  // The reader opened or closed something in view: the view keeps still around it
  // instead of following the end, until they scroll or send the next message.
  // Following the end moved what they clicked up by all that it unfolded.
  const pinned = useRef(false);
  const [, setPinnedAt] = useState(0);
  const previousEntries = useRef(entries);
  if (previousEntries.current !== entries) {
    const key = newTurnKey(previousEntries.current, entries);
    if (key !== null) {
      turn.current = key;
      hold.current = { key, offset: PEEK };
      pinned.current = false;
    }
    previousEntries.current = entries;
  }
  // The room below the content that lets the turn's message reach the top; worked
  // out after each render from what was measured, and drawn on the next.
  const [room, setRoom] = useState(0);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewportRef.current,
    estimateSize: (index) => {
      const row = rows[index]!;
      return row.entry ? estimateEntryHeight(row.entry) : 32;
    },
    getItemKey: (index) => rows[index]!.key,
    // Open in place at the end: what is drawn first is what stays in view.
    initialOffset: () => rows.reduce((sum, row) => sum + (row.entry ? estimateEntryHeight(row.entry) : 32) + GAP, 0),
    paddingStart: PADDING_START,
    paddingEnd: PADDING_END,
    gap: GAP,
    overscan: 6,
    // While a new turn has room below it, or the reader has pinned the view, it stays
    // where it is; otherwise it sticks to the end while it is there.
    anchorTo: room > 0 || pinned.current ? "start" : "end",
    followOnAppend: true,
    scrollEndThreshold: END_THRESHOLD,
    // A question the agent waits on keeps what was typed into it while out of view.
    rangeExtractor: (range) => {
      const indexes = new Set<number>();
      for (let index = Math.max(0, range.startIndex - range.overscan); index <= Math.min(range.count - 1, range.endIndex + range.overscan); index++) {
        indexes.add(index);
      }
      rows.forEach((row, index) => {
        if (row.entry?.kind === "interaction" && !row.entry.resolution) indexes.add(index);
      });
      return [...indexes].sort((a, b) => a - b);
    },
  });

  const contentEnd = virtualizer.getTotalSize();
  const viewportHeight = virtualizer.scrollRect?.height ?? 0;
  const startOf = (key: string): number | undefined =>
    virtualizer.measurementsCache[rows.findIndex((row) => row.key === key)]?.start;
  const anchorStart = turn.current === null ? undefined : startOf(turn.current);
  const nextRoom =
    anchorStart === undefined || viewportHeight === 0
      ? 0
      : turnRoom({ anchorStart, peek: PEEK, contentEnd, viewportHeight });

  // Open at the end.
  useLayoutEffect(() => {
    virtualizer.scrollToEnd();
  }, [virtualizer]);

  // Settle the room, and keep a held entry where it is held.
  useLayoutEffect(() => {
    if (nextRoom !== room) setRoom(nextRoom);
    const viewport = viewportRef.current;
    if (!viewport) return;
    if (turn.current !== null && nextRoom === 0) {
      // The answer fills the view: from here on it is followed like any other.
      if (hold.current?.key === turn.current) {
        hold.current = null;
        if (anchorStart !== undefined) virtualizer.scrollToEnd();
      }
      turn.current = null;
    }
    // Hold once the room for it is drawn.
    if (!hold.current || nextRoom !== room) return;
    const start = startOf(hold.current.key);
    if (start === undefined) {
      hold.current = null;
      return;
    }
    // Written straight to the element: the virtualizer's own scrolling would keep
    // to the offset it was given and undo how it makes up for the guessed heights
    // of entries above as they are measured.
    const top = Math.max(0, start - hold.current.offset);
    if (Math.abs(viewport.scrollTop - top) > 0.5) viewport.scrollTop = top;
  });

  // Whether the content goes on below the view, for the button that goes there.
  const [below, setBelow] = useState(false);
  const measureBelow = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const distance = contentEnd - viewport.clientHeight - viewport.scrollTop;
    setBelow(distance > Math.max(END_THRESHOLD, 8));
  }, [contentEnd]);
  useLayoutEffect(measureBelow);

  // Showing earlier conversations puts them above what is in view; the entry that
  // was first stays where it was on the screen.
  const showEarlier = () => {
    const after = rows.findIndex((row) => row.key === "earlier") + 1;
    const first = rows[after];
    const viewport = viewportRef.current;
    const firstStart = virtualizer.measurementsCache[after]?.start;
    hold.current =
      first && viewport && firstStart !== undefined ? { key: first.key, offset: firstStart - viewport.scrollTop } : null;
    setShownFor(thread.id);
  };

  // The reader scrolls: from their first wheel step, key or touch until the
  // scrolling it set off has stopped. Their scrolling ends any hold, and the
  // virtualizer leaves the scroll position to them meanwhile.
  const gesture = useRef({ on: false, at: 0 });
  const unpin = useCallback(() => {
    if (!pinned.current) return;
    pinned.current = false;
    setPinnedAt(performance.now());
  }, []);
  const scrolling = useCallback(() => {
    hold.current = null;
    unpin();
    gesture.current = { on: true, at: performance.now() };
  }, [unpin]);
  const place = useMemo<Place>(
    () => ({
      viewport: () => viewportRef.current,
      pin: () => {
        hold.current = null;
        if (pinned.current) return;
        pinned.current = true;
        setPinnedAt(performance.now());
      },
    }),
    [],
  );
  useLayoutEffect(() => {
    deferScrollWrites(virtualizer, () => {
      const current = gesture.current;
      // The first scroll event comes a little after the wheel event.
      if (current.on && !virtualizer.isScrolling && performance.now() - current.at > 100) current.on = false;
      return current.on;
    });
  }, [virtualizer]);

  const toEnd = () => {
    hold.current = null;
    unpin();
    const viewport = viewportRef.current;
    const far = viewport ? contentEnd - viewport.scrollTop > 3 * viewport.clientHeight : true;
    // A long way down smooth scrolling would walk through guessed heights; jump.
    virtualizer.scrollToEnd({ behavior: far ? "auto" : "smooth" });
  };

  const items = virtualizer.getVirtualItems();
  return (
    <div ref={frameRef} className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
      <div
        ref={viewportRef}
        role="region"
        aria-label="Timeline"
        tabIndex={0}
        className="size-full min-h-0 min-w-0 scrollbar-thin scrollbar-gutter-stable overflow-y-auto overscroll-contain contain-content"
        onScroll={measureBelow}
        onWheel={scrolling}
        onTouchMove={scrolling}
        onKeyDown={(event) => SCROLL_KEYS.has(event.key) && scrolling()}
        // Dragging the scrollbar: the press lands on the viewport itself.
        onPointerDown={(event) => event.target === event.currentTarget && scrolling()}
      >
        {rows.length === 0 ? (
          <div className="mx-auto w-full max-w-[860px] px-6 pt-5">
            <p className="hint">Nothing here yet. Send the first message below.</p>
          </div>
        ) : (
          <div className="mx-auto w-full max-w-[860px] px-6">
            <div role="log" aria-relevant="additions" className="relative w-full" style={{ height: contentEnd }}>
              {items.map((virtual) => {
                const row = rows[virtual.index]!;
                return (
                  <div
                    key={virtual.key}
                    data-index={virtual.index}
                    ref={virtualizer.measureElement}
                    className="absolute top-0 left-0 flex w-full min-w-0 flex-col"
                    style={{ transform: `translateY(${virtual.start}px)` }}
                  >
                    {row.entry ? (
                      <Remembered memory={memory} entryKey={row.key}>
                        <KeepInPlace place={place}>
                          <div className={cn("flex min-w-0 flex-col", row.source && "opacity-75")}>
                            <EntryView
                              thread={row.source ?? thread}
                              entry={row.entry}
                              turn={
                                row.entry.kind === "item"
                                  ? (row.source ? turnsThere : turnsHere)?.get(row.entry.item.id)
                                  : undefined
                              }
                            />
                          </div>
                        </KeepInPlace>
                      </Remembered>
                    ) : row.key === "forked" ? (
                      <ForkedFrom
                        thread={thread}
                        origin={origin}
                        shown={originShown}
                        toggle={() => setOriginFor(originShown ? null : thread.id)}
                      />
                    ) : (
                      <Button variant="ghost" size="sm" className="self-center" onClick={showEarlier}>
                        Show {earlier === 1 ? "the earlier conversation" : `${earlier} earlier conversations`}
                      </Button>
                    )}
                  </div>
                );
              })}
            </div>
            {room > 0 && <div aria-hidden style={{ height: room }} />}
          </div>
        )}
      </div>
      <Button
        variant="secondary"
        size="icon-sm"
        className={cn(
          "absolute bottom-4 left-1/2 -translate-x-1/2 border-border bg-background text-foreground transition-[translate,scale,opacity] duration-200 hover:bg-muted hover:text-foreground",
          below ? "translate-y-0 scale-100 opacity-100" : "pointer-events-none translate-y-full scale-95 opacity-0",
        )}
        tabIndex={below ? 0 : -1}
        aria-hidden={!below}
        onClick={toEnd}
      >
        <ArrowDownIcon />
        <span className="sr-only">Scroll to end</span>
      </Button>
      <QuoteSelection threadId={thread.id} viewport={viewportRef} frame={frameRef} />
    </div>
  );
}
