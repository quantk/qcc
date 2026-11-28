import { describe, expect, it } from "vitest";

import { applyCompletion, completionToken, mentionPath } from "./completion";

const triggers = ["/", "@"];

describe("completion", () => {
  it("finds a token at the start or after whitespace, up to the caret", () => {
    expect(completionToken("/pla", 4, triggers)).toEqual({ trigger: "/", start: 0, end: 4, query: "pla" });
    expect(completionToken("see @src/li", 11, triggers)).toEqual({ trigger: "@", start: 4, end: 11, query: "src/li" });
    expect(completionToken("one\n/", 5, triggers)).toEqual({ trigger: "/", start: 4, end: 5, query: "" });
    expect(completionToken("/plan more", 4, triggers)?.query).toBe("pla");
  });

  it("ignores triggers inside words, after the caret's word, and other characters", () => {
    expect(completionToken("a/b", 3, triggers)).toBeNull();
    expect(completionToken("mail@host", 9, triggers)).toBeNull();
    expect(completionToken("/plan ", 6, triggers)).toBeNull();
    expect(completionToken("#tag", 4, triggers)).toBeNull();
    expect(completionToken("", 0, triggers)).toBeNull();
  });

  it("replaces the token and puts the caret after it", () => {
    const token = completionToken("run /pl now", 7, triggers)!;
    expect(applyCompletion("run /pl now", token, "/plan ")).toEqual({ text: "run /plan now", caret: 10 });
    const end = completionToken("@sr", 3, triggers)!;
    expect(applyCompletion("@sr", end, "@src/")).toEqual({ text: "@src/", caret: 5 });
    expect(applyCompletion("@sr", end, "@src/ ")).toEqual({ text: "@src/ ", caret: 6 });
  });

  it("writes a path after @, quoted when it has spaces", () => {
    expect(mentionPath("src/lib.rs", false)).toBe("@src/lib.rs");
    expect(mentionPath("src", true)).toBe("@src/");
    expect(mentionPath("my notes.md", false)).toBe('@"my notes.md"');
  });
});
