// Comments on lines of the changes, as in a code review: left under a line of the
// diff, handed to the agent, and marked resolved by it (`qcc comment resolve`).

import { BotIcon, CheckIcon, ChevronDownIcon, ChevronUpIcon, RotateCcwIcon, SendIcon, Trash2Icon } from "lucide-react";
import { type KeyboardEvent, type ReactElement, useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { LineComment, Thread } from "../../api/protocol.gen";
import { type CommentStop, type StopPlace, commentStops, stepStop, stopIndex } from "../../state/commentNav";
import { type Placement, lineAnchor, placeComments } from "../../state/comments";
import type { DiffFile } from "../../state/diff";
import { store, useApp } from "../../state/store";
import { type DiffAddon, Markdown } from "../../ui/kit";
import { plural } from "../../util/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

const NO_COMMENTS: readonly LineComment[] = [];

/** What the agent is asked along with the comments, which reach it in full in a note. */
const REQUEST = "Please address my comments on your changes.";

/** How long a comment stepped to stays marked. */
const FLASH_MS = 1600;

export type Review = {
  comments: readonly LineComment[];
  open: LineComment[];
  /** Comments on files that are no longer changed. */
  elsewhere: LineComment[];
  addon: DiffAddon;
  /** Files with comments adrift in them, shown above their lines. */
  heads: ReadonlySet<string>;
  /** By file, the lines with comments or a draft under them. */
  below: ReadonlyMap<string, ReadonlySet<number>>;
  /** Where each comment is shown in the diffs read so far. */
  placement: Placement;
  /** Mark a comment for a moment, as the bar brings it into view. */
  flash: (id: LineComment["id"]) => void;
};

/**
 * The comments on a thread's changes, laid out over the diffs read so far (`files`)
 * of the files it changed (`changed`).
 */
export function useReview(thread: Thread, files: readonly DiffFile[], changed?: ReadonlySet<string>): Review {
  const comments = useApp((state) => state.comments[thread.id]) ?? NO_COMMENTS;
  const [draft, setDraft] = useState<{ path: string; index: number } | null>(null);
  const [flashed, setFlashed] = useState<LineComment["id"] | null>(null);
  const flashTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const flash = useCallback((id: LineComment["id"]) => {
    clearTimeout(flashTimer.current);
    setFlashed(id);
    flashTimer.current = setTimeout(() => setFlashed(null), FLASH_MS);
  }, []);
  useEffect(() => () => clearTimeout(flashTimer.current), []);

  useEffect(() => {
    void store().loadComments(thread.id);
    setDraft(null);
  }, [thread.id]);

  const placement = useMemo(() => placeComments(files, comments, changed), [files, comments, changed]);

  const marks = useMemo(() => {
    const below = new Map<string, Set<number>>();
    for (const [path, lines] of placement.byFile) below.set(path, new Set(lines.keys()));
    if (draft) {
      const lines = below.get(draft.path) ?? new Set<number>();
      lines.add(draft.index);
      below.set(draft.path, lines);
    }
    return { heads: new Set(placement.adrift.keys()), below };
  }, [placement, draft]);

  const addon = useMemo<DiffAddon>(
    () => ({
      canPick: (file, index) => lineAnchor(file.lines[index]!) !== null,
      pick: (file, index) => setDraft({ path: file.path, index }),
      head(file) {
        const adrift = placement.adrift.get(file.path);
        if (!adrift) return null;
        return (
          <CommentList
            thread={thread}
            comments={adrift}
            flashed={flashed}
            label={(comment) => `Line ${comment.line}, no longer in the diff`}
          />
        );
      },
      below(file, index) {
        const here = placement.byFile.get(file.path)?.get(index);
        const drafting = draft?.path === file.path && draft.index === index;
        if (!here && !drafting) return null;
        return (
          <div className="flex flex-col gap-2">
            {here && <CommentList thread={thread} comments={here} flashed={flashed} />}
            {drafting && (
              <CommentDraft
                onCancel={() => setDraft(null)}
                onSubmit={async (text) => {
                  const line = file.lines[index]!;
                  const anchor = lineAnchor(line)!;
                  const kept = await store().addComment({
                    thread_id: thread.id,
                    path: file.path,
                    line: anchor.line,
                    side: anchor.side,
                    quote: line.text,
                    text,
                  });
                  if (kept) setDraft(null);
                }}
              />
            )}
          </div>
        );
      },
    }),
    [thread, placement, draft, flashed],
  );

  const open = useMemo(() => comments.filter((comment) => comment.resolved_at == null), [comments]);
  return { comments, open, elsewhere: placement.elsewhere, addon, placement, flash, ...marks };
}

/** Hands the open comments to the agent, each in full, in a note beside the request. */
function useSendComments(thread: Thread, review: Review) {
  const [sending, setSending] = useState(false);
  const send = async () => {
    setSending(true);
    await store().send(thread.id, REQUEST, "queue", [
      { type: "comments", comment_ids: review.open.map((comment) => comment.id) },
    ]);
    setSending(false);
  };
  return { send, sending, disabled: review.open.length === 0 || sending };
}

/** The comments left on files that are no longer changed, which the diff has no place for. */
export function ReviewBar({ thread, review }: { thread: Thread; review: Review }) {
  if (review.elsewhere.length === 0) return null;
  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/40 p-2 text-sm">
      <span className="text-muted-foreground">On files no longer changed</span>
      <CommentList
        thread={thread}
        comments={review.elsewhere}
        label={(comment) => `${comment.path}:${comment.line}, no longer changed`}
      />
    </div>
  );
}

/** Every view's bar that owns the keys, the one used last (the main tab and the side panel may both show one). */
let keysOwner: object | null = null;

/**
 * Above the diff while it scrolls: how many comments are open, steps to the one
 * before or after (`p`/`n`, `Alt+↑`/`Alt+↓`), and the way to hand them to the agent.
 * `paths` are the changed files in the order of the diff; `go` brings a comment
 * into view, opening its file.
 */
export function CommentsBar(props: {
  thread: Thread;
  review: Review;
  paths: readonly string[];
  go: (stop: CommentStop) => void;
  compact?: boolean;
}) {
  const { thread, review, paths, go, compact = false } = props;
  const { send, sending, disabled } = useSendComments(thread, review);
  const stops = useMemo(() => commentStops(paths, review.comments, review.placement), [
    paths,
    review.comments,
    review.placement,
  ]);
  const [current, setCurrent] = useState<StopPlace | null>(null);
  const at = stopIndex(stops, current?.id);

  const step = useCallback(
    (direction: 1 | -1) => {
      const next = stepStop(stops, current, direction);
      if (!next) return;
      setCurrent({ id: next.comment.id, file: next.file, at: next.at });
      go(next);
    },
    [stops, current, go],
  );

  const ref = useRef<HTMLDivElement>(null);
  const stepRef = useRef(step);
  stepRef.current = step;
  const shown = review.comments.length > 0;
  useEffect(() => {
    const element = ref.current;
    const view = element?.closest("[data-changes-view]") ?? element;
    if (!element || !view) return;
    const token = {};
    keysOwner = token;
    const claim = () => (keysOwner = token);
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (keysOwner !== token || event.defaultPrevented || event.ctrlKey || event.metaKey || isEditable(event.target)) return;
      // A tab not drawn (behind another) keeps its bar but gives up the keys.
      if (element.getClientRects().length === 0) return;
      const direction = stepKey(event);
      if (!direction) return;
      event.preventDefault();
      stepRef.current(direction);
    };
    view.addEventListener("pointerdown", claim, true);
    view.addEventListener("focusin", claim, true);
    window.addEventListener("keydown", onKey);
    return () => {
      view.removeEventListener("pointerdown", claim, true);
      view.removeEventListener("focusin", claim, true);
      window.removeEventListener("keydown", onKey);
      if (keysOwner === token) keysOwner = null;
    };
  }, [shown]);

  if (!shown) return null;
  const resolved = review.comments.length - review.open.length;
  const outside = review.open.length - stops.length;
  const label =
    review.open.length === 0
      ? "Every comment is resolved"
      : at >= 0
        ? `Comment ${at + 1} of ${stops.length}`
        : plural(review.open.length, "open comment");
  return (
    <div
      ref={ref}
      className={`flex flex-none items-center gap-1 border-b bg-muted text-sm ${compact ? "py-1 pr-2 pl-4" : "px-5 py-1.5"}`}
    >
      <span className="min-w-0 flex-1 truncate" aria-live="polite">
        {label}
        {outside > 0 && <span className="text-muted-foreground"> · {outside} on files no longer changed</span>}
        {resolved > 0 && <span className="text-muted-foreground"> · {resolved} resolved</span>}
      </span>
      <StepButton label="Previous comment" keys={["P", "Alt+↑"]} disabled={stops.length === 0} onClick={() => step(-1)}>
        <ChevronUpIcon />
      </StepButton>
      <StepButton label="Next comment" keys={["N", "Alt+↓"]} disabled={stops.length === 0} onClick={() => step(1)}>
        <ChevronDownIcon />
      </StepButton>
      <Button
        size="xs"
        className="ml-1"
        disabled={disabled}
        title="Send the open comments to the agent, each in full, with how to mark it resolved"
        onClick={() => void send()}
      >
        <SendIcon /> {sending ? "Sending…" : "Send to agent"}
      </Button>
    </div>
  );
}

function StepButton(props: {
  label: string;
  keys: string[];
  disabled: boolean;
  onClick: () => void;
  children: ReactElement;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        delay={300}
        render={
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={props.label}
            aria-keyshortcuts={props.keys.join(" ")}
            disabled={props.disabled}
            onClick={props.onClick}
          />
        }
      >
        {props.children}
      </TooltipTrigger>
      <TooltipContent>
        {props.label}
        {props.keys.map((key) => (
          <Kbd key={key}>{key}</Kbd>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}

/** Which way a key steps through the comments, if it is one of theirs. */
function stepKey(event: globalThis.KeyboardEvent): 1 | -1 | null {
  if (event.altKey) {
    if (event.key === "ArrowDown") return 1;
    if (event.key === "ArrowUp") return -1;
    return null;
  }
  if (event.shiftKey) return null;
  if (event.key === "n") return 1;
  if (event.key === "p") return -1;
  return null;
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.closest("input, textarea, select, [contenteditable]") !== null;
}

function CommentList(props: {
  thread: Thread;
  comments: readonly LineComment[];
  label?: (comment: LineComment) => string;
  flashed?: LineComment["id"] | null;
}) {
  return (
    <div className="flex flex-col gap-2">
      {props.comments.map((comment) => (
        <CommentCard
          key={comment.id}
          thread={props.thread}
          comment={comment}
          label={props.label?.(comment)}
          flashed={props.flashed === comment.id}
        />
      ))}
    </div>
  );
}

function CommentCard(props: { thread: Thread; comment: LineComment; label?: string; flashed?: boolean }) {
  const { thread, comment, label, flashed = false } = props;
  const resolved = comment.resolved_at != null;
  return (
    <div
      data-comment-id={comment.id}
      className={`flex flex-col gap-1 rounded-md border bg-card px-2.5 py-1.5 text-sm ring-primary transition-shadow duration-300 ${resolved ? "opacity-70" : ""} ${flashed ? "ring-2" : "ring-0"}`}
    >
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {resolved ? <Badge variant="secondary">Resolved</Badge> : <Badge variant="outline">Open</Badge>}
        <CommentAuthorMark comment={comment} />
        {comment.sent_at != null && comment.resolved_at == null && (
          <Badge variant="outline" title="Handed to the agent with a message">
            Sent
          </Badge>
        )}
        {label && <span className="truncate font-mono">{label}</span>}
        {label && comment.quote && (
          <code className="min-w-0 truncate" title={comment.quote}>
            {comment.quote.trim()}
          </code>
        )}
        <span className="flex-1" />
        {resolved ? (
          <Button
            variant="ghost"
            size="icon-xs"
            title="Open it again"
            onClick={() => void store().resolveComments(thread.id, [comment.id], true)}
          >
            <RotateCcwIcon />
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="icon-xs"
            title="Mark resolved"
            onClick={() => void store().resolveComments(thread.id, [comment.id])}
          >
            <CheckIcon />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon-xs"
          title="Delete the comment"
          onClick={() => void store().deleteComment(thread.id, comment.id)}
        >
          <Trash2Icon />
        </Button>
      </div>
      <div className={resolved ? "line-through decoration-muted-foreground/50" : undefined}>
        <Markdown text={comment.text} />
      </div>
      {comment.resolution && (
        <div className="border-l-2 pl-2 text-muted-foreground">
          <Markdown text={comment.resolution} />
        </div>
      )}
    </div>
  );
}

/**
 * Who left a comment that is not the user's: a reviewer of a plugin, how much it
 * thinks the remark matters, whether the changes moved on while it looked, and a
 * way to open its thread.
 */
export function CommentAuthorMark({ comment }: { comment: LineComment }) {
  const author = comment.author;
  if (!author) return null;
  const by = `${author.name}, a reviewer of the ${author.plugin} plugin`;
  const open = author.thread ? () => store().select(author.thread!) : undefined;
  return (
    <>
      <Badge
        variant="secondary"
        className={open ? "cursor-pointer" : undefined}
        title={open ? `${by}. Open its thread` : by}
        onClick={open}
      >
        <BotIcon />
        {author.name}
      </Badge>
      {author.severity && <Badge variant="outline">{author.severity}</Badge>}
      {author.stale && (
        <Badge variant="outline" title="The changes moved on while the reviewer looked: the line may be off">
          Outdated
        </Badge>
      )}
    </>
  );
}

function CommentDraft({ onSubmit, onCancel }: { onSubmit: (text: string) => Promise<void>; onCancel: () => void }) {
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const submit = async () => {
    if (!text.trim() || saving) return;
    setSaving(true);
    await onSubmit(text.trim());
    setSaving(false);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void submit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  };
  return (
    <div className="flex flex-col gap-2">
      <Textarea
        autoFocus
        value={text}
        placeholder="What should the agent change here?"
        className="min-h-14 bg-background"
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <div className="flex items-center gap-2">
        <span className="flex-1 text-xs text-muted-foreground">Ctrl+Enter to comment</span>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" disabled={!text.trim() || saving} onClick={() => void submit()}>
          Comment
        </Button>
      </div>
    </div>
  );
}
