// The diff of a thread's changes as one list that draws only the rows in sight. Each
// file's diff is read when it comes into view, highlighted line by line in a worker,
// and the whole diff scrolls sideways as one.

import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRightIcon, FoldVerticalIcon, MessageSquarePlusIcon, UnfoldVerticalIcon } from "lucide-react";
import {
  type CSSProperties,
  type ReactNode,
  memo,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import type { ChangedFile } from "../../api/protocol.gen";
import { type ChangesState, type ThreadChangesReader, isBinary } from "../../state/changes";
import type { DiffFile, DiffLine } from "../../state/diff";
import {
  type DiffRow,
  type FileNote,
  LINE_HEIGHT,
  diffExtent,
  diffRows,
  estimateRowHeight,
  firstChange,
  shownFile,
  wantedPaths,
} from "../../state/diffRows";
import type { Token } from "@/lib/highlightProtocol";
import { type InView, diffHighlights } from "@/lib/diffHighlights";
import { DiffStat } from "../../ui/kit";
import { Button } from "@/components/ui/button";
import type { Review } from "./review";

/** Rows past the last one in sight whose files are read ahead, so scrolling finds them ready. */
const READ_AHEAD = 80;

const NO_FILES: ChangedFile[] = [];

const statusTag: Record<ChangedFile["kind"], string> = { added: "new", untracked: "new", deleted: "deleted", modified: "" };

/** Lines of context left in sight above the first change when a file opens whole. */
const LEAD_LINES = 4;

/** Where the first row in sight was, to keep it in place when rows above it change. */
type Anchor = { key: string; offset: number; header: string; headerOffset: number };

export function ChangesDiff({
  reader,
  state,
  review,
  lead,
  inset,
}: {
  reader: ThreadChangesReader;
  state: ChangesState;
  review: Review;
  /** What comes before the diff (the list of files, the comments' bar); `jump` brings a file into view. */
  lead: (jump: (path: string) => void) => ReactNode;
  /** Pixels between the diff and the sides of the view. */
  inset: number;
}) {
  const owner = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<ReadonlyMap<string, boolean>>(new Map());
  const files = state.changes?.files ?? NO_FILES;
  const changed = useMemo(() => new Map(files.map((file) => [file.path, file])), [files]);
  const rows = useMemo(
    () => diffRows({ files, diffs: state.diffs, opened, heads: review.heads, below: review.below }),
    [files, state.diffs, opened, review.heads, review.below],
  );
  const fileOf = useCallback((path: string) => shownFile(state.diffs.get(path)), [state.diffs]);
  const extent = useMemo(() => diffExtent(rows, fileOf), [rows, fileOf]);
  const viewWidth = useWidth(scrollRef);

  const virtualizer = useVirtualizer({
    count: rows.length + 1,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => (index === 0 ? 120 : estimateRowHeight(rows[index - 1]!)),
    getItemKey: (index) => (index === 0 ? "lead" : rows[index - 1]!.key),
    overscan: 12,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items.at(-1)?.index ?? 0;

  // Keep the row at the top of the view in place when rows above it come or go: a
  // diff read in, a file folded, a comment drawn.
  const anchor = useRef<Anchor | null>(null);
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const remember = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    const top = element.scrollTop;
    // The items drawn may be a frame behind the scroll; the measurements are not.
    const measured = virtualizer.measurementsCache;
    let low = 1;
    let high = measured.length - 1;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (measured[middle]!.end > top) high = middle;
      else low = middle + 1;
    }
    const shown = measured[low];
    const row = shown && rowsRef.current[low - 1];
    if (!shown || !row || top === 0) {
      anchor.current = null;
      return;
    }
    const header = `f:${row.path}`;
    const headerAt = rowsRef.current.findIndex((other) => other.key === header);
    const headerStart = measured[headerAt + 1]?.start ?? shown.start;
    anchor.current = { key: row.key, offset: shown.start - top, header, headerOffset: headerStart - top };
  }, [virtualizer]);
  useLayoutEffect(() => {
    const element = scrollRef.current;
    const held = anchor.current;
    if (!element || !held || element.scrollTop === 0) return;
    let at = rows.findIndex((row) => row.key === held.key);
    let offset = held.offset;
    if (at < 0) {
      at = rows.findIndex((row) => row.key === held.header);
      offset = held.headerOffset;
    }
    const start = at >= 0 ? virtualizer.measurementsCache[at + 1]?.start : undefined;
    if (start === undefined) return;
    if (Math.abs(start - offset - element.scrollTop) >= 1) element.scrollTop = start - offset;
  }, [rows, virtualizer]);

  // Read the diffs of the open files in sight, then those just past them.
  useEffect(() => {
    reader.want(wantedPaths(rows, first - 1, last - 1 + READ_AHEAD));
  }, [reader, rows, first, last, state]);

  // Highlight the lines in sight, and only those.
  useEffect(() => {
    const inView = new Map<DiffFile, InView>();
    for (let index = Math.max(1, first); index <= last; index++) {
      const row = rows[index - 1];
      if (row?.kind !== "line") continue;
      const file = fileOf(row.path);
      if (!file) continue;
      const seen = inView.get(file);
      if (seen) seen.to = row.index;
      else inView.set(file, { file, from: row.index, to: row.index });
    }
    diffHighlights.show(owner, [...inView.values()]);
  }, [owner, rows, first, last, fileOf]);
  useEffect(() => () => diffHighlights.show(owner, []), [owner]);
  useSyncExternalStore(diffHighlights.subscribe, diffHighlights.getVersion);

  const jump = useCallback(
    (path: string) => {
      const element = scrollRef.current;
      const key = `f:${path}`;
      const at = rowsRef.current.findIndex((row) => row.key === key);
      const start = virtualizer.measurementsCache[at + 1]?.start;
      if (!element || at < 0 || start === undefined) return;
      // Held by its header from here on, as diffs above and below it are read in.
      anchor.current = { key, offset: 0, header: key, headerOffset: 0 };
      element.scrollTop = start;
    },
    [virtualizer],
  );
  const toggle = useCallback((path: string, open: boolean) => {
    setOpened((current) => new Map(current).set(path, open));
  }, []);

  // A file asked for whole is brought to its first change once it is read so: the
  // lines before it would otherwise push the change out of sight.
  const seek = useRef<string | null>(null);
  const showWhole = useCallback(
    (path: string, on: boolean) => {
      seek.current = on ? path : null;
      if (on) toggle(path, true);
      reader.setFull(path, on);
    },
    [reader, toggle],
  );
  useLayoutEffect(() => {
    const path = seek.current;
    const element = scrollRef.current;
    if (!path || !element) return;
    const diff = state.diffs.get(path);
    if (!state.full.has(path)) {
      seek.current = null;
      return;
    }
    if (diff?.state !== "read" || !diff.full) {
      // Not read whole yet: wait, unless reading it failed.
      if (diff?.state === "failed") seek.current = null;
      return;
    }
    seek.current = null;
    const index = firstChange(diff.file);
    if (index === null) return;
    const key = `l:${path}:${Math.max(0, index - LEAD_LINES)}`;
    const at = rows.findIndex((row) => row.key === key);
    const start = virtualizer.measurementsCache[at + 1]?.start;
    if (at < 0 || start === undefined) return;
    // Held there from now on, as rows above it come and go.
    anchor.current = { key, offset: 0, header: `f:${path}`, headerOffset: 0 };
    element.scrollTop = start;
  }, [rows, state, virtualizer]);

  const gutter = `calc(${extent.digits}ch + 12px)`;
  const width = `max(100%, calc(${2 * inset + 2}px + 2 * ${gutter} + ${extent.columns + 3}ch + 12px))`;
  const pinned: CSSProperties = { width: viewWidth || "100%", paddingLeft: inset, paddingRight: inset };

  return (
    <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto [overflow-anchor:none]" onScroll={remember}>
      <div
        className="relative font-[family-name:var(--mono)] text-[12.5px]"
        style={{ height: virtualizer.getTotalSize(), width, lineHeight: `${LINE_HEIGHT}px` }}
      >
        {items.map((item) => {
          const place: CSSProperties = { transform: `translateY(${item.start}px)` };
          if (item.index === 0) {
            return (
              <div key={item.key} data-index={0} ref={virtualizer.measureElement} className="absolute top-0 left-0 w-full" style={place}>
                <div className="sticky left-0 py-3 font-sans text-sm leading-normal" style={pinned}>
                  {lead(jump)}
                </div>
              </div>
            );
          }
          const row = rows[item.index - 1]!;
          if (row.kind === "line") {
            const file = fileOf(row.path);
            const line = file?.lines[row.index];
            if (!file || !line) return null;
            return (
              <LineRow
                key={item.key}
                file={file}
                index={row.index}
                line={line}
                tokens={diffHighlights.tokens(file, row.index)}
                gutter={gutter}
                inset={inset}
                start={item.start}
                pick={review.addon.pick && (review.addon.canPick?.(file, row.index) ?? true) ? review.addon.pick : undefined}
              />
            );
          }
          return (
            <div
              key={item.key}
              data-index={item.index}
              ref={virtualizer.measureElement}
              className="absolute top-0 left-0 w-full font-sans leading-normal"
              style={place}
            >
              <div className="sticky left-0" style={pinned}>
                <PinnedRow
                  row={row}
                  changed={changed.get(row.path)}
                  file={fileOf(row.path)}
                  state={state}
                  review={review}
                  toggle={toggle}
                  showWhole={showWhole}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** The width of `ref`'s box as it changes. */
function useWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

const signOf = { add: "+", del: "−", context: " " } as const;

const lineTone: Record<DiffLine["kind"], string> = {
  add: "bg-[var(--green-bg)]",
  del: "bg-[var(--red-bg)]",
  context: "",
  hunk: "bg-[var(--panel)] text-muted-foreground",
  note: "bg-[var(--panel)] text-muted-foreground",
};

const LineRow = memo(function LineRow({
  file,
  index,
  line,
  tokens,
  gutter,
  inset,
  start,
  pick,
}: {
  file: DiffFile;
  index: number;
  line: DiffLine;
  tokens: Token[] | undefined;
  gutter: string;
  inset: number;
  start: number;
  pick?: (file: DiffFile, index: number) => void;
}) {
  const number = "flex-none pr-1.5 text-right text-muted-foreground opacity-70 select-none";
  return (
    <div
      className="absolute top-0 left-0 w-full"
      style={{ height: LINE_HEIGHT, transform: `translateY(${start}px)`, paddingLeft: inset, paddingRight: inset }}
    >
      <div className={`group/line flex h-full border-x whitespace-pre ${lineTone[line.kind]}`}>
        {line.kind === "hunk" || line.kind === "note" ? (
          <>
            <span className="flex-none" style={{ width: `calc(2 * ${gutter})` }} />
            <span className="px-1.5">{line.text}</span>
          </>
        ) : (
          <>
            <span className={`relative ${number}`} style={{ width: gutter }}>
              {pick && (
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="absolute top-0 left-0.5 size-[1.45em] bg-muted opacity-0 group-hover/line:opacity-100 focus-visible:opacity-100"
                  title="Comment on this line"
                  aria-label="Comment on this line"
                  onClick={() => pick(file, index)}
                >
                  <MessageSquarePlusIcon />
                </Button>
              )}
              {line.oldNo}
            </span>
            <span className={number} style={{ width: gutter }}>
              {line.newNo}
            </span>
            <span className="flex-1 px-1.5">
              <span className="inline-block w-[1.4ch] text-muted-foreground select-none">{signOf[line.kind]}</span>
              {tokens
                ? tokens.map(([content, style], at) => (
                    <span key={at} style={style as CSSProperties} className="text-(--shiki-light) dark:text-(--shiki-dark)">
                      {content}
                    </span>
                  ))
                : line.text}
            </span>
          </>
        )}
      </div>
    </div>
  );
});

function PinnedRow({
  row,
  changed,
  file,
  state,
  review,
  toggle,
  showWhole,
}: {
  row: Exclude<DiffRow, { kind: "line" }>;
  changed: ChangedFile | undefined;
  file: DiffFile | undefined;
  state: ChangesState;
  review: Review;
  toggle: (path: string, open: boolean) => void;
  showWhole: (path: string, on: boolean) => void;
}) {
  switch (row.kind) {
    case "file": {
      const additions = changed?.additions ?? file?.additions;
      const deletions = changed?.deletions ?? file?.deletions ?? 0;
      const tag = changed && isBinary(changed) ? "binary" : changed ? statusTag[changed.kind] : "";
      // A new or deleted file is shown whole anyway; a binary one not at all.
      const canShowWhole = changed?.kind === "modified" && !isBinary(changed) && !file?.binary;
      const whole = state.full.has(row.path);
      const wholeLabel = whole ? "Show only the changes" : "Show the whole file";
      return (
        <div className="flex overflow-hidden rounded-t-[7px] border border-b-0 bg-[var(--panel)]">
          <Button
            variant="ghost"
            className="h-auto min-w-0 flex-1 justify-start gap-2 rounded-none bg-[var(--panel)] px-2.5 py-[5px] text-left font-normal group"
            data-panel-open={row.open ? "" : undefined}
            aria-expanded={row.open}
            title={row.path}
            onClick={() => toggle(row.path, !row.open)}
          >
            <ChevronRightIcon className="size-3.5 flex-none text-muted-foreground transition-transform group-data-[panel-open]:rotate-90" />
            <span className="diff-path">{row.path}</span>
            {tag && <span className="tag">{tag}</span>}
            {additions !== undefined && additions !== null && <DiffStat additions={additions} deletions={deletions} />}
          </Button>
          {canShowWhole && (
            <Button
              variant="ghost"
              size="icon-xs"
              className="my-auto mr-1.5 flex-none text-muted-foreground aria-pressed:bg-muted aria-pressed:text-foreground"
              aria-pressed={whole}
              title={wholeLabel}
              aria-label={wholeLabel}
              onClick={() => showWhole(row.path, !whole)}
            >
              {whole ? <FoldVerticalIcon /> : <UnfoldVerticalIcon />}
            </Button>
          )}
        </div>
      );
    }
    case "note":
      return (
        <div className="flex items-center gap-2 border-x border-t px-3 py-1 text-xs text-muted-foreground">
          <NoteText note={row.note} changed={changed} error={noteError(state, row.path)} />
          {row.note === "folded" && (
            <Button variant="outline" size="xs" onClick={() => toggle(row.path, true)}>
              Show the diff
            </Button>
          )}
        </div>
      );
    case "end":
      return <div className="mb-3 h-px rounded-b-[7px] border-x border-b" />;
    case "head":
    case "below": {
      const content = file && (row.kind === "head" ? review.addon.head?.(file) : review.addon.below?.(file, row.index));
      return (
        <div className="border-x border-y bg-background p-2 text-sm whitespace-normal">
          <div className="max-w-2xl">{content}</div>
        </div>
      );
    }
  }
}

function noteError(state: ChangesState, path: string): string | undefined {
  const diff = state.diffs.get(path);
  return diff?.state === "failed" ? diff.error : undefined;
}

function NoteText({ note, changed, error }: { note: FileNote; changed: ChangedFile | undefined; error?: string }) {
  switch (note) {
    case "folded": {
      const lines = changed ? (changed.additions ?? 0) + (changed.deletions ?? 0) : 0;
      return <span className="flex-1">{lines > 0 ? `${lines.toLocaleString()} changed lines, folded` : "Folded"}</span>;
    }
    case "reading":
      return <span className="flex-1">Reading the diff…</span>;
    case "binary":
      return <span className="flex-1">Binary file, not shown</span>;
    case "empty":
      return <span className="flex-1">No changes to the text (a mode or an empty file)</span>;
    case "truncated":
      return <span className="flex-1">The diff of this file is too large to show in full; it is cut short.</span>;
    case "failed":
      return <span className="flex-1 text-destructive">Cannot read the diff: {error}</span>;
  }
}
