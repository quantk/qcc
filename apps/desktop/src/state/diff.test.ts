import { describe, expect, it } from "vitest";

import { parseDiff } from "./diff";

const GIT = `diff --git a/README.md b/README.md
index 1234567..89abcde 100644
--- a/README.md
+++ b/README.md
@@ -1,3 +1,4 @@
 # Title
-old line
+new line
+another
 tail
diff --git a/notes.txt b/notes.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/notes.txt
@@ -0,0 +1,2 @@
+one
+two
\\ No newline at end of file
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index e69de29..0000000
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/logo.png b/logo.png
new file mode 100644
Binary files /dev/null and b/logo.png differ
`;

describe("parseDiff", () => {
  it("splits a git diff into files with line numbers", () => {
    const files = parseDiff(GIT);
    expect(files.map(({ path, status, additions, deletions, binary }) => ({ path, status, additions, deletions, binary }))).toEqual([
      { path: "README.md", status: "modified", additions: 2, deletions: 1, binary: false },
      { path: "notes.txt", status: "added", additions: 2, deletions: 0, binary: false },
      { path: "gone.txt", status: "deleted", additions: 0, deletions: 1, binary: false },
      { path: "logo.png", status: "added", additions: 0, deletions: 0, binary: true },
    ]);
    expect(files[0]!.lines).toEqual([
      { kind: "hunk", text: "@@ -1,3 +1,4 @@" },
      { kind: "context", text: "# Title", oldNo: 1, newNo: 1 },
      { kind: "del", text: "old line", oldNo: 2 },
      { kind: "add", text: "new line", newNo: 2 },
      { kind: "add", text: "another", newNo: 3 },
      { kind: "context", text: "tail", oldNo: 3, newNo: 4 },
    ]);
    expect(files[1]!.lines.at(-1)).toEqual({ kind: "note", text: "No newline at end of file" });
  });

  it("reads bare hunks, naming the file from the caller", () => {
    const files = parseDiff("@@ -4,2 +4,2 @@\n ctx\n-a\n+b\n", "src/lib.rs");
    expect(files).toHaveLength(1);
    expect(files[0]).toMatchObject({ path: "src/lib.rs", additions: 1, deletions: 1 });
    expect(files[0]!.lines[2]).toEqual({ kind: "del", text: "a", oldNo: 5 });
  });

  it("reads a diff with file headers but no git line", () => {
    const files = parseDiff("--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-a\n+b\n--- a/y.txt\n+++ b/y.txt\n@@ -1 +1 @@\n-c\n+d\n");
    expect(files.map((file) => file.path)).toEqual(["x.txt", "y.txt"]);
  });

  it("keeps paths with spaces", () => {
    const files = parseDiff("diff --git a/my file.txt b/my file.txt\n--- a/my file.txt\n+++ b/my file.txt\n@@ -1 +1 @@\n-a\n+b\n");
    expect(files[0]!.path).toBe("my file.txt");
  });
});
