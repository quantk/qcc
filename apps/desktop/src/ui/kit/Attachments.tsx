// Files attached to a user message: a thumbnail for a picture, a chip for anything
// else. The composer shows the ones waiting to be sent; the timeline shows those a
// message carried, read from the daemon's copies.

import { FileIcon, ImageOffIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";

import type { Attachment, ThreadId } from "../../api/protocol.gen";
import { formatSize, isPicture } from "../../state/attachments";
import { store } from "../../state/store";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

export type AttachmentTileProps = {
  name: string;
  mime: string;
  size: number | null;
  /** Where the file is, shown on hover. */
  path?: string;
  /** A picture to show: absent for a file shown as a chip, empty while it is read,
   *  null if it cannot be. */
  url?: string | null;
  /** Why it cannot be sent. */
  problem?: string | null;
  onOpen?: () => void;
  onRemove?: () => void;
};

/** One attachment: a thumbnail for a picture that has a URL, a chip otherwise. */
export function AttachmentTile(props: AttachmentTileProps) {
  const { name, mime, size, path, url, problem, onOpen, onRemove } = props;
  const label = name || (isPicture(mime) ? "Pasted image" : "Pasted file");
  const details = [mime, size !== null ? formatSize(size) : null, path].filter(Boolean).join(" · ");
  const title = problem ? `${label}: ${problem}` : details ? `${label}\n${details}` : label;
  const picture = isPicture(mime) && url !== undefined;
  return (
    <div
      data-slot="attachment"
      className={cn(
        "group/attachment relative flex h-16 shrink-0 overflow-hidden rounded-md border bg-muted/40 text-left",
        picture ? "w-16" : "max-w-56 items-center gap-2 px-2.5",
        problem && "border-destructive",
      )}
      title={title}
    >
      {picture ? (
        <button
          type="button"
          className="absolute inset-0 flex cursor-zoom-in items-center justify-center p-0"
          onClick={onOpen}
          disabled={!onOpen || !url}
          aria-label={`Show ${label}`}
        >
          {url ? (
            <img src={url} alt={label} className="absolute inset-0 size-full max-w-none object-cover" draggable={false} />
          ) : url === null ? (
            <ImageOffIcon className="m-auto size-4 text-muted-foreground" aria-hidden />
          ) : null}
        </button>
      ) : (
        <>
          <FileIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <span className="flex min-w-0 flex-col text-xs">
            <span className="truncate font-medium">{label}</span>
            <span className={cn("truncate", problem ? "text-destructive" : "text-muted-foreground")}>
              {problem ?? (size !== null ? formatSize(size) : mime || "file")}
            </span>
          </span>
        </>
      )}
      {onRemove && (
        <Button
          type="button"
          variant="secondary"
          size="icon-xs"
          className="absolute top-0.5 right-0.5 opacity-0 shadow-sm group-hover/attachment:opacity-100 focus-visible:opacity-100"
          onClick={onRemove}
          aria-label={`Remove ${label}`}
          title="Remove"
        >
          <XIcon />
        </Button>
      )}
    </div>
  );
}

/** A picture at full size, over the app. */
export function PictureDialog(props: { name: string; url: string | null; onClose: () => void }) {
  return (
    <Dialog open={props.url !== null} onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent className="max-h-[90vh] w-auto max-w-[90vw] sm:max-w-[90vw]">
        <DialogTitle className="pr-8">{props.name || "Pasted image"}</DialogTitle>
        {props.url && <img src={props.url} alt={props.name} className="max-h-[78vh] max-w-full object-contain" />}
      </DialogContent>
    </Dialog>
  );
}

/** A data URL of a thread's attachment: undefined while it is read, null if it cannot be. */
function useAttachmentUrl(threadId: ThreadId, attachment: Attachment): string | null | undefined {
  const picture = isPicture(attachment.mime);
  const [url, setUrl] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    if (!picture) return;
    let live = true;
    void store()
      .attachmentUrl(threadId, attachment.path)
      .then((read) => live && setUrl(read));
    return () => {
      live = false;
    };
  }, [threadId, attachment.path, picture]);
  return url;
}

function StoredAttachment(props: { threadId: ThreadId; attachment: Attachment; open: (url: string) => void }) {
  const { attachment } = props;
  const url = useAttachmentUrl(props.threadId, attachment);
  return (
    <AttachmentTile
      name={attachment.name}
      mime={attachment.mime}
      size={attachment.size}
      path={attachment.path}
      url={isPicture(attachment.mime) ? (url === undefined ? "" : url) : undefined}
      onOpen={url ? () => props.open(url) : undefined}
    />
  );
}

/** The files a message carried, as the timeline shows them under it. */
export function MessageAttachments(props: {
  threadId: ThreadId;
  attachments: readonly Attachment[];
  className?: string;
}) {
  const [shown, setShown] = useState<{ name: string; url: string } | null>(null);
  if (props.attachments.length === 0) return null;
  return (
    <div className={cn("flex flex-wrap justify-end gap-2", props.className)} aria-label="Attachments">
      {props.attachments.map((attachment) => (
        <StoredAttachment
          key={attachment.path}
          threadId={props.threadId}
          attachment={attachment}
          open={(url) => setShown({ name: attachment.name, url })}
        />
      ))}
      <PictureDialog name={shown?.name ?? ""} url={shown?.url ?? null} onClose={() => setShown(null)} />
    </div>
  );
}
