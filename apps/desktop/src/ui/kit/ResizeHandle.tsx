import { type KeyboardEvent, type PointerEvent, useRef } from "react";

import { type Bounds, clampWidth, draggedWidth } from "../../state/layout";
import { cn } from "@/lib/utils";

/** How far one press of an arrow key moves the edge. */
const KEY_STEP = 16;

/** While dragging, the pointer leaves the thin handle: keep its cursor and select no text. */
function holdCursor(on: boolean) {
  document.body.style.cursor = on ? "col-resize" : "";
  document.body.style.userSelect = on ? "none" : "";
}

type Drag = { pointer: number; startX: number; start: number; bounds: Bounds; last: number };

/**
 * The draggable edge of a side of the window. It sits on the `edge` of its parent,
 * which must be positioned. `resize` is called while the pointer moves with
 * `done = false` and once more with `done = true` when it is let go, so the caller
 * can remember the width once rather than on every move. A double click puts the
 * initial width back.
 *
 * `room` caps the width by what the window has to spare right now, so the side
 * cannot push the thread out of sight.
 */
export function ResizeHandle({
  edge,
  width,
  bounds,
  room,
  resize,
  label,
}: {
  edge: "left" | "right";
  width: number;
  bounds: Bounds;
  room?: () => number;
  resize: (width: number, done: boolean) => void;
  label: string;
}) {
  const drag = useRef<Drag | null>(null);
  const limited = (): Bounds => {
    const spare = room?.();
    return spare === undefined ? bounds : { ...bounds, max: Math.max(bounds.min, Math.min(bounds.max, spare)) };
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    // Start from what is drawn: a remembered width may be more than the window allows now.
    const drawn = event.currentTarget.parentElement?.getBoundingClientRect().width ?? width;
    const start = clampWidth(drawn, limited());
    drag.current = { pointer: event.pointerId, startX: event.clientX, start, bounds: limited(), last: start };
    holdCursor(true);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    const next = draggedWidth(current.start, current.startX, event.clientX, edge, current.bounds);
    if (next === current.last) return;
    current.last = next;
    resize(next, false);
  };

  const finish = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    drag.current = null;
    holdCursor(false);
    resize(current.last, true);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const grow = edge === "right" ? "ArrowRight" : "ArrowLeft";
    const shrink = edge === "right" ? "ArrowLeft" : "ArrowRight";
    if (event.key !== grow && event.key !== shrink) return;
    event.preventDefault();
    resize(clampWidth(width + (event.key === grow ? KEY_STEP : -KEY_STEP), limited()), true);
  };

  return (
    // A separator that can be focused and moved is the ARIA pattern for a splitter.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={bounds.min}
      aria-valuemax={bounds.max}
      tabIndex={0}
      title={`${label}: drag to resize, double-click to reset`}
      className={cn(
        "group absolute inset-y-0 z-30 w-2 cursor-col-resize touch-none outline-none select-none",
        edge === "right" ? "-right-1" : "-left-1",
      )}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={finish}
      onPointerCancel={finish}
      onDoubleClick={() => resize(clampWidth(bounds.initial, limited()), true)}
      onKeyDown={onKeyDown}
    >
      <div className="mx-auto h-full w-0.5 bg-transparent transition-colors group-hover:bg-primary/50 group-focus-visible:bg-primary group-active:bg-primary" />
    </div>
  );
}
