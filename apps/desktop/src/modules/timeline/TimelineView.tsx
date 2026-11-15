import { memo, useLayoutEffect, useRef } from "react";

import type { Item, Thread } from "../../api/protocol.gen";
import type { Entry, Timeline } from "../../state/timeline";
import { DataTrace, InteractionOutcome, InteractionPrompt } from "../../ui/kit";
import { itemKey, useUi } from "../../ui/registry";

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
        <div className={`message user${entry.queued ? " queued" : ""}`}>
          <div className="user-text">{entry.text}</div>
          {(entry.queued || entry.steered) && <div className="user-note">{entry.queued ? "queued" : "steered"}</div>}
        </div>
      );
    case "item":
      return <ItemView item={entry.item} thread={thread} />;
    case "interaction":
      return <InteractionView thread={thread} entry={entry} />;
    case "notice":
      return <div className={`notice ${entry.level}`}>{entry.text}</div>;
  }
});

export function TimelineView({ thread, timeline }: { thread: Thread; timeline: Timeline }) {
  const scroller = useRef<HTMLDivElement>(null);
  // Follow new output only while the reader is at the bottom.
  const stick = useRef(true);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element && stick.current) element.scrollTop = element.scrollHeight;
  }, [timeline]);

  return (
    <div
      className="timeline"
      ref={scroller}
      onScroll={(event) => {
        const element = event.currentTarget;
        stick.current = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      }}
    >
      <div className="timeline-inner">
        {timeline.entries.length === 0 && <p className="hint">Nothing here yet. Send the first message below.</p>}
        {timeline.entries.map((entry) => (
          <EntryView key={entry.key} thread={thread} entry={entry} />
        ))}
      </div>
    </div>
  );
}
