import { QuoteIcon } from "lucide-react";
import { type RefObject, useCallback, useEffect, useRef, useState } from "react";

import { appendQuote } from "../../state/quote";
import { changeComposer } from "./composerInput";
import { Button } from "@/components/ui/button";

/** Where the button stands: over the selection, in the frame's coordinates. */
type Spot = { left: number; top: number; text: string };

/** Room kept between the button and the edges of the frame. */
const MARGIN = 8;
/** How far above the selection the button stands. */
const LIFT = 36;

/**
 * A button over text the user selected in the timeline that quotes it into the
 * thread's composer. It is drawn in `frame`, outside the scrolling `viewport`: the
 * viewport contains its paint, so a button inside it would scroll and clip with it.
 * Selections in the composer are not the viewport's and never offer it.
 */
export function QuoteSelection(props: {
  threadId: string;
  viewport: RefObject<HTMLElement | null>;
  frame: RefObject<HTMLElement | null>;
}) {
  const { threadId, viewport, frame } = props;
  const [spot, setSpot] = useState<Spot | null>(null);
  const button = useRef<HTMLDivElement>(null);
  const hide = useCallback(() => setSpot(null), []);

  // Offer the button once a selection is made: when the mouse is let go or a key
  // that extends it, not while the drag is still going.
  const offer = useCallback(() => {
    const view = viewport.current;
    const box = frame.current;
    const selection = window.getSelection();
    if (!view || !box || !selection || selection.isCollapsed || selection.rangeCount === 0) return hide();
    const { anchorNode, focusNode } = selection;
    if (!anchorNode || !focusNode || !view.contains(anchorNode) || !view.contains(focusNode)) return hide();
    const text = selection.toString();
    if (text.trim() === "") return hide();
    const range = selection.getRangeAt(selection.rangeCount - 1);
    const rects = range.getClientRects();
    const rect = rects.length > 0 ? rects[0]! : range.getBoundingClientRect();
    const bounds = box.getBoundingClientRect();
    // Scrolled out of view: nothing to stand over.
    if (rect.bottom < bounds.top || rect.top > bounds.bottom) return hide();
    const width = button.current?.offsetWidth ?? 88;
    const left = Math.min(Math.max(rect.left - bounds.left, MARGIN), bounds.width - width - MARGIN);
    // Above the first line when there is room, under it otherwise.
    const above = rect.top - bounds.top - LIFT;
    const top = above >= MARGIN ? above : rect.bottom - bounds.top + MARGIN;
    setSpot({ left, top, text });
  }, [viewport, frame, hide]);

  useEffect(() => {
    const view = viewport.current;
    if (!view) return;
    const release = () => requestAnimationFrame(offer);
    const key = (event: KeyboardEvent) => event.shiftKey && release();
    view.addEventListener("mouseup", release);
    view.addEventListener("keyup", key);
    return () => {
      view.removeEventListener("mouseup", release);
      view.removeEventListener("keyup", key);
    };
  }, [viewport, offer]);

  // Gone with the selection, on scrolling, on Escape and on a press anywhere else.
  useEffect(() => {
    if (!spot) return;
    const view = viewport.current;
    const changed = () => {
      const selection = window.getSelection();
      if (!selection || selection.isCollapsed || selection.toString().trim() === "") hide();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    const press = (event: PointerEvent) => {
      if (!button.current?.contains(event.target as Node)) hide();
    };
    document.addEventListener("selectionchange", changed);
    document.addEventListener("keydown", escape);
    document.addEventListener("pointerdown", press, true);
    // The reader scrolling puts it away; the view moving by itself (following a reply
    // as it streams) carries it along with the selection.
    const follow = () => offer();
    view?.addEventListener("wheel", hide, { passive: true });
    view?.addEventListener("touchmove", hide, { passive: true });
    view?.addEventListener("scroll", follow, { passive: true });
    return () => {
      document.removeEventListener("selectionchange", changed);
      document.removeEventListener("keydown", escape);
      document.removeEventListener("pointerdown", press, true);
      view?.removeEventListener("wheel", hide);
      view?.removeEventListener("touchmove", hide);
      view?.removeEventListener("scroll", follow);
    };
  }, [spot, viewport, hide, offer]);

  // The thread changed under a selection made in the one before.
  useEffect(() => hide, [threadId, hide]);

  const quote = () => {
    if (!spot) return;
    changeComposer(threadId, (current) => appendQuote(current, spot.text));
    window.getSelection()?.removeAllRanges();
    hide();
  };

  return (
    <div
      ref={button}
      className={spot ? "absolute z-20" : "pointer-events-none absolute invisible"}
      style={spot ? { left: spot.left, top: spot.top } : { left: 0, top: 0 }}
      // Pressing the button must not take the selection away before it is read.
      onPointerDown={(event) => event.preventDefault()}
      onMouseDown={(event) => event.preventDefault()}
    >
      <Button
        type="button"
        variant="outline"
        size="sm"
        tabIndex={spot ? 0 : -1}
        aria-hidden={!spot}
        className="bg-popover shadow-md"
        title="Quote the selection in a reply"
        onClick={quote}
      >
        <QuoteIcon /> Reply
      </Button>
    </div>
  );
}
