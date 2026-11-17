import { describe, expect, it } from "vitest";

import { type BlockInfo, type Layout, arrange, blocksIn, dropIndex, fits, moveBlock, parseLayout, resetBlock } from "./blocks";

const blocks: BlockInfo[] = [
  { id: "overview", scope: "app", placement: { slot: "views", order: 0 } },
  { id: "live", scope: "app", placement: { slot: "views", order: 5 } },
  { id: "timeline", scope: "thread", placement: { slot: "thread.tabs", order: 0 } },
  { id: "changes", scope: "thread", placement: { slot: "thread.tabs", order: 10 } },
  { id: "usage", scope: "thread", placement: { slot: "thread.panel", order: 0 } },
  { id: "details", scope: "thread", placement: { slot: "thread.panel", order: 20 } },
  { id: "extra", scope: "app", placement: { slot: null, order: 0 } },
];

const ids = (list: readonly BlockInfo[]) => list.map((block) => block.id);

describe("block layout", () => {
  it("puts every block in its default place without a layout", () => {
    const { slots, hidden } = arrange(blocks, {});
    expect(ids(slots.views)).toEqual(["overview", "live"]);
    expect(ids(slots["thread.tabs"])).toEqual(["timeline", "changes"]);
    expect(ids(slots["thread.panel"])).toEqual(["usage", "details"]);
    expect(ids(slots.sidebar)).toEqual([]);
    expect(ids(hidden)).toEqual(["extra"]);
  });

  it("lets a block that needs nothing go anywhere, and a thread's block only beside a thread", () => {
    expect(fits("app", "thread.panel")).toBe(true);
    expect(fits("app", "sidebar")).toBe(true);
    expect(fits("thread", "thread.tabs")).toBe(true);
    expect(fits("thread", "views")).toBe(false);
    expect(fits("thread", "sidebar")).toBe(false);
  });

  it("moves a block to another slot at a position and keeps the order written down", () => {
    const layout = moveBlock(blocks, {}, "changes", "thread.panel", 1);
    expect(ids(blocksIn(blocks, layout, "thread.panel"))).toEqual(["usage", "changes", "details"]);
    expect(ids(blocksIn(blocks, layout, "thread.tabs"))).toEqual(["timeline"]);
  });

  it("reorders within a slot", () => {
    const layout = moveBlock(blocks, {}, "live", "views", 0);
    expect(ids(blocksIn(blocks, layout, "views"))).toEqual(["live", "overview"]);
    const back = moveBlock(blocks, layout, "live", "views", 5);
    expect(ids(blocksIn(blocks, back, "views"))).toEqual(["overview", "live"]);
  });

  it("appends when no position is given", () => {
    const layout = moveBlock(blocks, {}, "overview", "thread.tabs");
    expect(ids(blocksIn(blocks, layout, "thread.tabs"))).toEqual(["timeline", "changes", "overview"]);
  });

  it("hides a block and shows it again", () => {
    const hidden = moveBlock(blocks, {}, "live", null);
    expect(ids(arrange(blocks, hidden).hidden)).toEqual(["extra", "live"]);
    const shown = moveBlock(blocks, hidden, "extra", "sidebar");
    expect(ids(blocksIn(blocks, shown, "sidebar"))).toEqual(["extra"]);
  });

  it("refuses to put a thread's block where there is no thread", () => {
    const layout: Layout = {};
    expect(moveBlock(blocks, layout, "timeline", "views", 0)).toBe(layout);
    expect(moveBlock(blocks, layout, "unknown", "views", 0)).toBe(layout);
  });

  it("ignores a stored place that no longer fits the block", () => {
    const layout: Layout = { timeline: { slot: "sidebar", order: 0 } };
    expect(ids(blocksIn(blocks, layout, "thread.tabs"))).toEqual(["timeline", "changes"]);
  });

  it("stores only what differs from the defaults and keeps places of blocks not here now", () => {
    const layout = moveBlock(blocks, { "plugin/board": { slot: "sidebar", order: 0 } }, "live", "views", 1);
    // `overview` stays at its default order, so nothing is written for it.
    expect(layout).toEqual({ "plugin/board": { slot: "sidebar", order: 0 }, live: { slot: "views", order: 10 } });
    const later = [...blocks, { id: "plugin/board", scope: "app", placement: { slot: "views", order: 100 } } as BlockInfo];
    expect(ids(blocksIn(later, layout, "sidebar"))).toEqual(["plugin/board"]);
  });

  it("puts a block that arrives later at its default place among the user's order", () => {
    const layout = moveBlock(blocks, {}, "live", "views", 0);
    const later = [...blocks, { id: "metrics", scope: "app", placement: { slot: "views", order: 20 } } as BlockInfo];
    expect(ids(blocksIn(later, layout, "views"))).toEqual(["live", "overview", "metrics"]);
  });

  it("resets one block", () => {
    const layout = moveBlock(blocks, {}, "usage", null);
    expect(ids(blocksIn(blocks, resetBlock(layout, "usage"), "thread.panel"))).toEqual(["usage", "details"]);
  });

  it("reads back only well-formed entries", () => {
    expect(parseLayout(null)).toEqual({});
    expect(parseLayout([1, 2])).toEqual({});
    expect(
      parseLayout({
        a: { slot: "sidebar", order: 3 },
        b: { slot: null, order: 0 },
        c: { slot: "nowhere", order: 0 },
        d: { slot: "views" },
        e: "views",
      }),
    ).toEqual({ a: { slot: "sidebar", order: 3 }, b: { slot: null, order: 0 } });
  });

  it("finds where a dragged block lands among the others", () => {
    expect(dropIndex([10, 30, 50], 5)).toBe(0);
    expect(dropIndex([10, 30, 50], 31)).toBe(2);
    expect(dropIndex([10, 30, 50], 99)).toBe(3);
    expect(dropIndex([], 99)).toBe(0);
  });
});
