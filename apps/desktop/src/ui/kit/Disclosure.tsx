import { type ReactNode, useState } from "react";

import type { ItemStatus } from "../../api/protocol.gen";

/** A one-line row that unfolds. Without children it is just the row. */
export function Disclosure(props: { head: ReactNode; defaultOpen?: boolean; className?: string; children?: ReactNode }) {
  const [open, setOpen] = useState(props.defaultOpen ?? false);
  const foldable = props.children !== undefined && props.children !== null && props.children !== false;
  return (
    <div className={`disclosure ${props.className ?? ""}`}>
      <button className="disclosure-head" disabled={!foldable} onClick={() => setOpen(!open)} aria-expanded={open}>
        <span className="chevron">{foldable ? (open ? "▾" : "▸") : "·"}</span>
        {props.head}
      </button>
      {open && foldable && <div className="disclosure-body">{props.children}</div>}
    </div>
  );
}

/** How an item ended, when that is worth a mark. */
export function StatusMark({ status }: { status: ItemStatus }) {
  if (status === "completed") return null;
  if (status === "pending") return <span className="spinner" aria-label="running" />;
  return <span className={`tag ${status}`}>{status}</span>;
}
