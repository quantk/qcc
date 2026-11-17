// The attachments of the message being written: picked with the paperclip, pasted
// from the clipboard or dropped on the window. Like the text, they survive
// switching between threads until sent.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";

import type { Attachment, NewAttachment } from "../../api/protocol.gen";
import {
  type DraftAttachment,
  dataUrl,
  fromAttachment,
  fromFile,
  filePaths,
  fromPath,
  isPicture,
  problem,
  toNewAttachment,
} from "../../state/attachments";
import { AttachmentTile, PictureDialog } from "../../ui/kit";

const drafts = new Map<string, readonly DraftAttachment[]>();
/** A file dialog is open: a second click does not open another. */
let picking = false;
/** Thumbnails of drafts, by key: read once, kept while the app runs. */
const previews = new Map<string, string | null>();

async function preview(draft: DraftAttachment): Promise<string | null> {
  if (!isPicture(draft.mime)) return null;
  if ("data" in draft.source) return dataUrl(draft.mime, draft.source.data);
  try {
    const data = await invoke<string | null>("attachment_preview", { path: draft.source.path });
    return data ? dataUrl(draft.mime, data) : null;
  } catch {
    return null;
  }
}

export type ComposerAttachments = {
  attachments: readonly DraftAttachment[];
  /** Thumbnails by draft key; missing while one is read. */
  previews: ReadonlyMap<string, string | null>;
  /** Files are being dragged over the window. */
  dragging: boolean;
  /** Every attachment can be sent. */
  ready: boolean;
  pick: () => Promise<void>;
  /** Take files from a paste; true if there were any, and the paste is handled. */
  paste: (data: DataTransfer) => boolean;
  remove: (key: string) => void;
  /** A queued message's attachments, back in the composer. */
  restore: (attachments: readonly Attachment[]) => void;
  /** What to send, and forget them once the message is taken. */
  outgoing: () => NewAttachment[];
  clear: () => void;
};

export function useComposerAttachments(threadId: string): ComposerAttachments {
  const [attachments, setAttachments] = useState<readonly DraftAttachment[]>(() => drafts.get(threadId) ?? []);
  const [shownPreviews, setPreviews] = useState<ReadonlyMap<string, string | null>>(() => new Map(previews));
  const [dragging, setDragging] = useState(false);

  // The composer stays mounted across threads: take up the next one's drafts.
  useEffect(() => setAttachments(drafts.get(threadId) ?? []), [threadId]);

  const add = useCallback(
    (added: readonly DraftAttachment[]) => {
      if (added.length === 0) return;
      setAttachments((current) => {
        const next = [...current, ...added];
        drafts.set(threadId, next);
        return next;
      });
      for (const draft of added) {
        void preview(draft).then((url) => {
          previews.set(draft.key, url);
          setPreviews(new Map(previews));
        });
      }
    },
    [threadId],
  );

  // Files dropped anywhere on the window go to the message being written.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let live = true;
    try {
      void getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (payload.type === "enter" || payload.type === "over") setDragging(true);
          else setDragging(false);
          if (payload.type === "drop") add(payload.paths.map(fromPath));
        })
        .then((unlisten) => {
          if (live) stop = unlisten;
          else unlisten();
        })
        .catch(() => {});
    } catch {
      // Outside the app's window there is nothing to drop on.
    }
    return () => {
      live = false;
      stop?.();
    };
  }, [add]);

  const set = (next: readonly DraftAttachment[]) => {
    drafts.set(threadId, next);
    setAttachments(next);
  };

  return {
    attachments,
    previews: shownPreviews,
    dragging,
    ready: attachments.every((draft) => problem(draft) === null),
    pick: async () => {
      if (picking) return;
      picking = true;
      try {
        const picked = await open({ multiple: true, directory: false, title: "Attach files" });
        if (picked !== null) add((Array.isArray(picked) ? picked : [picked]).map(fromPath));
      } finally {
        picking = false;
      }
    },
    paste: (data) => {
      const files = Array.from(data.files);
      if (files.length > 0) {
        void Promise.all(files.map(fromFile)).then(add);
        return true;
      }
      const paths = filePaths(data.getData("text/uri-list"));
      if (paths.length > 0) {
        add(paths.map(fromPath));
        return true;
      }
      return false;
    },
    remove: (key) => set(attachments.filter((draft) => draft.key !== key)),
    restore: (restored) => add(restored.map(fromAttachment)),
    outgoing: () => attachments.map(toNewAttachment),
    clear: () => set([]),
  };
}

/** The row above the text: what goes with the message, each removable. */
export function ComposerAttachmentList({ state }: { state: ComposerAttachments }) {
  const [shown, setShown] = useState<{ name: string; url: string } | null>(null);
  return (
    <div className="flex w-full flex-wrap gap-2" aria-label="Attachments">
      {state.attachments.map((draft) => {
        const url = state.previews.get(draft.key);
        return (
          <AttachmentTile
            key={draft.key}
            name={draft.name}
            mime={draft.mime}
            size={draft.size}
            path={"path" in draft.source ? draft.source.path : undefined}
            url={isPicture(draft.mime) && url ? url : undefined}
            problem={problem(draft)}
            onOpen={url ? () => setShown({ name: draft.name, url }) : undefined}
            onRemove={() => state.remove(draft.key)}
          />
        );
      })}
      <PictureDialog name={shown?.name ?? ""} url={shown?.url ?? null} onClose={() => setShown(null)} />
    </div>
  );
}
