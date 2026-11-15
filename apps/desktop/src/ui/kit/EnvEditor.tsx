import { PlusIcon, XIcon } from "lucide-react";
import { useState } from "react";

import type { EnvRow } from "../../state/settings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/**
 * Name and value pairs: environment variables, HTTP headers. Values are hidden as
 * they often are keys. A row without a name is kept while it is typed but is not
 * passed on.
 */
export function EnvEditor(props: {
  rows: EnvRow[];
  change: (rows: EnvRow[]) => void;
  /** What the add button says, and what a row is for. */
  addLabel?: string;
  title?: string;
  namePlaceholder?: string;
}) {
  // Rows being typed are kept here: a row without a name yet is not a setting.
  const [extra, setExtra] = useState<EnvRow[]>([]);
  const rows = [...props.rows, ...extra];
  const set = (index: number, row: EnvRow | null) => {
    const next = rows.slice();
    if (row) next[index] = row;
    else next.splice(index, 1);
    setExtra(next.filter((candidate) => !candidate.name.trim()));
    props.change(next);
  };
  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((row, index) => (
        <div key={index} className="flex gap-1.5">
          <Input
            aria-label="Name"
            // A row just added is there to be typed into.
            autoFocus={index >= props.rows.length && !row.name}
            value={row.name}
            placeholder={props.namePlaceholder ?? "NAME"}
            spellCheck={false}
            className="w-2/5 font-mono text-xs"
            onChange={(event) => set(index, { ...row, name: event.target.value })}
          />
          <Input
            aria-label="Value"
            type="password"
            value={row.value}
            placeholder="value"
            spellCheck={false}
            className="font-mono text-xs"
            onChange={(event) => set(index, { ...row, value: event.target.value })}
          />
          <Button type="button" variant="ghost" size="icon" aria-label={`Remove ${row.name || "the row"}`} onClick={() => set(index, null)}>
            <XIcon />
          </Button>
        </div>
      ))}
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="self-start"
        onClick={() => setExtra([...extra, { name: "", value: "" }])}
        title={props.title}
      >
        <PlusIcon /> {props.addLabel ?? "Add variable"}
      </Button>
    </div>
  );
}

