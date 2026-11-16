// Dragging threads in the sidebar to place them under another thread or on their own.
//
// Pointer events rather than HTML drag and drop: the Tauri window handles native
// drops itself, and in a webview that takes the HTML events away.

import { useCallback, useEffect, useRef, useState } from "react";

import type { Thread } from "../api/protocol.gen";
import { type Drop, type DropZone, dropOf, threadTitle } from "../state/selectors";
import { store, useApp } from "../state/store";

/** How far the pointer moves before a press becomes a drag, in pixels. */
const THRESHOLD = 5;

export type DragState = {
  /** The thread being dragged. */
  id: string;
  title: string;
  x: number;
  y: number;
  /** What it is over: a thread and which part of it, the top of its group, or nothing. */
  over: { target: string | null; zone: DropZone } | null;
  drop: Drop | null;
};

type Press = { id: string; title: string; group: readonly Thread[]; x: number; y: number; dragging: boolean };

/** Which part of a row the pointer is on: its middle puts a thread under it. */
function zoneOf(element: Element, y: number): DropZone {
  const rect = element.getBoundingClientRect();
  const offset = (y - rect.top) / rect.height;
  return offset > 0.25 && offset < 0.75 ? "into" : "beside";
}

/** What is under the pointer: a row (`data-thread-id`) or the top of a group (`data-group-top`). */
function overAt(x: number, y: number, groupKey: string): DragState["over"] {
  const element = document.elementFromPoint(x, y)?.closest("[data-thread-id], [data-group-top]");
  if (!element || element.getAttribute("data-group") !== groupKey) return null;
  const target = element.getAttribute("data-thread-id");
  return target ? { target, zone: zoneOf(element, y) } : { target: null, zone: "into" };
}

export function useThreadDrag() {
  const [drag, setDrag] = useState<DragState | null>(null);
  const press = useRef<(Press & { groupKey: string }) | null>(null);
  /** A drag ends with a click on whatever is under the pointer; that click is not a selection. */
  const dragged = useRef(false);
  const collapsed = useApp((state) => state.collapsed);

  const end = useCallback(() => {
    press.current = null;
    setDrag(null);
    document.body.style.removeProperty("cursor");
  }, []);

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const current = press.current;
      if (!current) return;
      if (!current.dragging) {
        if (Math.hypot(event.clientX - current.x, event.clientY - current.y) < THRESHOLD) return;
        current.dragging = true;
        dragged.current = true;
        document.body.style.cursor = "grabbing";
      }
      const over = overAt(event.clientX, event.clientY, current.groupKey);
      setDrag({
        id: current.id,
        title: current.title,
        x: event.clientX,
        y: event.clientY,
        over,
        drop: over ? dropOf(current.group, current.id, over.target, over.zone) : null,
      });
    };
    const up = (event: PointerEvent) => {
      const current = press.current;
      if (current?.dragging) {
        const over = overAt(event.clientX, event.clientY, current.groupKey);
        const drop = over ? dropOf(current.group, current.id, over.target, over.zone) : null;
        if (drop && "parent" in drop) {
          // Put under a folded thread, it would vanish; unfold it so it is seen landing.
          if (drop.parent && collapsed.has(drop.parent)) store().toggleCollapsed(drop.parent);
          void store().setParent(current.id, drop.parent);
        }
        setTimeout(() => (dragged.current = false));
      }
      end();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && press.current?.dragging) {
        setTimeout(() => (dragged.current = false));
        end();
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key);
    };
  }, [collapsed, end]);

  /** Begin watching a press on a row; it becomes a drag once the pointer moves. */
  const start = useCallback(
    (event: React.PointerEvent, thread: Thread, group: readonly Thread[], groupKey: string) => {
      if (event.button !== 0) return;
      press.current = {
        id: thread.id,
        title: threadTitle(thread),
        group,
        groupKey,
        x: event.clientX,
        y: event.clientY,
        dragging: false,
      };
    },
    [],
  );

  return { drag, start, wasDragged: () => dragged.current };
}
