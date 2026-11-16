import { memo, useState } from "react";

import type { Item, Thread } from "../../api/protocol.gen";
import { type Entry, type Timeline, conversationStart, earlierConversations } from "../../state/timeline";
import { DataTrace, InteractionOutcome, InteractionPrompt } from "../../ui/kit";
import { itemKey, useUi } from "../../ui/registry";
import { Bubble, BubbleContent } from "@/components/ui/bubble";
import { Button } from "@/components/ui/button";
import { Message, MessageContent, MessageFooter } from "@/components/ui/message";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/components/ui/message-scroller";

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
            <Bubble
              align="end"
              variant="muted"
              className="*:data-[slot=bubble-content]:bg-accent"
            >
              <BubbleContent className="whitespace-pre-wrap">{entry.text}</BubbleContent>
            </Bubble>
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

/**
 * The conversation, on the Message Scroller of AI Elements' Chat: it follows the reply
 * while the reader is at the end, keeps their place when they scroll up, and anchors each
 * message of the user so a new turn settles near the top with the answer under it.
 */
export function TimelineView({ thread, timeline }: { thread: Thread; timeline: Timeline }) {
  // Earlier conversations stay in the journal; they are shown only when asked for.
  // Asked for one thread, not for the next one shown in the same view.
  const [shownFor, setShownFor] = useState<string | null>(null);
  const start = shownFor === thread.id ? 0 : conversationStart(timeline.entries);
  const earlier = earlierConversations(timeline.entries);
  const entries = timeline.entries.slice(start);
  return (
    // The scroller goes back to following the end whenever the view is within the
    // threshold of it. A touchpad scrolls a few pixels at a time, so with the
    // default 8 px its first steps up were taken back by the next streamed text.
    <MessageScrollerProvider autoScroll defaultScrollPosition="end" scrollEdgeThreshold={1}>
      <MessageScroller className="h-auto flex-1">
        <MessageScrollerViewport aria-label="Timeline">
          <MessageScrollerContent className="mx-auto w-full max-w-[860px] gap-2.5 px-6 pt-5 pb-7">
            {start > 0 && (
              <Button variant="ghost" size="sm" className="self-center" onClick={() => setShownFor(thread.id)}>
                Show {earlier === 1 ? "the earlier conversation" : `${earlier} earlier conversations`}
              </Button>
            )}
            {entries.length === 0 && <p className="hint">Nothing here yet. Send the first message below.</p>}
            {/* Queued messages are listed above the composer, where they can be changed. */}
            {entries.filter((entry) => !(entry.kind === "user" && entry.queued)).map((entry) => (
              <MessageScrollerItem key={entry.key} messageId={entry.key} scrollAnchor={entry.kind === "user"}>
                <EntryView thread={thread} entry={entry} />
              </MessageScrollerItem>
            ))}
          </MessageScrollerContent>
        </MessageScrollerViewport>
        <MessageScrollerButton />
      </MessageScroller>
    </MessageScrollerProvider>
  );
}
