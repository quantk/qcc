// Files the user attaches to a message before sending it: picked from disk, pasted
// from the clipboard or dropped on the composer. The daemon keeps its own copy of
// each once the message is sent.

import type { Attachment, NewAttachment } from "../api/protocol.gen";

/** The largest file the daemon takes as an attachment. */
export const MAX_ATTACHMENT_BYTES = 50 * 1024 * 1024;

/** An attachment waiting in the composer. */
export type DraftAttachment = {
  /** Unique within the composer. */
  key: string;
  name: string;
  /** Empty when nothing tells; the daemon looks at the contents then. */
  mime: string;
  /** Unknown for a file picked from disk until the daemon reads it. */
  size: number | null;
  /** A file on disk, read by the daemon, or the contents of a pasted one. */
  source: { path: string } | { data: string };
};

/** Image types agents look at as pictures; others are files to them. */
const PICTURES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

export function isPicture(mime: string): boolean {
  return PICTURES.has(mime);
}

const BY_EXTENSION: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  pdf: "application/pdf",
  md: "text/markdown",
  txt: "text/plain",
  log: "text/plain",
  json: "application/json",
};

/** The media type a file name suggests, or "" if it says nothing we know. */
export function mimeFromName(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot < 0) return "";
  return BY_EXTENSION[name.slice(dot + 1).toLowerCase()] ?? "";
}

/** The last component of a path, for either separator. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

let counter = 0;
function nextKey(): string {
  counter += 1;
  return `attachment-${counter}`;
}

/** Paths of the local files a `text/uri-list` names: what file managers copy. */
export function filePaths(uriList: string): string[] {
  return uriList
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith("file://"))
    .map((line) => decodeURIComponent(new URL(line).pathname));
}

/** A file on disk, by its absolute path. */
export function fromPath(path: string): DraftAttachment {
  const name = fileName(path);
  return { key: nextKey(), name, mime: mimeFromName(name), size: null, source: { path } };
}

/** One of a thread's attachments, sent again (a queued message taken back to edit). */
export function fromAttachment(attachment: Attachment): DraftAttachment {
  const { name, mime, size, path } = attachment;
  return { key: nextKey(), name, mime, size, source: { path } };
}

/** Base64 of bytes, in chunks: a spread of a large array overflows the stack. */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/** A pasted or dropped file, by its contents. Clipboard images come unnamed. */
export async function fromFile(file: File): Promise<DraftAttachment> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const mime = file.type || mimeFromName(file.name);
  // Browsers call every pasted image `image.png`; the daemon names it after its type.
  const name = file.name && file.name !== "image.png" ? file.name : "";
  return { key: nextKey(), name, mime, size: bytes.length, source: { data: toBase64(bytes) } };
}

/** What the daemon is sent for it. */
export function toNewAttachment(draft: DraftAttachment): NewAttachment {
  const attachment: NewAttachment = { ...draft.source };
  if (draft.name) attachment.name = draft.name;
  if (draft.mime) attachment.mime = draft.mime;
  return attachment;
}

/** Why a draft cannot be sent, if it cannot. */
export function problem(draft: DraftAttachment): string | null {
  if (draft.size !== null && draft.size > MAX_ATTACHMENT_BYTES) {
    return `larger than ${MAX_ATTACHMENT_BYTES / 1024 / 1024} MiB`;
  }
  return null;
}

/** A data URL for showing contents the app already has. */
export function dataUrl(mime: string, base64: string): string {
  return `data:${mime};base64,${base64}`;
}

/** `1.2 MB`, `340 KB`, `12 B`. */
export function formatSize(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1000 * 1000) return `${Math.round(bytes / 1000)} KB`;
  return `${(bytes / 1000 / 1000).toFixed(1)} MB`;
}
