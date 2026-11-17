import { describe, expect, it } from "vitest";

import {
  MAX_ATTACHMENT_BYTES,
  fileName,
  filePaths,
  formatSize,
  fromAttachment,
  fromFile,
  fromPath,
  isPicture,
  mimeFromName,
  problem,
  toBase64,
  toNewAttachment,
} from "./attachments";

describe("attachments", () => {
  it("names and types picked files from their paths", () => {
    const draft = fromPath("/home/me/shots/Screen Shot.PNG");
    expect(draft.name).toBe("Screen Shot.PNG");
    expect(draft.mime).toBe("image/png");
    expect(draft.size).toBeNull();
    expect(toNewAttachment(draft)).toEqual({
      path: "/home/me/shots/Screen Shot.PNG",
      name: "Screen Shot.PNG",
      mime: "image/png",
    });
    expect(fileName("C:\\work\\a.txt")).toBe("a.txt");
    // Nothing known: the daemon looks at the contents.
    expect(toNewAttachment(fromPath("/tmp/blob"))).toEqual({ path: "/tmp/blob", name: "blob" });
  });

  it("sends pasted images by their bytes and leaves naming them to the daemon", async () => {
    const pasted = new File([new Uint8Array([137, 80, 78, 71])], "image.png", { type: "image/png" });
    const draft = await fromFile(pasted);
    expect(draft.size).toBe(4);
    expect(toNewAttachment(draft)).toEqual({ data: "iVBORw==", mime: "image/png" });

    const named = await fromFile(new File(["hi"], "notes.md"));
    expect(toNewAttachment(named)).toEqual({ data: "aGk=", name: "notes.md", mime: "text/markdown" });
  });

  it("encodes large contents without overflowing the stack", () => {
    const bytes = new Uint8Array(300_000).fill(65);
    expect(atob(toBase64(bytes))).toHaveLength(300_000);
  });

  it("sends a thread's own attachment again by its path", () => {
    const draft = fromAttachment({ name: "a.png", mime: "image/png", path: "/data/attachments/t/1/a.png", size: 10 });
    expect(toNewAttachment(draft)).toEqual({ path: "/data/attachments/t/1/a.png", name: "a.png", mime: "image/png" });
    expect(draft.size).toBe(10);
  });

  it("takes local files a file manager copied", () => {
    expect(filePaths("file:///home/me/My%20Shot.png\r\n# comment\r\nhttps://example.com/x\nfile:///tmp/a.txt")).toEqual([
      "/home/me/My Shot.png",
      "/tmp/a.txt",
    ]);
    expect(filePaths("")).toEqual([]);
  });

  it("tells pictures from other files and refuses what is too large", () => {
    expect(isPicture("image/webp")).toBe(true);
    expect(isPicture("image/svg+xml")).toBe(false);
    expect(mimeFromName("README")).toBe("");
    const big = { ...fromPath("/x.bin"), size: MAX_ATTACHMENT_BYTES + 1 };
    expect(problem(big)).toMatch(/larger than 50 MiB/);
    expect(problem(fromPath("/x.bin"))).toBeNull();
    expect(formatSize(12)).toBe("12 B");
    expect(formatSize(340_000)).toBe("340 KB");
    expect(formatSize(1_234_567)).toBe("1.2 MB");
  });
});
