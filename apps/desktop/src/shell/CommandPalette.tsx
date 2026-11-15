import { useEffect, useMemo, useRef, useState } from "react";

import type { Command } from "../ui/registry";

/** Every contributed command, filtered as you type. */
export function CommandPalette({ commands, close }: { commands: readonly Command[]; close: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);

  useEffect(() => dialog.current?.showModal(), []);

  const found = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return commands.filter((command) => {
      const title = command.title.toLowerCase();
      return (command.enabled?.() ?? true) && words.every((word) => title.includes(word));
    });
  }, [commands, query]);
  const active = Math.min(cursor, Math.max(found.length - 1, 0));

  const run = (command: Command | undefined) => {
    if (!command) return;
    close();
    command.run();
  };

  return (
    <dialog ref={dialog} className="palette" onClose={close}>
      <input
        autoFocus
        value={query}
        placeholder="Run a command…"
        onChange={(event) => {
          setQuery(event.target.value);
          setCursor(0);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") setCursor(Math.min(active + 1, found.length - 1));
          else if (event.key === "ArrowUp") setCursor(Math.max(active - 1, 0));
          else if (event.key === "Enter") run(found[active]);
          else return;
          event.preventDefault();
        }}
      />
      <ul role="listbox">
        {found.map((command, index) => (
          <li key={command.id} role="option" aria-selected={index === active}>
            <button onClick={() => run(command)} onMouseEnter={() => setCursor(index)}>
              <span>{command.title}</span>
              {command.shortcut && <kbd>{command.shortcut}</kbd>}
            </button>
          </li>
        ))}
        {found.length === 0 && <li className="hint">No matching command.</li>}
      </ul>
    </dialog>
  );
}
