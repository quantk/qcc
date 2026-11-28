import { describe, expect, it } from "vitest";

import { fuzzyMatch, fuzzyRank } from "./fuzzy";

const rank = (query: string, texts: string[]) => fuzzyRank(texts, query, (t) => t).map(({ item }) => item);

describe("fuzzy", () => {
  it("matches the query's characters in order and nothing else", () => {
    expect(fuzzyMatch("cmp", "src/Composer.tsx")).not.toBeNull();
    expect(fuzzyMatch("cmpz", "src/Composer.tsx")).toBeNull();
    expect(fuzzyMatch("cr", "src")).toBeNull();
    expect(fuzzyMatch("", "anything")).toEqual({ score: 0, positions: [] });
  });

  it("is case-insensitive unless the query has a capital", () => {
    expect(fuzzyMatch("comp", "Composer")).not.toBeNull();
    expect(fuzzyMatch("Comp", "composer")).toBeNull();
  });

  it("points at the best alignment", () => {
    expect(fuzzyMatch("main", "my/app/inner/main.rs")?.positions).toEqual([13, 14, 15, 16]);
    expect(fuzzyMatch("ab", "a/xb/b")?.positions).toEqual([0, 5]);
    expect(fuzzyMatch("inner", "src/deep/inner.rs")?.positions).toEqual([9, 10, 11, 12, 13]);
  });

  it("prefers the name, word starts and runs, then shorter texts", () => {
    expect(rank("lib", ["crates/lib-old/src/main.rs", "src/lib.rs", "docs/library/x.md"])).toEqual([
      "src/lib.rs",
      "docs/library/x.md",
      "crates/lib-old/src/main.rs",
    ]);
    expect(rank("comp", ["src/ui/c/o/m/p.ts", "src/modules/timeline/Composer.tsx"])).toEqual([
      "src/modules/timeline/Composer.tsx",
      "src/ui/c/o/m/p.ts",
    ]);
    expect(rank("rr", ["mirror", "rust-review"])).toEqual(["rust-review", "mirror"]);
    expect(rank("a", ["b/a/c", "a"])).toEqual(["a", "b/a/c"]);
  });

  it("keeps the order for an empty query", () => {
    expect(rank("", ["b", "a"])).toEqual(["b", "a"]);
  });
});
