import { memo, useState } from "react";

import type { DiffFile } from "../../state/diff";

/** Files longer than this start folded: a huge diff must not freeze the window. */
const FOLD_ABOVE = 400;

const statusLabel = { added: "new", deleted: "deleted", modified: "" } as const;

export function fileAnchor(path: string): string {
  return `diff-${path}`;
}

const DiffFileView = memo(function DiffFileView({ file, bare }: { file: DiffFile; bare: boolean }) {
  const [open, setOpen] = useState(file.lines.length <= FOLD_ABOVE);
  return (
    <section className="diff-file" id={bare ? undefined : fileAnchor(file.path)}>
      {!bare && (
        <button className="diff-file-head" onClick={() => setOpen(!open)} aria-expanded={open}>
          <span className="chevron">{open ? "▾" : "▸"}</span>
          <span className="diff-path">{file.path}</span>
          {statusLabel[file.status] && <span className="tag">{statusLabel[file.status]}</span>}
          <span className="diff-stat">
            <span className="add">+{file.additions}</span> <span className="del">−{file.deletions}</span>
          </span>
        </button>
      )}
      {(open || bare) && (
        <table className="diff-lines">
          <tbody>
            {file.lines.map((line, index) =>
              line.kind === "hunk" || line.kind === "note" ? (
                <tr key={index} className={line.kind}>
                  <td className="no" colSpan={2} />
                  <td className="code">{line.text}</td>
                </tr>
              ) : (
                <tr key={index} className={line.kind}>
                  <td className="no">{line.oldNo}</td>
                  <td className="no">{line.newNo}</td>
                  <td className="code">
                    <span className="sign">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
                    {line.text}
                  </td>
                </tr>
              ),
            )}
          </tbody>
        </table>
      )}
    </section>
  );
});

/** `bare` drops the per-file headers: for the diff of one file shown under its name. */
export function DiffView({ files, bare = false }: { files: DiffFile[]; bare?: boolean }) {
  return (
    <div className="diff">
      {files.map((file, index) => (
        <DiffFileView key={`${index}:${file.path}`} file={file} bare={bare} />
      ))}
    </div>
  );
}
