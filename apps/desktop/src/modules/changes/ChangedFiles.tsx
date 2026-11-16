import type { ChangeKind, EnvironmentChanges } from "../../api/protocol.gen";
import { DiffStat } from "../../ui/kit";

const kindMark: Record<ChangeKind, string> = { added: "A", modified: "M", deleted: "D", untracked: "N" };
const kindTitle: Record<ChangeKind, string> = {
  added: "Added",
  modified: "Modified",
  deleted: "Deleted",
  untracked: "New, not yet tracked",
};

/** The changed files, each a link to its diff. */
export function ChangedFiles({ changes, open }: { changes: EnvironmentChanges; open: (path: string) => void }) {
  return (
    <ul className="changed-files">
      {changes.files.map((file) => (
        <li key={file.path}>
          <button onClick={() => open(file.path)} title={file.path}>
            <span className={`kind ${file.kind}`} title={kindTitle[file.kind]}>
              {kindMark[file.kind]}
            </span>
            <span className="diff-path">{file.path}</span>
            {file.additions !== undefined && file.additions !== null && (
              <DiffStat additions={file.additions} deletions={file.deletions ?? 0} />
            )}
          </button>
        </li>
      ))}
    </ul>
  );
}
