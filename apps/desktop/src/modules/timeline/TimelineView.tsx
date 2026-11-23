import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon } from "lucide-react";
import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { Item, Thread } from "../../api/protocol.gen";
import { estimateEntryHeight, newTurnKey, turnRoom } from "../../state/scroll";
import { type Entry, type Timeline, conversationStart, earlierConversations } from "../../state/timeline";
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
function UnknownItem({ item }: { item: Item }) {
  return <DataTrace title={item.type === "extension" ? item.kind : item.type} status={item.status} value={item} />;
}

function ItemView({ item, thread }: { item: Item; thread: Thread }) {
  const Renderer = useUi((snapshot) => snapshot.items.get(itemKey(item)));
  return Renderer ? <Renderer item={item} thread={thread} /> : <UnknownItem item={item} />;
}

function InteractionView({ thread, entry }: { thread: Thread; entry: Extract<Entry, { kind: "interaction" }> }) {
  const { request, resolution } = entry;
  if (!resolution) return <InteractionPrompt threadId={thread.id} id={entry.id} request={request} />;
  return <InteractionOutcome request={request} resolution={resolution} />;
}

const EntryView = memo(function EntryView({ thread, entry }: { thread: Thread; entry: Entry }) {
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
      return <ItemView item={entry.item} thread={thread} />;
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

/** One row of the list: an entry, or the button that shows earlier conversations. */
type Row = { key: string; entry: Entry } | { key: "earlier"; entry: null };

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
  const rows = useMemo<Row[]>(() => {
    const list: Row[] = entries.map((entry) => ({ key: entry.key, entry }));
    return start > 0 ? [{ key: "earlier", entry: null }, ...list] : list;
  }, [entries, start]);

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
    const first = rows[1];
    const viewport = viewportRef.current;
    const firstStart = virtualizer.measurementsCache[1]?.start;
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
                          <EntryView thread={thread} entry={row.entry} />
                        </KeepInPlace>
                      </Remembered>
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
