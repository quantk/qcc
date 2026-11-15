// The conversation: the timeline of a thread, its composer, and how each built-in
// kind of item is drawn.

import type { UiModule } from "../../ui/registry";
import { Composer } from "./Composer";
import { CORE_ITEM_TYPES, CoreItem } from "./items";
import { TimelineView } from "./TimelineView";

export const timelineModule: UiModule = {
  id: "core.timeline",
  activate(ui) {
    ui.threadTab({
      id: "timeline",
      title: "Timeline",
      order: 0,
      render: ({ thread, timeline, provider }) =>
        timeline ? (
          <>
            <TimelineView thread={thread} timeline={timeline} />
            <Composer thread={thread} timeline={timeline} provider={provider} />
          </>
        ) : (
          <div className="empty">Loading…</div>
        ),
    });
    for (const type of CORE_ITEM_TYPES) ui.itemRenderer(type, CoreItem);
  },
};
