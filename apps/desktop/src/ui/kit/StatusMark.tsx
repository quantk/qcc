import type { ItemStatus } from "../../api/protocol.gen";

/** How an item ended, when that is worth a mark. */
export function StatusMark({ status }: { status: ItemStatus }) {
  if (status === "completed") return null;
  if (status === "pending") return <span className="spinner" aria-label="running" />;
  return <span className={`tag ${status}`}>{status}</span>;
}
