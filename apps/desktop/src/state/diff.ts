// Parses unified diffs — `git diff` output and the bare hunks agents report — into
// files and numbered lines.

export type DiffLine = {
  kind: "add" | "del" | "context" | "hunk" | "note";
  text: string;
  oldNo?: number;
  newNo?: number;
};

export type DiffFile = {
  path: string;
  status: "added" | "deleted" | "modified";
  binary: boolean;
  additions: number;
  deletions: number;
  lines: DiffLine[];
};

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

function stripPrefix(path: string): string {
  const unquoted = path.startsWith('"') && path.endsWith('"') ? path.slice(1, -1) : path;
  return unquoted.replace(/^[ab]\//, "");
}

export function parseDiff(text: string, fallbackPath = ""): DiffFile[] {
  const files: DiffFile[] = [];
  let file = null as DiffFile | null;
  let oldNo = 0;
  let newNo = 0;
  // Lines the current hunk still owes, by its header. A hunk that is paid up has
  // ended, so a following `--- a/file` is a header and not a deleted line.
  let oldLeft = 0;
  let newLeft = 0;
  let inHunk = false;
  const open = (path: string): DiffFile => {
    file = { path, status: "modified", binary: false, additions: 0, deletions: 0, lines: [] };
    files.push(file);
    inHunk = false;
    return file;
  };
  const current = (): DiffFile => file ?? open(fallbackPath);

  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  for (const line of lines) {
    if (inHunk && oldLeft <= 0 && newLeft <= 0 && !line.startsWith("\\")) inHunk = false;
    if (line.startsWith("diff --git ")) {
      // `a/<path> b/<path>`: with equal halves the path is either one.
      const rest = line.slice("diff --git ".length);
      const half = (rest.length - 1) / 2;
      const path = Number.isInteger(half) ? rest.slice(half + 1) : (rest.split(" ").at(-1) ?? rest);
      open(stripPrefix(path));
      continue;
    }
    if (!inHunk) {
      if (line.startsWith("--- ")) {
        const target = file && file.lines.length === 0 ? file : open(fallbackPath);
        const path = line.slice(4).split("\t")[0]!;
        if (path === "/dev/null") target.status = "added";
        else if (!target.path) target.path = stripPrefix(path);
        continue;
      }
      if (line.startsWith("+++ ")) {
        const target = current();
        const path = line.slice(4).split("\t")[0]!;
        if (path === "/dev/null") target.status = "deleted";
        else target.path = stripPrefix(path);
        continue;
      }
      if (line.startsWith("new file mode")) {
        current().status = "added";
        continue;
      }
      if (line.startsWith("deleted file mode")) {
        current().status = "deleted";
        continue;
      }
      if (line.startsWith("Binary files ") || line.startsWith("GIT binary patch")) {
        const target = current();
        target.binary = true;
        target.lines.push({ kind: "note", text: "Binary file" });
        continue;
      }
    }
    const hunk = HUNK.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[3]);
      oldLeft = Number(hunk[2] ?? 1);
      newLeft = Number(hunk[4] ?? 1);
      current().lines.push({ kind: "hunk", text: line });
      inHunk = true;
      continue;
    }
    if (!inHunk) continue; // index lines, modes, similarity headers
    const target = current();
    const body = line.slice(1);
    switch (line[0]) {
      case "+":
        newLeft--;
        target.additions++;
        target.lines.push({ kind: "add", text: body, newNo: newNo++ });
        break;
      case "-":
        oldLeft--;
        target.deletions++;
        target.lines.push({ kind: "del", text: body, oldNo: oldNo++ });
        break;
      case "\\":
        target.lines.push({ kind: "note", text: body.trim() });
        break;
      default:
        oldLeft--;
        newLeft--;
        target.lines.push({ kind: "context", text: body, oldNo: oldNo++, newNo: newNo++ });
    }
  }
  return files;
}
