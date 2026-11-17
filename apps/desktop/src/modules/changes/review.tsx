// Comments on lines of the changes, as in a code review: left under a line of the
// diff, handed to the agent, and marked resolved by it (`qcc comment resolve`).

import { CheckIcon, RotateCcwIcon, SendIcon, Trash2Icon } from "lucide-react";
import { type KeyboardEvent, useEffect, useMemo, useState } from "react";

import type { LineComment, Thread } from "../../api/protocol.gen";
import { lineAnchor, placeComments } from "../../state/comments";
import type { DiffFile } from "../../state/diff";
import { store, useApp } from "../../state/store";
import { type DiffAddon, Markdown } from "../../ui/kit";
import { plural } from "../../util/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

const NO_COMMENTS: readonly LineComment[] = [];

/** What the agent is asked along with the comments, which reach it in full in a note. */
const REQUEST = "Please address my comments on your changes.";

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
};

/**
 * The comments on a thread's changes, laid out over the diffs read so far (`files`)
 * of the files it changed (`changed`).
 */
export function useReview(thread: Thread, files: readonly DiffFile[], changed?: ReadonlySet<string>): Review {
  const comments = useApp((state) => state.comments[thread.id]) ?? NO_COMMENTS;
  const [draft, setDraft] = useState<{ path: string; index: number } | null>(null);

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
          <CommentList thread={thread} comments={adrift} label={(comment) => `Line ${comment.line}, no longer in the diff`} />
        );
      },
      below(file, index) {
        const here = placement.byFile.get(file.path)?.get(index);
        const drafting = draft?.path === file.path && draft.index === index;
        if (!here && !drafting) return null;
        return (
          <div className="flex flex-col gap-2">
            {here && <CommentList thread={thread} comments={here} />}
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
    [thread, placement, draft],
  );

  const open = useMemo(() => comments.filter((comment) => comment.resolved_at == null), [comments]);
  return { comments, open, elsewhere: placement.elsewhere, addon, ...marks };
}

/** How many comments are open, a way to hand them to the agent, and those left without a file. */
export function ReviewBar({ thread, review, compact = false }: { thread: Thread; review: Review; compact?: boolean }) {
  const [sending, setSending] = useState(false);
  if (review.comments.length === 0) return null;
  const send = async () => {
    setSending(true);
    await store().send(thread.id, REQUEST, "queue", [
      { type: "comments", comment_ids: review.open.map((comment) => comment.id) },
    ]);
    setSending(false);
  };
  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/40 p-2 text-sm">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate">
          {review.open.length > 0 ? `${plural(review.open.length, "open comment")}` : "Every comment is resolved"}
          {review.comments.length > review.open.length && (
            <span className="text-muted-foreground"> · {review.comments.length - review.open.length} resolved</span>
          )}
        </span>
        <Button
          size={compact ? "xs" : "sm"}
          disabled={review.open.length === 0 || sending}
          title="Send the open comments to the agent, each in full, with how to mark it resolved"
          onClick={() => void send()}
        >
          <SendIcon /> Send to agent
        </Button>
      </div>
      {review.elsewhere.length > 0 && (
        <CommentList
          thread={thread}
          comments={review.elsewhere}
          label={(comment) => `${comment.path}:${comment.line}, no longer changed`}
        />
      )}
    </div>
  );
}

function CommentList(props: {
  thread: Thread;
  comments: readonly LineComment[];
  label?: (comment: LineComment) => string;
}) {
  return (
    <div className="flex flex-col gap-2">
      {props.comments.map((comment) => (
        <CommentCard key={comment.id} thread={props.thread} comment={comment} label={props.label?.(comment)} />
      ))}
    </div>
  );
}

function CommentCard({ thread, comment, label }: { thread: Thread; comment: LineComment; label?: string }) {
  const resolved = comment.resolved_at != null;
  return (
    <div className={`flex flex-col gap-1 rounded-md border bg-card px-2.5 py-1.5 text-sm ${resolved ? "opacity-70" : ""}`}>
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {resolved ? <Badge variant="secondary">Resolved</Badge> : <Badge variant="outline">Open</Badge>}
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
