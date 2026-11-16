import { Virtualizer } from "@tanstack/react-virtual";
import { describe, expect, it, vi } from "vitest";

import { deferScrollWrites } from "./deferScrollWrites";

// `deferScrollWrites` replaces a private method of the virtualizer. If an update of
// @tanstack/react-virtual renames it or stops consulting it, this fails, rather than
// scrolling up a long timeline quietly losing half of each wheel step again.
describe("deferScrollWrites", () => {
  function virtualizer() {
    const element = { scrollTop: 0, scrollHeight: 1000, clientHeight: 100 } as unknown as HTMLElement;
    const scrollToFn = vi.fn();
    const instance = new Virtualizer<HTMLElement, Element>({
      count: 10,
      getScrollElement: () => element,
      estimateSize: () => 50,
      scrollToFn,
      observeElementRect: () => undefined,
      observeElementOffset: () => undefined,
    });
    return { instance, scrollToFn };
  }

  it("holds a correction while busy and writes it otherwise", () => {
    const { instance, scrollToFn } = virtualizer();
    let busy = true;
    deferScrollWrites(instance, () => busy);
    // Private too: the correction the virtualizer applies when the size of an entry
    // above the view turns out other than guessed.
    const internals = instance as unknown as { _holdOffset: number; applyScrollAdjustment: (delta: number) => void };

    internals.applyScrollAdjustment(12);
    expect(scrollToFn).not.toHaveBeenCalled();
    expect(internals._holdOffset).toBe(12);

    busy = false;
    internals.applyScrollAdjustment(5);
    expect(scrollToFn).toHaveBeenCalled();
  });
});
