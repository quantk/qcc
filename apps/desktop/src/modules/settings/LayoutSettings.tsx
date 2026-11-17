// The layout of the window's blocks, editable: each slot lists the blocks it shows in
// order, and a block can be dragged to another place, moved with its buttons, or
// hidden. Changes apply at once and are remembered by this window.

import { ArrowDownIcon, ArrowUpIcon, GripVerticalIcon, RotateCcwIcon } from "lucide-react";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";

import { type SlotId, SLOTS, defaultLayout, dropIndex, fits, resetBlock } from "../../state/blocks";
import { store, useApp } from "../../state/store";
import { placeBlock, useArrangement } from "../../ui/blocks";
import { Choice } from "../../ui/kit";
import type { Block } from "../../ui/registry";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

/** The element the settings scroll to when the layout is asked for. */
export const LAYOUT_SECTION = "settings-layout";

/** How far the pointer moves before a press on a handle becomes a drag, in pixels. */
const THRESHOLD = 4;

/** Where a dragged block would land: a slot (null: hidden) and a position in it. */
type Target = { slot: SlotId | null; index: number };

type Drag = { id: string; title: string; x: number; y: number; target: Target | null; refused: boolean };

const HIDDEN = "hidden";

/** What the pointer is over: a slot's list, and where among its blocks. */
function targetAt(x: number, y: number, dragged: string): Target | null {
  const list = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-layout-slot]");
  if (!list) return null;
  const name = list.dataset.layoutSlot;
  const middles = [...list.querySelectorAll<HTMLElement>("[data-layout-block]")]
    .filter((row) => row.dataset.layoutBlock !== dragged)
    .map((row) => {
      const rect = row.getBoundingClientRect();
      return rect.top + rect.height / 2;
    });
  return { slot: name === HIDDEN ? null : (name as SlotId), index: dropIndex(middles, y) };
}

function useBlockDrag() {
  const [drag, setDrag] = useState<Drag | null>(null);
  const press = useRef<{ block: Block; x: number; y: number; dragging: boolean } | null>(null);

  const end = useCallback(() => {
    press.current = null;
    setDrag(null);
    document.body.style.removeProperty("cursor");
  }, []);

  useEffect(() => {
    const at = (event: PointerEvent) => {
      const current = press.current;
      if (!current) return null;
      const target = targetAt(event.clientX, event.clientY, current.block.id);
      const refused = target?.slot != null && !fits(current.block.scope, target.slot);
      return { target: refused ? null : target, refused };
    };
    const move = (event: PointerEvent) => {
      const current = press.current;
      if (!current) return;
      if (!current.dragging) {
        if (Math.hypot(event.clientX - current.x, event.clientY - current.y) < THRESHOLD) return;
        current.dragging = true;
        document.body.style.cursor = "grabbing";
      }
      const over = at(event);
      if (!over) return;
      setDrag({ id: current.block.id, title: current.block.title, x: event.clientX, y: event.clientY, ...over });
    };
    const up = (event: PointerEvent) => {
      const current = press.current;
      const over = current?.dragging ? at(event) : null;
      if (current && over?.target) placeBlock(current.block.id, over.target.slot, over.target.index);
      end();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && press.current) end();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key);
    };
  }, [end]);

  const start = useCallback((event: React.PointerEvent, block: Block) => {
    if (event.button !== 0) return;
    event.preventDefault();
    press.current = { block, x: event.clientX, y: event.clientY, dragging: false };
  }, []);

  return { drag, start };
}

export function LayoutSettings() {
  const { slots, hidden } = useArrangement();
  const layout = useApp((state) => state.blockLayout);
  const { drag, start } = useBlockDrag();

  const list = (slot: SlotId | null, blocks: readonly Block[]) => {
    const shown = blocks.filter((block) => block.id !== drag?.id);
    const landing = drag?.target && drag.target.slot === slot ? (slot === null ? 0 : drag.target.index) : null;
    const line = <div aria-hidden className="mx-2 h-0.5 rounded bg-primary" />;
    return (
      <div
        data-layout-slot={slot ?? HIDDEN}
        className={`flex min-h-10 flex-col gap-0.5 rounded-lg border p-1 ${landing !== null ? "border-primary/60 bg-primary/5" : "bg-card"}`}
      >
        {blocks.length === 0 && landing === null && (
          <p className="px-2 py-2 text-[13px] text-muted-foreground">{slot ? "Nothing here. Drag a block in." : "Nothing is hidden."}</p>
        )}
        {blocks.map((block) => {
          const position = shown.indexOf(block);
          return (
            <Fragment key={block.id}>
              {landing !== null && slot !== null && position === landing && line}
              <BlockRow
                block={block}
                slot={slot}
                position={position}
                count={shown.length}
                dragged={drag?.id === block.id}
                changed={block.id in layout}
                startDrag={start}
              />
            </Fragment>
          );
        })}
        {landing !== null && (slot === null || landing === shown.length) && line}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-4 pt-2">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-[13px] text-muted-foreground">
          Drag a block by its handle to another place, or right-click its button, tab or sidebar section to move it from where it is.
          A block about a thread can only stand beside a thread.
        </p>
        <Button variant="outline" size="sm" disabled={Object.keys(layout).length === 0} onClick={() => store().setBlockLayout(defaultLayout)}>
          <RotateCcwIcon /> Reset layout
        </Button>
      </div>
      {SLOTS.map((slot) => (
        <section key={slot.id} className="flex flex-col gap-1.5">
          <div className="flex items-baseline gap-2">
            <h3 className="text-sm font-semibold">{slot.title}</h3>
            <span className="text-[13px] text-muted-foreground">{slot.hint}</span>
          </div>
          {list(slot.id, slots[slot.id])}
        </section>
      ))}
      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-semibold">Hidden</h3>
          <span className="text-[13px] text-muted-foreground">Not shown anywhere; drag one out to show it again</span>
        </div>
        {list(null, hidden)}
      </section>
      {drag && (
        <div
          className="pointer-events-none fixed z-50 max-w-72 rounded-md border bg-popover px-2 py-1 text-xs shadow-md"
          style={{ left: drag.x + 14, top: drag.y + 10 }}
        >
          <div className="truncate font-medium">{drag.title}</div>
          <div className={drag.refused ? "text-destructive" : "text-muted-foreground"}>
            {drag.refused ? "Needs a thread: put it in a thread's tabs, side panel or beside its composer" : drag.target ? "Let go to put it here" : "Drop it on a list"}
          </div>
        </div>
      )}
    </div>
  );
}

function BlockRow(props: {
  block: Block;
  slot: SlotId | null;
  /** Its position among the slot's blocks; -1 while it is dragged. */
  position: number;
  count: number;
  dragged: boolean;
  changed: boolean;
  startDrag: (event: React.PointerEvent, block: Block) => void;
}) {
  const { block, slot, position, count } = props;
  const options = [
    ...SLOTS.filter((candidate) => fits(block.scope, candidate.id)).map((candidate) => ({ value: candidate.id, label: candidate.title })),
    { value: null, label: "Hidden" },
  ];
  return (
    <div
      data-layout-block={block.id}
      className={`flex items-center gap-2 rounded-md px-1 py-1 hover:bg-muted ${props.dragged ? "opacity-40" : ""}`}
    >
      <Button
        variant="ghost"
        size="icon-xs"
        className="cursor-grab touch-none text-muted-foreground"
        aria-label={`Drag ${block.title}`}
        title="Drag to another place"
        onPointerDown={(event) => props.startDrag(event, block)}
      >
        <GripVerticalIcon />
      </Button>
      {block.icon ? <block.icon className="size-4 shrink-0 text-muted-foreground" /> : <span className="size-4 shrink-0" />}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{block.title}</span>
          {block.scope === "thread" && <Badge variant="outline">Thread</Badge>}
        </div>
        {block.hint && <div className="truncate text-xs text-muted-foreground">{block.hint}</div>}
      </div>
      {slot && (
        <>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Move ${block.title} up`}
            disabled={position <= 0}
            onClick={() => placeBlock(block.id, slot, position - 1)}
          >
            <ArrowUpIcon />
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`Move ${block.title} down`}
            disabled={position < 0 || position >= count - 1}
            onClick={() => placeBlock(block.id, slot, position + 1)}
          >
            <ArrowDownIcon />
          </Button>
        </>
      )}
      <Choice
        aria-label={`Where ${block.title} stands`}
        size="sm"
        className="w-44"
        value={slot}
        options={options}
        onChange={(value) => placeBlock(block.id, value as SlotId | null)}
      />
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Put ${block.title} back where it was`}
        title="Back to its default place"
        className={props.changed ? undefined : "invisible"}
        onClick={() => store().setBlockLayout(resetBlock(store().getState().blockLayout, block.id))}
      >
        <RotateCcwIcon />
      </Button>
    </div>
  );
}
